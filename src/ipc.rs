//! A small client for the IPC protocol shared by i3 and sway.
//!
//! Replies are kept as raw JSON: the two window managers do not report
//! exactly the same fields, and a typed model of one of them fails to parse
//! some messages of the other.

use serde_json::Value;
use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

const MAGIC: &[u8] = b"i3-ipc";

pub const RUN_COMMAND: u32 = 0;
pub const GET_WORKSPACES: u32 = 1;
pub const SUBSCRIBE: u32 = 2;
pub const GET_TREE: u32 = 4;
pub const GET_VERSION: u32 = 7;

pub const EVENT_WORKSPACE: u32 = 0x8000_0000;
pub const EVENT_SHUTDOWN: u32 = 0x8000_0006;

pub struct Connection {
    stream: UnixStream,
    path: String,
    sway: Option<bool>,
}

/// The socket path i3 publishes on the X root window.
fn i3_socket_path_from_x() -> Option<String> {
    env::var_os("DISPLAY")?;
    let output = Command::new("i3")
        .arg("--get-socketpath")
        .env_remove("I3SOCK")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    Some(path.trim().to_string())
}

impl Connection {
    /// Connect to the running window manager. The socket paths tried are
    /// `I3SOCK`, `SWAYSOCK`, then the path i3 publishes on the X root window
    /// (the variables can be missing or stale in an i3 session).
    pub fn connect() -> io::Result<Self> {
        let mut last = io::Error::new(
            io::ErrorKind::NotFound,
            "no window manager socket: neither I3SOCK nor SWAYSOCK is usable",
        );
        let mut tried = Vec::new();
        for var in ["I3SOCK", "SWAYSOCK"] {
            let Ok(path) = env::var(var) else { continue };
            if path.is_empty() || tried.contains(&path) {
                continue;
            }
            match UnixStream::connect(&path) {
                Ok(stream) => return Ok(Connection::new(stream, path)),
                Err(err) => last = err,
            }
            tried.push(path);
        }
        if let Some(path) = i3_socket_path_from_x() {
            if !path.is_empty() && !tried.contains(&path) {
                match UnixStream::connect(&path) {
                    Ok(stream) => return Ok(Connection::new(stream, path)),
                    Err(err) => last = err,
                }
            }
        }
        Err(last)
    }

    fn new(stream: UnixStream, path: String) -> Self {
        Connection {
            stream,
            path,
            sway: None,
        }
    }

    /// The path of the socket this connection uses.
    pub fn socket_path(&self) -> &str {
        &self.path
    }

    fn send(&mut self, kind: u32, payload: &str) -> io::Result<()> {
        let mut message = Vec::with_capacity(MAGIC.len() + 8 + payload.len());
        message.extend_from_slice(MAGIC);
        message.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
        message.extend_from_slice(&kind.to_ne_bytes());
        message.extend_from_slice(payload.as_bytes());
        self.stream.write_all(&message)
    }

    /// Read one message (a reply or an event): its type and JSON payload.
    pub fn read_message(&mut self) -> io::Result<(u32, Value)> {
        let mut header = [0u8; 14];
        self.stream.read_exact(&mut header)?;
        if &header[..6] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad IPC magic"));
        }
        let len = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
        let kind = u32::from_ne_bytes(header[10..14].try_into().unwrap());
        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload)?;
        let value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
        Ok((kind, value))
    }

    /// Send a request and wait for its reply, skipping any event.
    pub fn request(&mut self, kind: u32, payload: &str) -> io::Result<Value> {
        self.send(kind, payload)?;
        loop {
            let (reply_kind, value) = self.read_message()?;
            if reply_kind & 0x8000_0000 == 0 {
                return Ok(value);
            }
        }
    }

    /// Run a command; true when every part of it succeeded.
    pub fn run_command(&mut self, command: &str) -> io::Result<bool> {
        let reply = self.request(RUN_COMMAND, command)?;
        Ok(match reply.as_array() {
            Some(results) => results
                .iter()
                .all(|r| r.get("success").and_then(Value::as_bool) == Some(true)),
            None => false,
        })
    }

    pub fn get_workspaces(&mut self) -> io::Result<Value> {
        self.request(GET_WORKSPACES, "")
    }

    pub fn get_tree(&mut self) -> io::Result<Value> {
        self.request(GET_TREE, "")
    }

    /// True when the window manager on the other end is sway.
    pub fn is_sway(&mut self) -> io::Result<bool> {
        if let Some(sway) = self.sway {
            return Ok(sway);
        }
        let version = self.request(GET_VERSION, "")?;
        let sway = match version.get("variant").and_then(Value::as_str) {
            Some(variant) => variant == "sway",
            // Older sway releases do not report a variant; i3 reports major 4.
            None => version.get("major").and_then(Value::as_i64) != Some(4),
        };
        self.sway = Some(sway);
        Ok(sway)
    }

    /// Quote a string as one command argument for this window manager.
    pub fn quote(&mut self, text: &str) -> io::Result<String> {
        Ok(quote(text, self.is_sway()?))
    }

    /// Subscribe this connection to the given event types.
    pub fn subscribe(&mut self, events: &[&str]) -> io::Result<()> {
        let payload = serde_json::to_string(events).unwrap();
        let reply = self.request(SUBSCRIBE, &payload)?;
        if reply.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(io::Error::other("subscription refused"))
        }
    }
}

/// Quote a string as one argument of a window manager command.
///
/// i3 removes the backslash from `\"` and `\\` inside a quoted argument.
/// sway keeps every backslash, so a name with a double quote in it is put
/// in single quotes there instead.
pub fn quote(text: &str, sway: bool) -> String {
    if sway {
        if text.contains('"') && !text.contains('\'') {
            return format!("'{text}'");
        }
        if !text.contains('"') {
            return format!("\"{text}\"");
        }
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod test {
    use super::quote;

    #[test]
    fn test_quote_i3_escapes() {
        assert_eq!(r#""a \"b\" c\\d""#, quote(r#"a "b" c\d"#, false));
    }

    #[test]
    fn test_quote_sway() {
        assert_eq!(r#""a; b\c""#, quote(r#"a; b\c"#, true));
        assert_eq!(r#"'a "b"; c\d'"#, quote(r#"a "b"; c\d"#, true));
    }
}
