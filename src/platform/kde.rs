//! KDE Plasma through a KWin script bridge.
//!
//! Plasma offers no window list or focus API to ordinary Wayland clients, and
//! it does not run GNOME Shell extensions. KWin does run scripts, though, and
//! a script can see `workspace.windowList()` and set `workspace.activeWindow`.
//! KWin scripts cannot register their own D-Bus service, but they can call one
//! asynchronously and receive the method's reply in a callback, so this module
//! owns a small session-bus service that the script polls:
//!
//! ```text
//! touchpad-snap (this process)                 KWin (org.kde.KWin)
//!   io.github.hjosugi.TouchpadSnap.KWin.P<pid>  <──  Poll(<json window list>)
//!   /io/github/hjosugi/TouchpadSnap/KWin         ──> "activate <internalId>" | ""
//! ```
//!
//! The script is embedded in the binary and written to
//! `$XDG_RUNTIME_DIR/touchpad-snap/` (or the cache directory) with a
//! per-process name before `org.kde.KWin.Scripting.LoadScript` loads it. The
//! per-process D-Bus name and script mean a daemon and a one-shot
//! `touchpad-snap next` can coexist. The script is unloaded again when the
//! backend is dropped; a process that is killed outright leaves an idle script
//! behind until KWin restarts (it stops polling when the service is gone).
//!
//! KWin 6 exposes `internalId` (a QUuid) on both X11 and Wayland, so
//! [`Window::id`] is a stable hash of that string. X11 window ids are used as a
//! fallback on older KWin versions. Hotkeys come from the XDG GlobalShortcuts
//! portal (see [`super::portal`]); this backend only lists and focuses windows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use zbus::blocking::Connection;
use zbus::fdo::RequestNameFlags;

use super::Backend;
use crate::model::{Snapshot, Window};

/// KWin's session-bus service and scripting object.
pub const KWIN_BUS_NAME: &str = "org.kde.KWin";
pub const KWIN_SCRIPTING_PATH: &str = "/Scripting";
pub const KWIN_SCRIPTING_INTERFACE: &str = "org.kde.kwin.Scripting";
pub const KWIN_SCRIPT_INTERFACE: &str = "org.kde.kwin.Script";

/// This bridge's D-Bus names. The per-process suffix lives in [`bus_name`].
pub const BUS_NAME: &str = "io.github.hjosugi.TouchpadSnap.KWin";
pub const OBJECT_PATH: &str = "/io/github/hjosugi/TouchpadSnap/KWin";
pub const INTERFACE: &str = "io.github.hjosugi.TouchpadSnap.KWin1";

const SCRIPT: &str = include_str!("kde/kwin-script.js");
const SERVICE_PLACEHOLDER: &str = "__TOUCHPAD_SNAP_SERVICE__";
const FIRST_POLL_TIMEOUT: Duration = Duration::from_secs(5);
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(2);

/// The session-bus name this process's KWin script calls. The pid keeps a
/// daemon and a one-shot command from stealing each other's bridge.
pub fn bus_name(pid: u32) -> String {
    format!("{BUS_NAME}.P{pid}")
}

/// The KWin plugin name used for `loadScript`/`unloadScript`. Unique per
/// process for the same reason as [`bus_name`].
pub fn plugin_name(pid: u32) -> String {
    format!("touchpad-snap-kde-bridge-{pid}")
}

/// Where the generated script is written. `$XDG_RUNTIME_DIR` is private to the
/// user and disappears at logout; the cache directory is the fallback.
pub fn script_path(pid: u32) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("touchpad-snap")
        .join(format!("kde-bridge-{pid}.js"))
}

/// What KDE environment variables were set. Pure so detection is testable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Detection {
    /// `KDE_FULL_SESSION=true`.
    pub full_session: bool,
    /// `XDG_CURRENT_DESKTOP` contains `KDE` or `Plasma` (`KDE`, `plasma`,
    /// `KDE:foo`, ...).
    pub desktop: bool,
    /// `KDE_SESSION_VERSION` is set (for example `6`).
    pub session_version: bool,
}

