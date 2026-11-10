//! `childe --workspace-names`: keep every workspace's name showing the apps
//! on it.
//!
//! The daemon never trusts the order of rename events. Any event only
//! triggers a fresh look at the layout tree, taken on the same connection
//! the daemon renames through: a rename it issued has been applied by the
//! time its reply arrives, so a name in the tree that differs from the last
//! one the daemon gave a workspace was given by someone else.

use crate::ipc::{Connection, EVENT_SHUTDOWN, EVENT_WORKSPACE};
use crate::resources::Resources;
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::os::unix::net::UnixListener;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Some changes (an app_id or WM_CLASS set on a mapped window) produce no
/// event in either window manager, so the tree is also checked regularly.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How long to wait for i3 to come back after a `restart`.
const RESTART_WAIT: Duration = Duration::from_secs(10);
/// How long to wait for the window manager after an unannounced disconnect.
const LOST_WAIT: Duration = Duration::from_millis(1500);
const RECONNECT_INTERVAL: Duration = Duration::from_millis(50);

const SCRATCHPAD: &str = "__i3_scratch";

enum Message {
    Event {
        generation: u64,
        kind: u32,
        payload: Value,
    },
    Closed {
        generation: u64,
    },
    Signal,
}

struct Workspace {
    id: u64,
    name: String,
    identifiers: Vec<String>,
}

struct Tracked {
    /// The name to give back on exit, and to decorate.
    base: String,
    /// The name the workspace had when the daemon last looked at or renamed it.
    applied: String,
}

struct Daemon {
    conn: Connection,
    resources: Resources,
    labels: HashMap<String, String>,
    tracked: HashMap<u64, Tracked>,
    /// After an i3 restart container ids change: the state from before the
    /// restart, keyed by the name the daemon had applied.
    carried: HashMap<String, Tracked>,
    generation: u64,
    tx: Sender<Message>,
}

pub fn run() -> i32 {
    let (tx, rx) = mpsc::channel();

    if let Err(err) = watch_signals(tx.clone()) {
        eprintln!("childe: cannot install signal handlers: {err}");
        return 1;
    }

    let conn = match Connection::connect() {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("childe: cannot connect to the window manager: {err}");
            return 1;
        }
    };
    // Held until the process ends.
    let _instance = match claim_instance(conn.socket_path()) {
        Ok(Some(listener)) => Some(listener),
        Ok(None) => {
            eprintln!("childe: workspace names are already kept by another childe for this window manager");
            return 0;
        }
        Err(err) => {
            eprintln!("childe: cannot check for another running childe: {err}");
            None
        }
    };
    let mut daemon = Daemon {
        conn,
        resources: Resources::X(HashMap::new()),
        labels: HashMap::new(),
        tracked: HashMap::new(),
        carried: HashMap::new(),
        generation: 0,
        tx,
    };
    if let Err(err) = daemon.start_session() {
        eprintln!("childe: cannot talk to the window manager: {err}");
        return 1;
    }
    daemon.main_loop(&rx)
}

/// Make sure this is the only daemon for the window manager behind
/// `wm_socket`. A second one would take the names the first one gives for
/// base names and decorate them again, and the two would keep lengthening
/// every name; that happens as soon as the daemon is started with
/// `exec_always`, which runs again on every reload.
///
/// The claim is an abstract Unix socket named after the window manager's
/// IPC socket, which stays the same across an i3 `restart`. The kernel
/// releases it when the process ends, however it ends. `Ok(None)` means
/// another daemon holds it.
fn claim_instance(wm_socket: &str) -> io::Result<Option<UnixListener>> {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::SocketAddr;

    let path = std::fs::canonicalize(wm_socket)
        .map(|path| path.into_os_string())
        .unwrap_or_else(|_| wm_socket.into());
    let name = format!("childe-workspace-names-{:016x}", fnv1a(path.as_bytes()));
    let addr = SocketAddr::from_abstract_name(name.as_bytes())?;
    match UnixListener::bind_addr(&addr) {
        Ok(listener) => Ok(Some(listener)),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => Ok(None),
        Err(err) => Err(err),
    }
}

