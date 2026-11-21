//! Sway's i3-compatible IPC.
//!
//! Framing: `"i3-ipc"`, a native-endian u32 payload length, a native-endian
//! u32 message type, then that many bytes of JSON.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::model::{Snapshot, Window};

const MAGIC: &[u8; 6] = b"i3-ipc";
const HEADER_LEN: usize = 14;
const GET_TREE: u32 = 4;
const GET_WORKSPACES: u32 = 5;
const RUN_COMMAND: u32 = 2;
const READ_TIMEOUT: Duration = Duration::from_secs(2);

pub struct Sway {
    socket: PathBuf,
}

impl Sway {
    pub fn connect() -> Result<Self> {
        let socket = std::env::var_os("SWAYSOCK")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .context("SWAYSOCK is not set; start window-jump from inside Sway")?;
        if !socket.exists() {
            bail!(
                "SWAYSOCK points to {}, which does not exist; is Sway running?",
                socket.display()
            );
        }
        Ok(Self { socket })
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let tree: Node = serde_json::from_slice(&self.request(GET_TREE, "")?)
            .context("parsing Sway's window tree")?;
        let workspaces: Vec<Workspace> = serde_json::from_slice(&self.request(GET_WORKSPACES, "")?)
            .context("parsing Sway's workspace list")?;
        Ok(build_snapshot(&tree, &workspaces))
    }

    pub fn activate(&self, id: u64) -> Result<()> {
        let command = focus_command(id);
        let body = self.request(RUN_COMMAND, &command)?;
        let replies: Vec<CommandReply> =
            serde_json::from_slice(&body).context("parsing Sway's command reply")?;
        let failed = replies.iter().find(|reply| !reply.success);
        if let Some(reply) = failed {
            bail!(
                "Sway could not focus window {id}: {}",
                reply.error.as_deref().unwrap_or("unknown error")
            );
        }
        if replies.is_empty() {
            bail!("Sway returned an empty reply to `{command}`");
        }
        Ok(())
    }

    pub fn describe(&self) -> String {
        format!("Sway IPC ({})", self.socket.display())
    }

    fn request(&self, message_type: u32, payload: &str) -> Result<Vec<u8>> {
        let mut stream = UnixStream::connect(&self.socket)
            .with_context(|| format!("connecting to Sway at {}", self.socket.display()))?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(READ_TIMEOUT))?;
        stream.write_all(&frame(message_type, payload.as_bytes()))?;
        stream.flush()?;

        let mut header = [0u8; HEADER_LEN];
        stream
            .read_exact(&mut header)
            .context("reading Sway's IPC reply header")?;
        let (length, _) = parse_header(&header)?;
        let mut body = vec![0u8; usize::try_from(length).context("Sway reply is too large")?];
        stream
            .read_exact(&mut body)
            .context("reading Sway's IPC reply")?;
        Ok(body)
    }
}

/// The i3 `run_command` payload that focuses a window by container id.
pub fn focus_command(id: u64) -> String {
    format!("[con_id={id}] focus")
}

/// A complete i3-ipc frame.
pub fn frame(message_type: u32, payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).expect("i3 IPC payload fits in u32");
    let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&length.to_ne_bytes());
    buf.extend_from_slice(&message_type.to_ne_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Returns `(payload length, message type)` from the first 14 bytes.
pub fn parse_header(bytes: &[u8]) -> Result<(u32, u32)> {
    if bytes.len() < HEADER_LEN {
        bail!("short i3 IPC header ({} bytes)", bytes.len());
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        bail!("not an i3 IPC message (bad magic)");
    }
    let length = u32::from_ne_bytes(bytes[6..10].try_into().expect("four bytes"));
    let message_type = u32::from_ne_bytes(bytes[10..14].try_into().expect("four bytes"));
    Ok((length, message_type))
}

#[derive(Debug, Deserialize, Default)]
struct Node {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    num: Option<i32>,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    sticky: bool,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    pid: Option<i64>,
    #[serde(default)]
    window: Option<u64>,
    #[serde(default)]
    window_properties: Option<WindowProperties>,
    #[serde(default)]
    rect: Option<Rect>,
    #[serde(default)]
    nodes: Vec<Node>,
    #[serde(default)]
    floating_nodes: Vec<Node>,
}

#[derive(Debug, Deserialize, Default)]
struct WindowProperties {
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Rect {
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default)]
    width: i32,
    #[serde(default)]
    height: i32,
}