impl Detection {
    pub fn detected(self) -> bool {
        self.full_session || self.desktop || self.session_version
    }

    /// The variable that identified the session, for `doctor` and `describe`.
    pub fn source(self) -> Option<&'static str> {
        if self.full_session {
            Some("KDE_FULL_SESSION")
        } else if self.desktop {
            Some("XDG_CURRENT_DESKTOP")
        } else if self.session_version {
            Some("KDE_SESSION_VERSION")
        } else {
            None
        }
    }
}

/// Detects a Plasma session from `KDE_FULL_SESSION`, `XDG_CURRENT_DESKTOP` and
/// `KDE_SESSION_VERSION`. `get` keeps this testable without touching the real
/// environment.
pub fn detect<F>(get: F) -> Detection
where
    F: Fn(&str) -> Option<String>,
{
    let value = |key: &str| {
        get(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let full_session =
        value("KDE_FULL_SESSION").is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let desktop = value("XDG_CURRENT_DESKTOP").is_some_and(|desktop| {
        desktop
            .split([':', ';'])
            .map(str::trim)
            .any(|part| part.eq_ignore_ascii_case("kde") || part.eq_ignore_ascii_case("plasma"))
    });
    let session_version = value("KDE_SESSION_VERSION").is_some();
    Detection {
        full_session,
        desktop,
        session_version,
    }
}

pub fn detect_from_env() -> Detection {
    detect(|key| std::env::var(key).ok())
}

/// One line for `touchpad-snap doctor`; never fails when Plasma is not running.
pub fn doctor() -> String {
    let detection = detect_from_env();
    if !detection.detected() {
        return "not detected".to_string();
    }
    let source = detection.source().unwrap_or("KDE");
    let conn = match Connection::session() {
        Ok(conn) => conn,
        Err(e) => return format!("detected ({source}); session bus unavailable: {e}"),
    };
    let owner: bool = match conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(KWIN_BUS_NAME,),
    ) {
        Ok(reply) => reply.body().deserialize().unwrap_or(false),
        Err(e) => return format!("detected ({source}); could not ask the session bus: {e}"),
    };
    if !owner {
        return format!("detected ({source}); KWin is not on the session bus");
    }
    // isScriptLoaded is part of the same scripting interface loadScript uses;
    // calling it proves the bridge can be loaded without creating a script.
    match conn.call_method(
        Some(KWIN_BUS_NAME),
        KWIN_SCRIPTING_PATH,
        Some(KWIN_SCRIPTING_INTERFACE),
        "isScriptLoaded",
        &("",),
    ) {
        Ok(reply) => match reply.body().deserialize::<bool>() {
            Ok(_) => format!(
                "detected ({source}); KWin scripting is reachable; the bridge loads on demand"
            ),
            Err(e) => format!("detected ({source}); unexpected KWin scripting reply: {e}"),
        },
        Err(e) => format!(
            "detected ({source}); KWin is running but scripting is unavailable: {}",
            explain(e)
        ),
    }
}

/// A command the script applies after its next snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Activate(String),
}

impl Command {
    pub fn encode(&self) -> String {
        match self {
            Command::Activate(token) => format!("activate {token}"),
        }
    }
}

pub fn parse_command(command: &str) -> Option<Command> {
    let token = command.strip_prefix("activate ")?;
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return None;
    }
    Some(Command::Activate(token.to_string()))
}

/// A parsed snapshot plus the mapping needed to activate a window later.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapshotData {
    pub snapshot: Snapshot,
    /// `Window::id` to KWin's `internalId` (or the X11 window id).
    pub tokens: HashMap<u64, String>,
}