/// 64-bit FNV-1a, to keep the claim's name short whatever the path length.
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Report SIGTERM and SIGINT to the main loop, from a thread of its own.
fn watch_signals(tx: Sender<Message>) -> std::io::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // Install the handlers now, so a signal sent right after start-up is
    // not missed.
    let (mut term, mut int) = {
        let _guard = runtime.enter();
        (
            signal(SignalKind::terminate())?,
            signal(SignalKind::interrupt())?,
        )
    };
    thread::spawn(move || {
        runtime.block_on(async {
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
            }
        });
        let _ = tx.send(Message::Signal);
    });
    Ok(())
}

impl Daemon {
    /// Detect the window manager, pick the resource store and start
    /// listening for events on a fresh connection.
    fn start_session(&mut self) -> std::io::Result<()> {
        let sway = self.conn.is_sway()?;
        self.resources = Resources::for_session(sway);
        self.labels.clear();

        let mut events = Connection::connect()?;
        events.subscribe(&["workspace", "window", "shutdown"])?;
        self.generation += 1;
        let generation = self.generation;
        let tx = self.tx.clone();
        thread::spawn(move || loop {
            match events.read_message() {
                Ok((kind, payload)) => {
                    let message = Message::Event {
                        generation,
                        kind,
                        payload,
                    };
                    if tx.send(message).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(Message::Closed { generation });
                    break;
                }
            }
        });
        Ok(())
    }