#[derive(Debug, Deserialize, Default)]
struct Workspace {
    #[serde(default)]
    num: i32,
    #[serde(default)]
    focused: bool,
}

#[derive(Debug, Deserialize)]
struct CommandReply {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    error: Option<String>,
}

impl Node {
    fn is_window(&self) -> bool {
        self.window.is_some()
            || self.app_id.as_deref().is_some_and(|app| !app.is_empty())
            || self.window_properties.is_some()
    }

    fn to_window(&self, workspace: i32, mru: u32) -> Window {
        let class = self
            .window_properties
            .as_ref()
            .and_then(|properties| properties.class.clone())
            .filter(|class| !class.is_empty());
        let wayland_id = self.app_id.clone().filter(|app_id| !app_id.is_empty());
        let app_id = wayland_id
            .clone()
            .or_else(|| class.clone())
            .unwrap_or_default();
        let title = self
            .name
            .clone()
            .or_else(|| {
                self.window_properties
                    .as_ref()
                    .and_then(|properties| properties.title.clone())
            })
            .unwrap_or_default();
        let rect = self.rect.as_ref();
        Window {
            id: self.id,
            title,
            app_id: app_id.clone(),
            app_name: app_id,
            wm_class: class.unwrap_or_default(),
            pid: self
                .pid
                .and_then(|pid| u32::try_from(pid).ok())
                .unwrap_or(0),
            workspace: if self.sticky { -1 } else { workspace },
            minimized: false,
            focused: self.focused,
            mru,
            x: rect.map_or(0, |rect| rect.x),
            y: rect.map_or(0, |rect| rect.y),
            width: rect.map_or(0, |rect| rect.width),
            height: rect.map_or(0, |rect| rect.height),
        }
    }
}

fn walk(node: &Node, inherited_workspace: i32, next_mru: &mut u32, windows: &mut Vec<Window>) {
    let workspace = if node.kind.as_deref() == Some("workspace") {
        node.num.unwrap_or(inherited_workspace)
    } else {
        inherited_workspace
    };
    if node.is_window() {
        windows.push(node.to_window(workspace, *next_mru));
        *next_mru += 1;
    }
    for child in node.nodes.iter().chain(node.floating_nodes.iter()) {
        walk(child, workspace, next_mru, windows);
    }
}