/// Parses the JSON the KWin script sends.
pub fn parse_snapshot(json: &str) -> Result<SnapshotData> {
    let raw: RawSnapshot =
        serde_json::from_str(json).context("parsing the window list from the KWin script")?;
    let active_workspace = raw.active_workspace;
    let mut entries: Vec<(i32, usize, Window)> = Vec::new();
    let mut tokens = HashMap::new();
    for (index, window) in raw.windows.into_iter().enumerate() {
        // KWin's own "would this be in a taskbar?" judgement. We keep minimized
        // windows (the core may skip them) but drop panels, docks and overlays.
        if window.deleted || window.skip_taskbar || !window.normal_window || window.id.is_empty() {
            continue;
        }
        let token = window.id.clone();
        let stacking = window.stacking_order;
        let mapped = window.into_window(active_workspace);
        let _ = tokens.insert(mapped.id, token);
        entries.push((stacking, index, mapped));
    }

    // KWin's stacking order is best effort MRU: the active window is normally
    // on top. Ties (and older KWin without the property) keep KWin's list order.
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let windows = entries
        .into_iter()
        .enumerate()
        .map(|(mru, (_, _, mut window))| {
            window.mru = u32::try_from(mru).unwrap_or(u32::MAX);
            window
        })
        .collect();

    Ok(SnapshotData {
        snapshot: Snapshot {
            active_workspace,
            windows,
        },
        tokens,
    })
}