    fn main_loop(&mut self, rx: &Receiver<Message>) -> i32 {
        let mut restarting = false;
        let mut dirty = true;
        loop {
            if dirty {
                dirty = false;
                if self.sync().is_err() {
                    if !self.reconnect(rx, restarting) {
                        return 0;
                    }
                    restarting = false;
                    dirty = true;
                    continue;
                }
            }

            let first = match rx.recv_timeout(POLL_INTERVAL) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    dirty = true;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return 0,
            };
            // Take everything already queued, so a burst of events costs
            // one look at the tree.
            let mut closed = false;
            for message in std::iter::once(first).chain(rx.try_iter()) {
                match message {
                    Message::Signal => {
                        self.restore();
                        return 0;
                    }
                    Message::Event { generation, .. } | Message::Closed { generation }
                        if generation != self.generation => {}
                    Message::Event { kind, payload, .. } => {
                        dirty = true;
                        let change = payload.get("change").and_then(Value::as_str);
                        if kind == EVENT_SHUTDOWN {
                            if change == Some("restart") {
                                restarting = true;
                            } else {
                                // The window manager is exiting.
                                return 0;
                            }
                        } else if kind == EVENT_WORKSPACE && change == Some("reload") {
                            self.resources.refresh();
                            self.labels.clear();
                        }
                    }
                    Message::Closed { .. } => closed = true,
                }
            }
            if closed {
                if !self.reconnect(rx, restarting) {
                    return 0;
                }
                restarting = false;
                dirty = true;
            }
        }
    }

    /// The connection to the window manager is gone. Wait for it to come
    /// back (i3 `restart`), or report that it exited.
    fn reconnect(&mut self, rx: &Receiver<Message>, mut restarting: bool) -> bool {
        let old_generation = self.generation;
        // Ignore anything the old connections still send.
        self.generation += 1;
        let started = Instant::now();
        loop {
            // The shutdown event can still be queued when a request fails.
            for message in rx.try_iter() {
                match message {
                    Message::Signal => return false,
                    Message::Event {
                        generation,
                        kind,
                        payload,
                    } if generation == old_generation && kind == EVENT_SHUTDOWN => {
                        if payload.get("change").and_then(Value::as_str) == Some("restart") {
                            restarting = true;
                        } else {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            if let Ok(conn) = Connection::connect() {
                self.conn = conn;
                if self.start_session().is_ok() {
                    let tracked: Vec<Tracked> = self.tracked.drain().map(|(_, s)| s).collect();
                    self.carried
                        .extend(tracked.into_iter().map(|s| (s.applied.clone(), s)));
                    return true;
                }
            }
            let wait = if restarting { RESTART_WAIT } else { LOST_WAIT };
            if started.elapsed() >= wait {
                return false;
            }
            thread::sleep(RECONNECT_INTERVAL);
        }
    }

    fn label(&mut self, identifier: &str) -> String {
        let key = identifier.to_lowercase();
        if let Some(label) = self.labels.get(&key) {
            return label.clone();
        }
        let label = self
            .resources
            .get(&format!("wm.workspace.icon.{key}"))
            .unwrap_or_else(|| key.clone());
        self.labels.insert(key, label.clone());
        label
    }

    /// Look at the tree and rename every workspace whose name is not the
    /// one it should show.
    fn sync(&mut self) -> std::io::Result<()> {
        let tree = self.conn.get_tree()?;
        let mut workspaces = Vec::new();
        collect_workspaces(&tree, &mut workspaces);

        let mut present = Vec::with_capacity(workspaces.len());
        for ws in workspaces {
            present.push(ws.id);
            let mut labels: Vec<String> = Vec::new();
            for identifier in &ws.identifiers {
                let label = self.label(identifier);
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }

            let carried = self.carried.remove(&ws.name);
            let state = self.tracked.entry(ws.id).or_insert_with(|| {
                carried.unwrap_or_else(|| Tracked {
                    base: ws.name.clone(),
                    applied: ws.name.clone(),
                })
            });
            if ws.name != state.applied {
                // Renamed by someone else: that is the new base name.
                state.base = ws.name.clone();
                state.applied = ws.name.clone();
            }

            let wanted = display_name(&state.base, &labels);
            if wanted != ws.name {
                let command = format!(
                    "rename workspace {} to {}",
                    self.conn.quote(&ws.name)?,
                    self.conn.quote(&wanted)?
                );
                // A failure (the workspace was renamed or removed meanwhile)
                // is picked up by the next look at the tree.
                if self.conn.run_command(&command)? {
                    state.applied = wanted;
                }
            }
        }
        self.tracked.retain(|id, _| present.contains(id));
        self.carried.clear();
        Ok(())
    }

    /// Give every workspace the daemon decorated its base name back.
    fn restore(&mut self) {
        let Ok(tree) = self.conn.get_tree() else {
            return;
        };
        let mut workspaces = Vec::new();
        collect_workspaces(&tree, &mut workspaces);
        for ws in workspaces {
            let Some(state) = self.tracked.get(&ws.id) else {
                continue;
            };
            if ws.name == state.applied && ws.name != state.base {
                let (Ok(from), Ok(to)) = (self.conn.quote(&ws.name), self.conn.quote(&state.base))
                else {
                    return;
                };
                let command = format!("rename workspace {from} to {to}");
                if self.conn.run_command(&command).is_err() {
                    return;
                }
            }
        }
    }
}

/// The name a workspace shows: its base name, followed by the labels.
pub fn display_name(base: &str, labels: &[String]) -> String {
    if labels.is_empty() {
        return base.to_string();
    }
    let separator = if !base.is_empty() && base.bytes().all(|b| b.is_ascii_digit()) {
        ": "
    } else {
        " "
    };
    format!("{base}{separator}{}", labels.join(" "))
}

fn collect_workspaces(node: &Value, out: &mut Vec<Workspace>) {
    if node.get("type").and_then(Value::as_str) == Some("workspace") {
        let name = node.get("name").and_then(Value::as_str).unwrap_or_default();
        let Some(id) = node.get("id").and_then(Value::as_u64) else {
            return;
        };
        if name == SCRATCHPAD || name.is_empty() {
            return;
        }
        let mut identifiers = Vec::new();
        for child in children(node, "nodes") {
            collect_identifiers(child, &mut identifiers);
        }
        for child in children(node, "floating_nodes") {
            collect_identifiers(child, &mut identifiers);
        }
        out.push(Workspace {
            id,
            name: name.to_string(),
            identifiers,
        });
        return;
    }
    for child in children(node, "nodes").chain(children(node, "floating_nodes")) {
        collect_workspaces(child, out);
    }
}

fn children<'a>(node: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    node.get(key)
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter())
        .into_iter()
        .flatten()
}

/// Window identifiers in layout order: depth-first through the tiled
/// containers, then the floating ones.
fn collect_identifiers(node: &Value, out: &mut Vec<String>) {
    if let Some(identifier) = window_identifier(node) {
        out.push(identifier);
    }
    for child in children(node, "nodes").chain(children(node, "floating_nodes")) {
        collect_identifiers(child, out);
    }
}

/// A native Wayland window is identified by its app_id, an X11 window by
/// the class part of WM_CLASS.
fn window_identifier(node: &Value) -> Option<String> {
    if let Some(app_id) = node.get("app_id").and_then(Value::as_str) {
        if !app_id.is_empty() {
            return Some(app_id.to_string());
        }
    }
    let class = node
        .get("window_properties")
        .and_then(|props| props.get("class"))
        .and_then(Value::as_str)?;
    if class.is_empty() {
        None
    } else {
        Some(class.to_string())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_display_name_numeric_base() {
        assert_eq!("2: web foot", display_name("2", &labels(&["web", "foot"])));
        assert_eq!("07: alpha", display_name("07", &labels(&["alpha"])));
    }

    #[test]
    fn test_display_name_named_base() {
        assert_eq!("3: Chat foot", display_name("3: Chat", &labels(&["foot"])));
        assert_eq!("music foot", display_name("music", &labels(&["foot"])));
    }

    #[test]
    fn test_display_name_without_labels() {
        assert_eq!("2", display_name("2", &[]));
        assert_eq!("music", display_name("music", &[]));
    }

    #[test]
    fn test_identifiers_tiled_then_floating() {
        let tree = json!({
            "type": "root", "id": 1, "nodes": [{
                "type": "output", "id": 2, "nodes": [
                    {"type": "workspace", "id": 3, "name": "1",
                     "nodes": [
                        {"type": "con", "id": 4, "app_id": "alpha", "nodes": []},
                        {"type": "con", "id": 5, "nodes": [
                            {"type": "con", "id": 6, "app_id": null,
                             "window_properties": {"class": "Bravo", "instance": "x"},
                             "nodes": []}
                        ]}
                     ],
                     "floating_nodes": [
                        {"type": "floating_con", "id": 7, "nodes": [
                            {"type": "con", "id": 8,
                             "window_properties": {"class": "Charlie"}, "nodes": []}
                        ]}
                     ]},
                    {"type": "workspace", "id": 9, "name": "__i3_scratch",
                     "nodes": [], "floating_nodes": [
                        {"type": "floating_con", "id": 10, "app_id": "hidden"}
                     ]}
                ]
            }]
        });
        let mut workspaces = Vec::new();
        collect_workspaces(&tree, &mut workspaces);
        assert_eq!(1, workspaces.len());
        assert_eq!(3, workspaces[0].id);
        assert_eq!(vec!["alpha", "Bravo", "Charlie"], workspaces[0].identifiers);
    }

    #[test]
    fn test_one_daemon_per_window_manager() {
        let socket = format!("/nonexistent/childe-test-{}.sock", std::process::id());
        let other = format!("{socket}.other");
        let first = claim_instance(&socket).unwrap();
        assert!(first.is_some());
        assert!(claim_instance(&socket).unwrap().is_none());
        // another window manager is another claim
        assert!(claim_instance(&other).unwrap().is_some());
        drop(first);
        assert!(claim_instance(&socket).unwrap().is_some());
    }

    #[test]
    fn test_window_without_identifier_is_ignored() {
        assert_eq!(None, window_identifier(&json!({"app_id": null})));
        assert_eq!(None, window_identifier(&json!({"app_id": ""})));
        assert_eq!(
            None,
            window_identifier(&json!({"window_properties": {"class": "", "instance": "decoy"}}))
        );
    }
}