fn build_snapshot(tree: &Node, workspaces: &[Workspace]) -> Snapshot {
    let mut windows = Vec::new();
    let mut mru = 0;
    walk(tree, -1, &mut mru, &mut windows);

    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.focused)
        .map(|workspace| workspace.num)
        .or_else(|| {
            windows
                .iter()
                .find(|window| window.focused)
                .map(|window| window.workspace)
        })
        .filter(|workspace| *workspace >= 0)
        .unwrap_or(0);

    Snapshot {
        active_workspace,
        windows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = r#"{
        "id": 1, "type": "root", "name": "root", "focused": false,
        "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080},
        "nodes": [{
            "id": 2, "type": "output", "name": "eDP-1", "focused": false,
            "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080},
            "nodes": [{
                "id": 3, "type": "workspace", "name": "1", "num": 1, "focused": true,
                "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080},
                "nodes": [
                    {"id": 10, "type": "con", "name": "Editor", "app_id": "code",
                     "pid": 100, "focused": true, "sticky": false,
                     "rect": {"x": 0, "y": 0, "width": 960, "height": 1080},
                     "nodes": [], "floating_nodes": []},
                    {"id": 11, "type": "con", "name": "Terminal", "app_id": null,
                     "pid": 101, "focused": false, "sticky": false, "window": 4194305,
                     "window_properties": {"class": "Alacritty", "instance": "Alacritty",
                                           "title": "Terminal"},
                     "rect": {"x": 960, "y": 0, "width": 960, "height": 1080},
                     "nodes": [], "floating_nodes": []}
                ],
                "floating_nodes": [
                    {"id": 12, "type": "floating_con", "name": "Float", "app_id": "mpv",
                     "pid": 102, "focused": false, "sticky": false,
                     "rect": {"x": 10, "y": 20, "width": 300, "height": 200},
                     "nodes": [], "floating_nodes": []}
                ]
            }, {
                "id": 4, "type": "workspace", "name": "2", "num": 2, "focused": false,
                "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080},
                "nodes": [
                    {"id": 20, "type": "con", "name": "Sticky", "app_id": "notes",
                     "pid": 103, "focused": false, "sticky": true,
                     "rect": {"x": 0, "y": 0, "width": 400, "height": 300},
                     "nodes": [], "floating_nodes": []},
                    {"id": 21, "type": "con", "name": "Split parent", "focused": false,
                     "rect": {"x": 0, "y": 0, "width": 800, "height": 600},
                     "nodes": [
                         {"id": 22, "type": "con", "name": "Deep", "app_id": "deep",
                          "pid": 104, "focused": false, "sticky": false,
                          "rect": {"x": 0, "y": 0, "width": 400, "height": 600},
                          "nodes": [], "floating_nodes": []}
                     ], "floating_nodes": []}
                ],
                "floating_nodes": []
            }]
        }],
        "floating_nodes": []
    }"#;

    const WORKSPACES: &str = r#"[
        {"num": 1, "name": "1", "focused": true, "visible": true,
         "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080}, "output": "eDP-1"},
        {"num": 2, "name": "2", "focused": false, "visible": false,
         "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080}, "output": "eDP-1"}
    ]"#;

    #[test]
    fn frame_round_trips_header_and_payload() {
        let framed = frame(GET_TREE, b"{}");
        assert_eq!(&framed[..6], b"i3-ipc");
        assert_eq!(&framed[6..10], &2u32.to_ne_bytes());
        assert_eq!(&framed[10..14], &4u32.to_ne_bytes());
        assert_eq!(&framed[14..], b"{}");
        assert_eq!(parse_header(&framed).unwrap(), (2, 4));
        assert!(parse_header(b"i3-ipc").is_err());
        assert!(parse_header(b"not-i3-ipc-0000").is_err());
    }

    #[test]
    fn focus_uses_a_container_id_command() {
        assert_eq!(focus_command(42), "[con_id=42] focus");
    }

    #[test]
    fn walks_split_containers_and_workspaces() {
        let tree: Node = serde_json::from_str(TREE).unwrap();
        let workspaces: Vec<Workspace> = serde_json::from_str(WORKSPACES).unwrap();
        let snapshot = build_snapshot(&tree, &workspaces);

        assert_eq!(snapshot.active_workspace, 1);
        assert_eq!(snapshot.windows.len(), 5);
        assert_eq!(
            snapshot
                .windows
                .iter()
                .map(|window| window.id)
                .collect::<Vec<_>>(),
            vec![10, 11, 12, 20, 22]
        );
        assert_eq!(
            snapshot
                .windows
                .iter()
                .map(|window| window.mru)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );

        let focused = snapshot.focused().unwrap();
        assert_eq!(focused.id, 10);
        assert_eq!(focused.title, "Editor");
        assert_eq!(focused.app_id, "code");
        assert_eq!(focused.workspace, 1);
        assert_eq!(
            (focused.x, focused.y, focused.width, focused.height),
            (0, 0, 960, 1080)
        );

        let x11 = snapshot.get(11).unwrap();
        assert_eq!(x11.app_id, "Alacritty");
        assert_eq!(x11.wm_class, "Alacritty");
        assert_eq!(x11.pid, 101);

        assert_eq!(snapshot.get(12).unwrap().workspace, 1);
        assert_eq!(snapshot.get(12).unwrap().y, 20);

        assert_eq!(snapshot.get(20).unwrap().workspace, -1);
        assert_eq!(snapshot.get(22).unwrap().workspace, 2);
    }
}
