//! Hyprland's text IPC (`hyprctl`'s sockets).
//!
//! `.socket.sock` answers `j/clients`, `j/workspaces`, `j/activeworkspace`,
//! `j/activewindow` and text commands such as
//! `dispatch focuswindow address:0x...`. `.socket2.sock` is the event socket;
//! it is only tried as a fallback on setups where the command socket is
//! missing.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::model::{Snapshot, Window};

const READ_TIMEOUT: Duration = Duration::from_secs(2);
const VERIFY_ATTEMPTS: usize = 5;
const VERIFY_DELAY: Duration = Duration::from_millis(25);

pub struct Hyprland {
    socket: PathBuf,
    command_socket: PathBuf,
    signature: String,
}

impl Hyprland {
    pub fn connect() -> Result<Self> {
        let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
            .ok()
            .filter(|signature| !signature.trim().is_empty())
            .context(
                "HYPRLAND_INSTANCE_SIGNATURE is not set; start touchpad-snap from inside Hyprland",
            )?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        let mut candidates = Vec::new();
        if let Some(runtime) = runtime {
            candidates.push(runtime.join("hypr").join(&signature));
        }
        // Hyprland older than the XDG_RUNTIME_DIR layout uses /tmp/hypr.
        candidates.push(PathBuf::from("/tmp/hypr").join(&signature));
        let directory = candidates
            .iter()
            .find(|directory| directory.join(".socket.sock").exists());
        let Some(directory) = directory.cloned() else {
            let searched = candidates
                .iter()
                .map(|directory| directory.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "Hyprland's IPC socket was not found (looked in {searched}); is Hyprland running?"
            );
        };
        let socket = directory.join(".socket.sock");
        let command_socket = directory.join(".socket2.sock");
        Ok(Self {
            socket,
            command_socket,
            signature,
        })
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let clients: Vec<Client> = self.json("j/clients")?;
        let workspaces: Vec<Workspace> = self.json("j/workspaces")?;
        let active_workspace: Workspace = self.json("j/activeworkspace")?;
        let active_window: ActiveWindow = self.json("j/activewindow").unwrap_or_default();
        Ok(build_snapshot(
            &clients,
            &workspaces,
            &active_workspace,
            &active_window,
        ))
    }

    pub fn activate(&self, id: u64) -> Result<()> {
        let address = format_address(id);
        let command = format!("dispatch focuswindow address:{address}");
        let reply = self.command(&command)?;
        let reply = reply.trim();
        if reply.to_ascii_lowercase().starts_with("error")
            || (!reply.is_empty() && !reply.eq_ignore_ascii_case("ok"))
        {
            bail!("Hyprland could not focus window {address}: {reply}");
        }
        self.verify_focus(id, &address)
    }

    pub fn describe(&self) -> String {
        format!(
            "Hyprland IPC ({} via {})",
            self.signature,
            self.socket.display()
        )
    }

    fn json<T: DeserializeOwned>(&self, command: &str) -> Result<T> {
        let text = self.request_text(&self.socket, command)?;
        serde_json::from_str(&text).with_context(|| format!("parsing Hyprland's `{command}` reply"))
    }

    fn command(&self, command: &str) -> Result<String> {
        match self.request_text(&self.socket, command) {
            Ok(reply) => Ok(reply),
            Err(first) => self
                .request_text(&self.command_socket, command)
                .with_context(|| format!("{first:#}")),
        }
    }

    fn request_text(&self, path: &Path, command: &str) -> Result<String> {
        let mut stream = UnixStream::connect(path)
            .with_context(|| format!("connecting to Hyprland at {}", path.display()))?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(READ_TIMEOUT))?;
        stream.write_all(command.as_bytes())?;
        stream.flush()?;
        stream.shutdown(Shutdown::Write)?;
        let mut reply = String::new();
        stream
            .read_to_string(&mut reply)
            .context("reading Hyprland's IPC reply")?;
        Ok(reply)
    }

    fn verify_focus(&self, id: u64, address: &str) -> Result<()> {
        let mut active = String::new();
        for attempt in 0..VERIFY_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(VERIFY_DELAY);
            }
            let window: ActiveWindow = self.json("j/activewindow").unwrap_or_default();
            active = window.address;
            if parse_address(&active) == Some(id) {
                return Ok(());
            }
        }
        let active = if active.is_empty() {
            "none".to_string()
        } else {
            active
        };
        bail!("Hyprland did not focus window {address}; the active window is still {active}")
    }
}

/// Hyprland prints addresses as `0x`-prefixed hex.
pub fn parse_address(address: &str) -> Option<u64> {
    let address = address.trim();
    let digits = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"))
        .unwrap_or(address);
    u64::from_str_radix(digits, 16).ok()
}

pub fn format_address(id: u64) -> String {
    format!("0x{id:x}")
}

