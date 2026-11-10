//! Regolith resources, from the store the current session uses.
//!
//! The sway session keeps its resources in trawl. The i3 session has no
//! trawld; its resources are in the X server's resource database, as loaded
//! by `xrdb`.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use trawlcat::rescat;

pub enum Resources {
    Trawl(tokio::runtime::Runtime),
    X(HashMap<String, String>),
}

impl Resources {
    /// The resource store of the session: trawl under sway, the X resource
    /// database under i3.
    pub fn for_session(sway: bool) -> Self {
        if sway {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Cannot create async runtime");
            Resources::Trawl(runtime)
        } else {
            Resources::X(load_x_resources())
        }
    }

    /// Re-read the store, so that changed values are seen.
    pub fn refresh(&mut self) {
        if let Resources::X(db) = self {
            *db = load_x_resources();
        }
    }

    /// The value of a resource; None when it is absent or empty.
    pub fn get(&mut self, name: &str) -> Option<String> {
        let value = match self {
            Resources::Trawl(runtime) => {
                runtime.block_on(rescat(name, Some(String::new()))).ok()?
            }
            Resources::X(db) => db.get(name)?.clone(),
        };
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    }
}

/// Read the X server's resource database (the root window's
/// RESOURCE_MANAGER property) through `xrdb -query`.
fn load_x_resources() -> HashMap<String, String> {
    let output = Command::new("xrdb")
        .arg("-query")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => parse_x_resources(&output.stdout),
        _ => HashMap::new(),
    }
}

/// Parse `name: value` lines, undoing the escapes of the X resource syntax
/// (`\\`, `\n`, `\NNN` octal and a leading `\ `). Other backslashes are
/// kept, as Xlib does.
fn parse_x_resources(data: &[u8]) -> HashMap<String, String> {
    let mut resources = HashMap::new();
    for line in data.split(|&b| b == b'\n') {
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue;
        };
        let name = String::from_utf8_lossy(&line[..colon]).trim().to_string();
        if name.is_empty() || name.starts_with('!') {
            continue;
        }
        let mut value = &line[colon + 1..];
        while let [b' ' | b'\t', rest @ ..] = value {
            value = rest;
        }
        let mut out = Vec::with_capacity(value.len());
        let mut i = 0;
        while i < value.len() {
            let b = value[i];
            if b == b'\\' && i + 1 < value.len() {
                let next = value[i + 1];
                let octal = |c: u8| (b'0'..=b'7').contains(&c);
                if next == b'\\' {
                    out.push(b'\\');
                    i += 2;
                    continue;
                } else if next == b'n' {
                    out.push(b'\n');
                    i += 2;
                    continue;
                } else if (next == b' ' || next == b'\t') && out.is_empty() {
                    out.push(next);
                    i += 2;
                    continue;
                } else if i + 3 < value.len()
                    && octal(next)
                    && octal(value[i + 2])
                    && octal(value[i + 3])
                {
                    let code =
                        (next - b'0') * 64 + (value[i + 2] - b'0') * 8 + (value[i + 3] - b'0');
                    out.push(code);
                    i += 4;
                    continue;
                }
            }
            out.push(b);
            i += 1;
        }
        resources.insert(name, String::from_utf8_lossy(&out).into_owned());
    }
    resources
}

#[cfg(test)]
mod test {
    use super::parse_x_resources;

    #[test]
    fn test_parse_x_resources() {
        let data = "wm.workspace.03.name:\t3: Chat\n\
                    wm.workspace.icon.firefox:\tWeb \"Browser\"; back\\slash Caf\u{e9}\n\
                    wm.workspace.icon.foot:\t\u{f120}\n\
                    escaped:\t\\ a\\\\b\\nc\\101\n\
                    no colon here\n";
        let resources = parse_x_resources(data.as_bytes());
        assert_eq!("3: Chat", resources["wm.workspace.03.name"]);
        assert_eq!(
            "Web \"Browser\"; back\\slash Caf\u{e9}",
            resources["wm.workspace.icon.firefox"]
        );
        assert_eq!("\u{f120}", resources["wm.workspace.icon.foot"]);
        assert_eq!(" a\\b\ncA", resources["escaped"]);
        assert_eq!(4, resources.len());
    }
}
