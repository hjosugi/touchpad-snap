//! niri's JSON IPC.
//!
//! One request per line, one JSON reply per line. Replies are
//! `{"Ok": ...}` or `{"Err": "..."}`; the successful payload names the
//! response variant, e.g. `{"Ok":{"Windows":[...]}}`.

use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::model::{Snapshot, Window};

const READ_TIMEOUT: Duration = Duration::from_secs(2);
const VERIFY_ATTEMPTS: usize = 5;
const VERIFY_DELAY: Duration = Duration::from_millis(25);

const REQUEST_WINDOWS: &str = "\"Windows\"";
const REQUEST_WORKSPACES: &str = "\"Workspaces\"";
const REQUEST_FOCUSED_WINDOW: &str = "\"FocusedWindow\"";

pub struct Niri {
    socket: PathBuf,
}

impl Niri {
    pub fn connect() -> Result<Self> {
        let socket = std::env::var_os("NIRI_SOCKET")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .context("NIRI_SOCKET is not set; start window-jump from inside niri")?;
        if !socket.exists() {
            bail!(
                "NIRI_SOCKET points to {}, which does not exist; is niri running?",
                socket.display()
            );
        }
        Ok(Self { socket })
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let windows: Vec<WindowInfo> = self.query(REQUEST_WINDOWS, "Windows")?;
        let workspaces: Vec<Workspace> = self.query(REQUEST_WORKSPACES, "Workspaces")?;
        let focused: Option<WindowInfo> = self.query(REQUEST_FOCUSED_WINDOW, "FocusedWindow")?;
        Ok(build_snapshot(&windows, &workspaces, focused.as_ref()))
    }

    pub fn activate(&self, id: u64) -> Result<()> {
        let request = focus_window_request(id);
        match self.send(&request)? {
            Reply::Ok(serde_json::Value::String(message)) if message == "Handled" => {}
            Reply::Ok(other) => bail!("unexpected reply from niri when focusing {id}: {other}"),
            Reply::Err(message) => bail!("niri could not focus window {id}: {message}"),
        }
        self.verify_focus(id)
    }

    pub fn describe(&self) -> String {
        format!("niri IPC ({})", self.socket.display())
    }

    fn query<T: DeserializeOwned>(&self, request: &str, variant: &str) -> Result<T> {
        let reply = self.send(request)?;
        let payload = match reply {
            Reply::Ok(payload) => payload,
            Reply::Err(message) => bail!("niri rejected `{request}`: {message}"),
        };
        let inner = payload
            .get(variant)
            .with_context(|| format!("niri's reply to `{request}` has no `{variant}` field"))?;
        serde_json::from_value(inner.clone())
            .with_context(|| format!("parsing niri's `{variant}` reply"))
    }

    fn send(&self, request: &str) -> Result<Reply<serde_json::Value>> {
        let mut stream = UnixStream::connect(&self.socket)
            .with_context(|| format!("connecting to niri at {}", self.socket.display()))?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(READ_TIMEOUT))?;
        stream.write_all(request.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        stream.shutdown(Shutdown::Write)?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("reading niri's IPC reply")?;
        if line.trim().is_empty() {
            bail!("niri closed the IPC connection without replying");
        }
        serde_json::from_str(line.trim()).context("parsing niri's IPC reply")
    }

    fn verify_focus(&self, id: u64) -> Result<()> {
        let mut focused = None;
        for attempt in 0..VERIFY_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(VERIFY_DELAY);
            }
            let window: Option<WindowInfo> = self.query(REQUEST_FOCUSED_WINDOW, "FocusedWindow")?;
            focused = window.map(|window| window.id);
            if focused == Some(id) {
                return Ok(());
            }
        }
        let focused = focused.map_or_else(|| "none".to_string(), |id| id.to_string());
        bail!("niri did not focus window {id}; the focused window is {focused}")
    }
}

/// The documented shape of an `Action::FocusWindow` request.
pub fn focus_window_request(id: u64) -> String {
    format!("{{\"Action\":{{\"FocusWindow\":{{\"id\":{id}}}}}}}")
}

#[derive(Debug, Deserialize)]
enum Reply<T> {
    #[serde(rename = "Ok")]
    Ok(T),
    #[serde(rename = "Err")]
    Err(String),
}

#[derive(Debug, Deserialize, Default)]
struct WindowInfo {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    pid: Option<i32>,
    #[serde(default)]
    workspace_id: Option<u64>,
    #[serde(default)]
    is_focused: bool,
    #[serde(default)]
    layout: Option<Layout>,
    #[serde(default)]
    focus_timestamp: Option<Timestamp>,
}

#[derive(Debug, Deserialize, Default)]
struct Layout {
    #[serde(default)]
    tile_pos_in_workspace_view: Option<(f64, f64)>,
    #[serde(default)]
    tile_size: (f64, f64),
    #[serde(default)]
    window_size: (i32, i32),
}

#[derive(Debug, Deserialize, Default, Clone, Copy)]
struct Timestamp {
    #[serde(default)]
    secs: u64,
    #[serde(default)]
    nanos: u32,
}