#[derive(Debug, Deserialize, Default)]
struct Client {
    #[serde(default)]
    address: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    class: String,
    #[serde(default)]
    pid: u32,
    #[serde(default)]
    workspace: WorkspaceRef,
    #[serde(default)]
    at: [i32; 2],
    #[serde(default)]
    size: [i32; 2],
    #[serde(default, rename = "focusHistoryID")]
    focus_history_id: Option<i32>,
    #[serde(default)]
    mapped: Option<bool>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    pinned: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct WorkspaceRef {
    #[serde(default)]
    id: i32,
}

#[derive(Debug, Deserialize, Default)]
struct Workspace {
    #[serde(default)]
    id: i32,
    #[serde(default)]
    windows: u32,
}

#[derive(Debug, Deserialize, Default)]
struct ActiveWindow {
    #[serde(default)]
    address: String,
}

fn build_snapshot(
    clients: &[Client],
    workspaces: &[Workspace],
    active_workspace: &Workspace,
    active_window: &ActiveWindow,
) -> Snapshot {
    let active_id = if workspaces.is_empty()
        || workspaces
            .iter()
            .any(|workspace| workspace.id == active_workspace.id)
    {
        active_workspace.id
    } else {
        workspaces
            .iter()
            .find(|workspace| workspace.windows > 0)
            .map_or(active_workspace.id, |workspace| workspace.id)
    };
    let active_address = parse_address(&active_window.address);
    let windows = clients
        .iter()
        .enumerate()
        .filter_map(|(index, client)| {
            let id = parse_address(&client.address)?;
            let focused =
                active_address.map_or(client.focus_history_id == Some(0), |active| active == id);
            let minimized = client.hidden.unwrap_or(false) || !client.mapped.unwrap_or(true);
            let workspace = if client.pinned.unwrap_or(false) {
                -1
            } else {
                client.workspace.id
            };
            let mru = client
                .focus_history_id
                .filter(|history| *history >= 0)
                .and_then(|history| u32::try_from(history).ok())
                .unwrap_or_else(|| u32::try_from(index).unwrap_or(u32::MAX));
            Some(Window {
                id,
                title: client.title.clone(),
                app_id: client.class.clone(),
                app_name: client.class.clone(),
                wm_class: client.class.clone(),
                pid: client.pid,
                workspace,
                minimized,
                focused,
                mru,
                x: client.at[0],
                y: client.at[1],
                width: client.size[0],
                height: client.size[1],
            })
        })
        .collect();

    Snapshot {
        active_workspace: active_id,
        windows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: &str = r#"[
        {"address": "0x55d0a1", "mapped": true, "hidden": false, "at": [10, 20],
         "size": [800, 600], "workspace": {"id": 1, "name": "1"},
         "class": "kitty", "title": "zsh", "pid": 123, "pinned": false,
         "focusHistoryID": 1},
        {"address": "0X55D0A2", "mapped": true, "hidden": false, "at": [0, 0],
         "size": [1920, 1080], "workspace": {"id": 2, "name": "2"},
         "class": "firefox", "title": "Mozilla", "pid": 124, "pinned": false,
         "focusHistoryID": 0},
        {"address": "0x55d0a3", "mapped": false, "hidden": true, "at": [5, 5],
         "size": [100, 100], "workspace": {"id": 1, "name": "1"},
         "class": "mpv", "title": "video", "pid": 125, "pinned": true,
         "focusHistoryID": 2}
    ]"#;

    const WORKSPACES: &str = r#"[
        {"id": 1, "name": "1", "monitor": "DP-1", "monitorID": 0, "windows": 2,
         "hasfullscreen": false, "lastwindow": "0x55d0a1", "lastwindowtitle": "zsh",
         "ispersistent": false},
        {"id": 2, "name": "2", "monitor": "DP-1", "monitorID": 0, "windows": 1,
         "hasfullscreen": false, "lastwindow": "0x55d0a2", "lastwindowtitle": "Mozilla",
         "ispersistent": false}
    ]"#;

    #[test]
    fn addresses_are_hex_with_optional_prefix() {
        assert_eq!(parse_address("0x55d0a2"), Some(0x55d0a2));
        assert_eq!(parse_address("0X55D0A2"), Some(0x55d0a2));
        assert_eq!(parse_address("55d0a2"), Some(0x55d0a2));
        assert_eq!(parse_address("not-an-address"), None);
        assert_eq!(format_address(0x55d0a2), "0x55d0a2");
    }

    #[test]
    fn maps_clients_with_focus_history() {
        let clients: Vec<Client> = serde_json::from_str(CLIENTS).unwrap();
        let workspaces: Vec<Workspace> = serde_json::from_str(WORKSPACES).unwrap();
        let active_workspace: Workspace =
            serde_json::from_str(r#"{"id": 2, "name": "2", "windows": 1}"#).unwrap();
        let active_window: ActiveWindow =
            serde_json::from_str(r#"{"address": "0x55d0a2"}"#).unwrap();

        let snapshot = build_snapshot(&clients, &workspaces, &active_workspace, &active_window);
        assert_eq!(snapshot.active_workspace, 2);
        assert_eq!(snapshot.windows.len(), 3);

        let firefox = snapshot.focused().unwrap();
        assert_eq!(firefox.id, 0x55d0a2);
        assert_eq!(firefox.title, "Mozilla");
        assert_eq!(firefox.app_id, "firefox");
        assert_eq!(firefox.workspace, 2);
        assert_eq!(firefox.mru, 0);
        assert_eq!(
            (firefox.x, firefox.y, firefox.width, firefox.height),
            (0, 0, 1920, 1080)
        );

        let kitty = snapshot.get(0x55d0a1).unwrap();
        assert_eq!(kitty.mru, 1);
        assert_eq!(kitty.workspace, 1);

        let pinned = snapshot.get(0x55d0a3).unwrap();
        assert_eq!(pinned.workspace, -1);
        assert!(pinned.minimized);
        assert_eq!(pinned.mru, 2);
    }

    #[test]
    fn falls_back_to_focus_history_when_no_active_window() {
        let clients: Vec<Client> = serde_json::from_str(CLIENTS).unwrap();
        let active_workspace: Workspace = serde_json::from_str(r#"{"id": 1}"#).unwrap();
        let snapshot = build_snapshot(&clients, &[], &active_workspace, &ActiveWindow::default());
        assert_eq!(snapshot.focused().unwrap().id, 0x55d0a2);
    }
}