/// FNV-1a 64 over KWin's `internalId` string. Stable for the window's lifetime
/// and across restarts; the script keeps the original string for activation.
pub fn window_id(token: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in token.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawSnapshot {
    active_workspace: i32,
    windows: Vec<RawWindow>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawWindow {
    /// KWin's internalId, or the X11 window id when internalId is absent.
    id: String,
    title: String,
    resource_class: String,
    resource_name: String,
    desktop_file: String,
    pid: i64,
    desktops: Vec<i32>,
    on_all_desktops: bool,
    /// Sent by the script for completeness; touchpad-snap's model groups by
    /// desktop, so activities are parsed but do not choose the workspace.
    #[allow(dead_code)]
    activities: Vec<String>,
    minimized: bool,
    active: bool,
    skip_taskbar: bool,
    normal_window: bool,
    deleted: bool,
    stacking_order: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl RawWindow {
    fn into_window(self, active_workspace: i32) -> Window {
        let workspace = if self.on_all_desktops || self.desktops.is_empty() {
            -1
        } else if self.desktops.contains(&active_workspace) {
            active_workspace
        } else {
            self.desktops[0]
        };
        let app_id = if self.desktop_file.is_empty() {
            self.resource_class.clone()
        } else {
            self.desktop_file
        };
        let app_name = if !self.resource_class.is_empty() {
            self.resource_class.clone()
        } else if !self.resource_name.is_empty() {
            self.resource_name.clone()
        } else {
            app_id.clone()
        };
        Window {
            id: window_id(&self.id),
            title: self.title,
            app_id,
            app_name,
            wm_class: self.resource_class,
            pid: u32::try_from(self.pid).unwrap_or(0),
            workspace,
            minimized: self.minimized,
            focused: self.active,
            mru: 0,
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Default)]
struct State {
    seq: u64,
    polls: u64,
    last: Option<Result<SnapshotData, String>>,
    pending: Option<String>,
    /// The snapshot sequence at which a pending command was handed to KWin.
    delivered_seq: Option<u64>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

struct Bridge {
    state: Arc<Mutex<State>>,
    wake: Arc<Condvar>,
}

#[zbus::interface(name = "io.github.hjosugi.TouchpadSnap.KWin1")]
impl Bridge {
    /// Called by the KWin script with a JSON snapshot; returns the command to
    /// apply. The script passes the return value to its callback and runs it.
    fn poll(&self, payload: String) -> String {
        let parsed = parse_snapshot(&payload).map_err(|error| format!("{error:#}"));
        let mut state = lock(&self.state);
        state.seq += 1;
        state.polls += 1;
        state.last = Some(parsed);
        let command = state.pending.take().unwrap_or_default();
        if !command.is_empty() {
            state.delivered_seq = Some(state.seq);
        }
        drop(state);
        self.wake.notify_all();
        command
    }
}

pub struct Kde {
    conn: Connection,
    state: Arc<Mutex<State>>,
    wake: Arc<Condvar>,
    plugin: String,
    script_path: PathBuf,
    bus: String,
    loaded: bool,
}

impl Kde {
    pub fn connect() -> Result<Self> {
        let pid = std::process::id();
        let bus = bus_name(pid);
        let plugin = plugin_name(pid);
        let conn = Connection::session().context("connecting to the D-Bus session bus")?;
        conn.request_name_with_flags(bus.as_str(), RequestNameFlags::DoNotQueue.into())
            .with_context(|| format!("claiming the KWin bridge name {bus}"))?;

        let state = Arc::new(Mutex::new(State::default()));
        let wake = Arc::new(Condvar::new());
        conn.object_server()
            .at(
                OBJECT_PATH,
                Bridge {
                    state: Arc::clone(&state),
                    wake: Arc::clone(&wake),
                },
            )
            .context("registering the KWin bridge object")?;

        let script_path = script_path(pid);
        write_script(&script_path, &bus)?;

        let id = match load_script(&conn, &script_path, &plugin) {
            Ok(id) if id >= 0 => id,
            Ok(_) => {
                // A previous process with this pid was killed without
                // unloading its script. Take the name over.
                let _ = unload_script(&conn, &plugin);
                let id = load_script(&conn, &script_path, &plugin)?;
                if id < 0 {
                    let _ = std::fs::remove_file(&script_path);
                    bail!("KWin refused to load the touchpad-snap bridge script");
                }
                id
            }
            Err(error) => {
                let _ = std::fs::remove_file(&script_path);
                return Err(error);
            }
        };

        let kde = Self {
            conn,
            state,
            wake,
            plugin,
            script_path,
            bus,
            loaded: true,
        };
        run_script(&kde.conn, id)?;
        kde.wait_first_poll(FIRST_POLL_TIMEOUT)?;
        Ok(kde)
    }

    fn wait_first_poll(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            if state.polls > 0 {
                return match &state.last {
                    Some(Ok(_)) => Ok(()),
                    Some(Err(error)) => {
                        bail!("the KWin bridge sent an unreadable window list: {error}")
                    }
                    None => Ok(()),
                };
            }
            let now = Instant::now();
            if now >= deadline {
                bail!(
                    "KWin loaded the bridge script but it did not call back within {}s; \
                     check `journalctl --user -u plasma-kwin_wayland` for script errors",
                    timeout.as_secs()
                );
            }
            let (next, _) = self
                .wake
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
        }
    }
}

impl Backend for Kde {
    fn snapshot(&self) -> Result<Snapshot> {
        let state = lock(&self.state);
        match &state.last {
            Some(Ok(data)) => Ok(data.snapshot.clone()),
            Some(Err(error)) => {
                bail!("the KWin bridge sent an unreadable window list: {error}")
            }
            None => bail!("the KWin bridge has not reported any windows"),
        }
    }

    fn activate(&self, id: u64) -> Result<()> {
        let token = {
            let state = lock(&self.state);
            match &state.last {
                Some(Ok(data)) => data.tokens.get(&id).cloned(),
                _ => None,
            }
        };
        let token = token.ok_or_else(|| anyhow!("window {id} is gone"))?;

        let deadline = Instant::now() + ACTIVATE_TIMEOUT;
        {
            let mut state = lock(&self.state);
            state.pending = Some(Command::Activate(token).encode());
            state.delivered_seq = None;
        }

        let mut state = lock(&self.state);
        loop {
            // The snapshot in the Poll that carried the command is from before
            // KWin applied it; wait for a later one that reports the focus.
            if state.delivered_seq.is_some()
                && let Some(Ok(data)) = &state.last
                && data
                    .snapshot
                    .focused()
                    .is_some_and(|window| window.id == id)
            {
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                let delivered = state.delivered_seq.is_some();
                drop(state);
                if delivered {
                    bail!(
                        "KWin did not activate window {id} \
                         (focus stealing prevention, or the window is gone)"
                    );
                }
                bail!(
                    "the KWin bridge did not poll while activating window {id}; \
                     is KWin responsive?"
                );
            }
            let (next, _) = self
                .wake
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
        }
    }

    fn describe(&self) -> String {
        let version = std::env::var("KDE_SESSION_VERSION")
            .ok()
            .map(|version| version.trim().to_string())
            .filter(|version| !version.is_empty())
            .map(|version| format!(" {version}"))
            .unwrap_or_default();
        format!(
            "KDE Plasma{version} (KWin script bridge {}, bridge {})",
            self.plugin, self.bus
        )
    }
}

impl Drop for Kde {
    fn drop(&mut self) {
        if self.loaded {
            let _ = unload_script(&self.conn, &self.plugin);
            self.loaded = false;
        }
        let _ = std::fs::remove_file(&self.script_path);
    }
}

fn write_script(path: &Path, bus: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let script = SCRIPT.replace(SERVICE_PLACEHOLDER, bus);
    std::fs::write(path, script).with_context(|| format!("writing {}", path.display()))
}

fn load_script(conn: &Connection, path: &Path, plugin: &str) -> Result<i32> {
    let reply = conn
        .call_method(
            Some(KWIN_BUS_NAME),
            KWIN_SCRIPTING_PATH,
            Some(KWIN_SCRIPTING_INTERFACE),
            "loadScript",
            &(path.to_string_lossy().into_owned(), plugin.to_string()),
        )
        .map_err(explain)?;
    reply
        .body()
        .deserialize()
        .context("reading KWin's loadScript reply")
}

fn run_script(conn: &Connection, id: i32) -> Result<()> {
    let path = format!("{KWIN_SCRIPTING_PATH}/Script{id}");
    conn.call_method(
        Some(KWIN_BUS_NAME),
        path.as_str(),
        Some(KWIN_SCRIPT_INTERFACE),
        "run",
        &(),
    )
    .map_err(explain)?;
    Ok(())
}

fn unload_script(conn: &Connection, plugin: &str) -> Result<bool> {
    let reply = conn
        .call_method(
            Some(KWIN_BUS_NAME),
            KWIN_SCRIPTING_PATH,
            Some(KWIN_SCRIPTING_INTERFACE),
            "unloadScript",
            &(plugin.to_string(),),
        )
        .map_err(explain)?;
    reply
        .body()
        .deserialize()
        .context("reading KWin's unloadScript reply")
}

fn explain(error: zbus::Error) -> anyhow::Error {
    let text = error.to_string();
    if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        anyhow!(
            "KWin is not reachable on the session bus. \
             Start touchpad-snap inside a Plasma session (is KWin running?)"
        )
    } else if text.contains("UnknownMethod") || text.contains("UnknownInterface") {
        anyhow!(
            "this KWin does not expose the scripting interface touchpad-snap needs \
             (KWin 5.20+ / Plasma 6)"
        )
    } else {
        anyhow::Error::new(error)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn detects_kde_from_each_variable() {
        let full = detect(lookup(&[("KDE_FULL_SESSION", "true")]));
        assert!(full.detected());
        assert_eq!(full.source(), Some("KDE_FULL_SESSION"));

        for desktop in ["KDE", "plasma", "Plasma", "ubuntu:KDE", "GNOME:KDE"] {
            let detection = detect(lookup(&[("XDG_CURRENT_DESKTOP", desktop)]));
            assert!(detection.detected(), "{desktop}");
            assert_eq!(detection.source(), Some("XDG_CURRENT_DESKTOP"), "{desktop}");
        }

        let version = detect(lookup(&[("KDE_SESSION_VERSION", "6")]));
        assert!(version.detected());
        assert_eq!(version.source(), Some("KDE_SESSION_VERSION"));
    }

    #[test]
    fn other_desktops_and_empty_values_are_not_kde() {
        for desktop in ["GNOME", "ubuntu:GNOME", "sway", "Hyprland", "niri", ""] {
            assert!(
                !detect(lookup(&[("XDG_CURRENT_DESKTOP", desktop)])).detected(),
                "{desktop}"
            );
        }
        let empty = detect(lookup(&[
            ("KDE_FULL_SESSION", ""),
            ("KDE_SESSION_VERSION", "  "),
            ("XDG_CURRENT_DESKTOP", "GNOME"),
        ]));
        assert!(!empty.detected());
        assert_eq!(empty.source(), None);

        // KDE_FULL_SESSION must actually say true.
        assert!(!detect(lookup(&[("KDE_FULL_SESSION", "false")])).detected());
        assert!(!detect(lookup(&[])).detected());
    }

    #[test]
    fn bridge_names_are_per_process_and_namespaced() {
        assert_eq!(bus_name(42), "io.github.hjosugi.TouchpadSnap.KWin.P42");
        assert_eq!(plugin_name(42), "touchpad-snap-kde-bridge-42");
        assert_eq!(
            bus_name(42).rsplit('.').next(),
            Some("P42"),
            "the pid suffix is a valid D-Bus name element"
        );
        assert_eq!(OBJECT_PATH, "/io/github/hjosugi/TouchpadSnap/KWin");
        assert_eq!(INTERFACE, "io.github.hjosugi.TouchpadSnap.KWin1");
        assert!(script_path(42).ends_with("touchpad-snap/kde-bridge-42.js"));
        assert!(SCRIPT.contains(SERVICE_PLACEHOLDER));
        assert!(SCRIPT.contains("callDBus"));
        assert!(SCRIPT.contains("windowList"));
    }

    #[test]
    fn window_ids_are_stable_fnv1a_hashes() {
        assert_eq!(window_id(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(window_id("a"), 0xaf63_dc4c_8601_ec8c);
        let uuid = "{5a5c0c2a-0000-4000-8000-000000000000}";
        assert_eq!(window_id(uuid), window_id(uuid));
        assert_ne!(window_id(uuid), window_id(""));
        assert_ne!(window_id("window-a"), window_id("window-b"));
    }

    #[test]
    fn commands_round_trip() {
        let command = Command::Activate("{abc}".into());
        assert_eq!(command.encode(), "activate {abc}");
        assert_eq!(parse_command("activate {abc}"), Some(command));
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("activate"), None);
        assert_eq!(parse_command("activate "), None);
        assert_eq!(parse_command("activate a b"), None);
        assert_eq!(parse_command("focus {abc}"), None);
    }

    const SNAPSHOT: &str = r#"{
        "version": 1,
        "active_workspace": 1,
        "windows": [
            {"id":"{a}","title":"Editor","resource_class":"code","resource_name":"code",
             "desktop_file":"code","pid":100,"desktops":[1],"on_all_desktops":false,
             "activities":["act1"],"minimized":false,"active":true,"skip_taskbar":false,
             "normal_window":true,"deleted":false,"stacking_order":9,
             "x":0,"y":0,"width":800,"height":600},
            {"id":"{b}","title":"Terminal","resource_class":"konsole","resource_name":"konsole",
             "desktop_file":"","pid":101,"desktops":[1],"on_all_desktops":false,
             "activities":["act1"],"minimized":true,"active":false,"skip_taskbar":false,
             "normal_window":true,"deleted":false,"stacking_order":4,
             "x":10,"y":20,"width":700,"height":500},
            {"id":"{c}","title":"Sticky","resource_class":"notes","resource_name":"notes",
             "desktop_file":"","pid":102,"desktops":[],"on_all_desktops":true,
             "activities":[],"minimized":false,"active":false,"skip_taskbar":false,
             "normal_window":true,"deleted":false,"stacking_order":2,
             "x":30,"y":40,"width":300,"height":200},
            {"id":"{d}","title":"Elsewhere","resource_class":"other","resource_name":"other",
             "desktop_file":"","pid":103,"desktops":[0],"on_all_desktops":false,
             "activities":["act1"],"minimized":false,"active":false,"skip_taskbar":false,
             "normal_window":true,"deleted":false,"stacking_order":6,
             "x":50,"y":60,"width":400,"height":300},
            {"id":"{e}","title":"Panel","resource_class":"panel","resource_name":"panel",
             "desktop_file":"","pid":104,"desktops":[1],"on_all_desktops":false,
             "activities":["act1"],"minimized":false,"active":false,"skip_taskbar":true,
             "normal_window":true,"deleted":false,"stacking_order":8,
             "x":0,"y":0,"width":1920,"height":30},
            {"id":"{f}","title":"Special","resource_class":"special","resource_name":"special",
             "desktop_file":"","pid":105,"desktops":[1],"on_all_desktops":false,
             "activities":["act1"],"minimized":false,"active":false,"skip_taskbar":false,
             "normal_window":false,"deleted":false,"stacking_order":7,
             "x":0,"y":0,"width":100,"height":100},
            {"id":"{g}","title":"Deleted","resource_class":"deleted","resource_name":"deleted",
             "desktop_file":"","pid":106,"desktops":[1],"on_all_desktops":false,
             "activities":["act1"],"minimized":false,"active":false,"skip_taskbar":false,
             "normal_window":true,"deleted":true,"stacking_order":10,
             "x":0,"y":0,"width":100,"height":100},
            {"id":"","title":"No id","resource_class":"none","resource_name":"none",
             "desktop_file":"","pid":107,"desktops":[1],"on_all_desktops":false,
             "activities":["act1"],"minimized":false,"active":false,"skip_taskbar":false,
             "normal_window":true,"deleted":false,"stacking_order":11,
             "x":0,"y":0,"width":100,"height":100}
        ]
    }"#;

    #[test]
    fn maps_kwin_windows_filters_and_mru() {
        let data = parse_snapshot(SNAPSHOT).unwrap();
        let snapshot = &data.snapshot;
        assert_eq!(snapshot.active_workspace, 1);

        // Deleted, skipTaskbar, non-normal and id-less windows are gone.
        let ids: Vec<u64> = snapshot.windows.iter().map(|window| window.id).collect();
        assert_eq!(ids.len(), 4);
        assert_eq!(
            ids,
            vec![
                window_id("{a}"),
                window_id("{d}"),
                window_id("{b}"),
                window_id("{c}")
            ],
            "stacking order descending is the best-effort MRU"
        );
        assert_eq!(
            snapshot
                .windows
                .iter()
                .map(|window| window.mru)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );

        let editor = snapshot.get(window_id("{a}")).unwrap();
        assert_eq!(editor.title, "Editor");
        assert_eq!(editor.app_id, "code");
        assert_eq!(editor.app_name, "code");
        assert_eq!(editor.wm_class, "code");
        assert_eq!(editor.pid, 100);
        assert_eq!(editor.workspace, 1);
        assert!(!editor.minimized);
        assert!(editor.focused);
        assert_eq!(
            (editor.x, editor.y, editor.width, editor.height),
            (0, 0, 800, 600)
        );

        let terminal = snapshot.get(window_id("{b}")).unwrap();
        assert_eq!(terminal.app_id, "konsole", "empty desktop_file falls back");
        assert!(terminal.minimized);
        assert_eq!(terminal.workspace, 1);

        let sticky = snapshot.get(window_id("{c}")).unwrap();
        assert_eq!(sticky.workspace, -1, "on all desktops");

        let elsewhere = snapshot.get(window_id("{d}")).unwrap();
        assert_eq!(elsewhere.workspace, 0, "window on an inactive desktop");

        assert_eq!(
            data.tokens.get(&window_id("{a}")).map(String::as_str),
            Some("{a}")
        );
        assert_eq!(
            data.tokens.get(&window_id("{b}")).map(String::as_str),
            Some("{b}")
        );
        assert!(!data.tokens.contains_key(&window_id("{e}")));
    }

    #[test]
    fn malformed_or_empty_json_is_an_error() {
        assert!(parse_snapshot("not json").is_err());
        assert!(parse_snapshot("").is_err());
        let empty = parse_snapshot("{}").unwrap();
        assert_eq!(empty.snapshot, Snapshot::default());
        assert!(empty.tokens.is_empty());
    }
}