#[derive(Debug, Deserialize, Default)]
struct Workspace {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    idx: u8,
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    is_focused: bool,
}

impl WindowInfo {
    fn to_window(&self, workspace: i32, mru: u32, focused: bool) -> Window {
        let app_id = self.app_id.clone().unwrap_or_default();
        let (x, y, width, height) = self.geometry();
        Window {
            id: self.id,
            title: self.title.clone().unwrap_or_default(),
            app_id: app_id.clone(),
            app_name: app_id.clone(),
            wm_class: app_id,
            pid: self
                .pid
                .and_then(|pid| u32::try_from(pid).ok())
                .unwrap_or(0),
            workspace,
            minimized: false,
            focused,
            mru,
            x,
            y,
            width,
            height,
        }
    }

    fn geometry(&self) -> (i32, i32, i32, i32) {
        let Some(layout) = self.layout.as_ref() else {
            return (0, 0, 0, 0);
        };
        let (x, y) = layout
            .tile_pos_in_workspace_view
            .map_or((0, 0), |(x, y)| (x.round() as i32, y.round() as i32));
        let (tile_width, tile_height) = layout.tile_size;
        let (width, height) = if tile_width > 0.0 && tile_height > 0.0 {
            (tile_width, tile_height)
        } else {
            (
                f64::from(layout.window_size.0),
                f64::from(layout.window_size.1),
            )
        };
        (x, y, width.round() as i32, height.round() as i32)
    }
}

fn build_snapshot(
    windows: &[WindowInfo],
    workspaces: &[Workspace],
    focused: Option<&WindowInfo>,
) -> Snapshot {
    let indices: std::collections::HashMap<u64, i32> = workspaces
        .iter()
        .map(|workspace| (workspace.id, i32::from(workspace.idx)))
        .collect();
    let focused_id = focused.map(|window| window.id);
    let mrus = assign_mru(windows);

    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.is_focused)
        .map(|workspace| i32::from(workspace.idx))
        .or_else(|| {
            focused
                .and_then(|window| window.workspace_id)
                .and_then(|id| indices.get(&id).copied())
        })
        .or_else(|| {
            windows
                .iter()
                .find(|window| window.is_focused)
                .and_then(|window| window.workspace_id)
                .and_then(|id| indices.get(&id).copied())
        })
        .or_else(|| {
            workspaces
                .iter()
                .find(|workspace| workspace.is_active)
                .map(|workspace| i32::from(workspace.idx))
        })
        .unwrap_or(0);

    let windows = windows
        .iter()
        .enumerate()
        .map(|(index, window)| {
            let workspace = window
                .workspace_id
                .and_then(|id| indices.get(&id).copied())
                .unwrap_or(active_workspace);
            let focused = match focused_id {
                Some(id) => window.id == id,
                None => window.is_focused,
            };
            window.to_window(workspace, mrus[index], focused)
        })
        .collect();

    Snapshot {
        active_workspace,
        windows,
    }
}

/// niri exposes a debounced focus timestamp on newer versions; use it for
/// MRU order when present, otherwise keep the list order.
fn assign_mru(windows: &[WindowInfo]) -> Vec<u32> {
    let mut order: Vec<usize> = (0..windows.len()).collect();
    let any_timestamp = windows
        .iter()
        .any(|window| window.focus_timestamp.is_some());
    if any_timestamp {
        order.sort_by(|&left, &right| {
            let left_time = windows[left]
                .focus_timestamp
                .map(|time| (time.secs, time.nanos));
            let right_time = windows[right]
                .focus_timestamp
                .map(|time| (time.secs, time.nanos));
            match (left_time, right_time) {
                (Some(left_time), Some(right_time)) => {
                    right_time.cmp(&left_time).then_with(|| left.cmp(&right))
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left.cmp(&right),
            }
        });
    }
    let mut mru = vec![0u32; windows.len()];
    for (rank, &index) in order.iter().enumerate() {
        mru[index] = u32::try_from(rank).unwrap_or(u32::MAX);
    }
    mru
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: &str = r#"{"Ok":{"Windows":[
        {"id": 1, "title": "term", "app_id": "Alacritty", "pid": 42,
         "workspace_id": 10, "is_focused": false, "is_floating": false,
         "is_urgent": false,
         "layout": {"pos_in_scrolling_layout": [1, 1], "tile_size": [960.0, 1080.0],
                    "window_size": [960, 1080], "tile_pos_in_workspace_view": [0.0, 0.0],
                    "window_offset_in_tile": [0.0, 0.0]},
         "focus_timestamp": {"secs": 100, "nanos": 0}},
        {"id": 2, "title": "editor", "app_id": "code", "pid": 43,
         "workspace_id": 10, "is_focused": true, "is_floating": false,
         "is_urgent": false,
         "layout": {"pos_in_scrolling_layout": [2, 1], "tile_size": [960.0, 1080.0],
                    "window_size": [960, 1080], "tile_pos_in_workspace_view": [960.0, 0.0],
                    "window_offset_in_tile": [0.0, 0.0]},
         "focus_timestamp": {"secs": 200, "nanos": 0}},
        {"id": 3, "title": "other", "app_id": "foo", "pid": 44,
         "workspace_id": 11, "is_focused": false, "is_floating": true,
         "is_urgent": false,
         "layout": {"tile_size": [400.0, 300.0], "window_size": [400, 300],
                    "tile_pos_in_workspace_view": [10.0, 20.0],
                    "window_offset_in_tile": [0.0, 0.0]},
         "focus_timestamp": null}
    ]}}"#;

    const WORKSPACES: &str = r#"{"Ok":{"Workspaces":[
        {"id": 10, "idx": 1, "name": "1", "output": "eDP-1", "is_active": true,
         "is_focused": true, "active_window_id": 2},
        {"id": 11, "idx": 2, "name": "2", "output": "eDP-1", "is_active": false,
         "is_focused": false, "active_window_id": null}
    ]}}"#;

    const FOCUSED: &str = r#"{"Ok":{"FocusedWindow":{
        "id": 2, "title": "editor", "app_id": "code", "pid": 43,
        "workspace_id": 10, "is_focused": true, "is_floating": false,
        "is_urgent": false,
        "layout": {"tile_size": [960.0, 1080.0], "window_size": [960, 1080],
                   "tile_pos_in_workspace_view": [960.0, 0.0],
                   "window_offset_in_tile": [0.0, 0.0]}
    }}}"#;

    fn windows(reply: &str) -> Vec<WindowInfo> {
        let reply: Reply<serde_json::Value> = serde_json::from_str(reply).unwrap();
        let Reply::Ok(payload) = reply else {
            panic!("expected Ok");
        };
        serde_json::from_value(payload.get("Windows").unwrap().clone()).unwrap()
    }

    fn workspaces(reply: &str) -> Vec<Workspace> {
        let reply: Reply<serde_json::Value> = serde_json::from_str(reply).unwrap();
        let Reply::Ok(payload) = reply else {
            panic!("expected Ok");
        };
        serde_json::from_value(payload.get("Workspaces").unwrap().clone()).unwrap()
    }

    fn focused(reply: &str) -> Option<WindowInfo> {
        let reply: Reply<serde_json::Value> = serde_json::from_str(reply).unwrap();
        let Reply::Ok(payload) = reply else {
            panic!("expected Ok");
        };
        serde_json::from_value(payload.get("FocusedWindow").unwrap().clone()).unwrap()
    }

    #[test]
    fn focus_request_uses_the_documented_action_shape() {
        assert_eq!(
            focus_window_request(7),
            r#"{"Action":{"FocusWindow":{"id":7}}}"#
        );
        assert_eq!(REQUEST_WINDOWS, r#""Windows""#);
        assert_eq!(REQUEST_WORKSPACES, r#""Workspaces""#);
        assert_eq!(REQUEST_FOCUSED_WINDOW, r#""FocusedWindow""#);
    }

    #[test]
    fn parses_replies_and_maps_mru() {
        let windows = windows(WINDOWS);
        let workspaces = workspaces(WORKSPACES);
        let focused = focused(FOCUSED);
        let snapshot = build_snapshot(&windows, &workspaces, focused.as_ref());

        assert_eq!(snapshot.active_workspace, 1);
        assert_eq!(snapshot.windows.len(), 3);
        assert_eq!(snapshot.focused().unwrap().id, 2);

        let editor = snapshot.get(2).unwrap();
        assert_eq!(editor.workspace, 1);
        assert_eq!(editor.mru, 0);
        assert_eq!(editor.app_id, "code");
        assert_eq!(
            (editor.x, editor.y, editor.width, editor.height),
            (960, 0, 960, 1080)
        );

        let terminal = snapshot.get(1).unwrap();
        assert_eq!(terminal.workspace, 1);
        assert_eq!(terminal.mru, 1);

        let other = snapshot.get(3).unwrap();
        assert_eq!(other.workspace, 2);
        assert_eq!(other.mru, 2);
        assert_eq!((other.x, other.y), (10, 20));
        assert!(!other.focused);
    }

    #[test]
    fn empty_focused_window_reply_is_valid() {
        assert!(focused(r#"{"Ok":{"FocusedWindow":null}}"#).is_none());
    }

    #[test]
    fn error_replies_surface_the_message() {
        let reply: Reply<serde_json::Value> =
            serde_json::from_str(r#"{"Err":"unsupported request"}"#).unwrap();
        match reply {
            Reply::Err(message) => assert_eq!(message, "unsupported request"),
            Reply::Ok(_) => panic!("expected Err"),
        }
    }

    #[test]
    fn without_timestamps_the_list_order_is_mru() {
        let mut windows = windows(WINDOWS);
        for window in &mut windows {
            window.focus_timestamp = None;
        }
        let mrus = assign_mru(&windows);
        assert_eq!(mrus, vec![0, 1, 2]);
    }
}
