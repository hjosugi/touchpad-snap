//! Platform backends.
//!
//! A backend lists windows and activates one ([`Backend`]); the daemon side
//! ([`run_daemon`]) also turns the platform's hotkey and gesture plumbing into
//! [`Event`]s. Everything else — which window to pick, what a swipe means —
//! is platform-neutral and lives outside this module.

use std::path::Path;

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;

use crate::config::{Claim, Config};
use crate::gesture::Phase;
use crate::hotkey::{Hotkey, Mods};
use crate::model::Snapshot;
use crate::when::PlatformDescription;

#[cfg(target_os = "linux")]
pub mod gnome;
#[cfg(target_os = "linux")]
pub mod kde;
#[cfg(target_os = "linux")]
pub mod libinput;
#[cfg(target_os = "linux")]
pub mod portal;
#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
mod windows_hud;
#[cfg(windows)]
mod windows_touchpad;
#[cfg(any(windows, test))]
mod windows_tray;
#[cfg(target_os = "linux")]
pub mod wlroots;

pub trait Backend {
    fn snapshot(&self) -> Result<Snapshot>;
    fn activate(&self, id: u64) -> Result<()>;
    /// Briefly shows which window was jumped to (`index` is 1-based within
    /// `total`). Best effort; backends without an overlay do nothing.
    fn show_hud(&self, _id: u64, _index: usize, _total: usize, _duration_ms: u64) -> Result<()> {
        Ok(())
    }
    /// Types `keys` into the focused window. `held` are modifiers that are
    /// physically down (the trigger's own); `Alt` + swipe sending `Ctrl+Tab`
    /// has to arrive as `Ctrl+Tab`. A backend that can release keys does so
    /// while it types and presses them again afterwards (Windows); one that
    /// cannot types when they come up (GNOME: mutter ignores a virtual
    /// release of a key the user holds).
    fn send_keys(&self, _keys: Hotkey, _held: Mods) -> Result<()> {
        anyhow::bail!("sending keys is not supported on this desktop yet")
    }
    /// One line for `touchpad-snap doctor`.
    fn describe(&self) -> String;
    /// What an installation under the old name (see [`crate::legacy`]) still
    /// has running here and how to get rid of it, for `touchpad-snap doctor`.
    fn leftovers(&self) -> Vec<String> {
        Vec::new()
    }
}

/// How often the platform loops send [`Event::Tick`].
pub const TICK: std::time::Duration = std::time::Duration::from_secs(2);

/// Something the daemon reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Sent about every [`TICK`] for housekeeping (the config file check).
    Tick,
    /// The user asked to re-read the config file (the Windows tray menu).
    Reload,
    /// The hotkey at this index of `Config::hotkeys` was pressed. With
    /// `tracked`, the platform sends [`Event::Released`] once the modifiers
    /// held for it are all up again, so presses in between form one session.
    Hotkey { index: usize, tracked: bool },
    /// The modifiers of the last tracked hotkey have all been released.
    Released,
    /// Motion of a claimed touchpad swipe. `app_id` is the focused app at
    /// Begin when the platform knows it (GNOME protocol 4+); the daemon
    /// otherwise snapshots the focused window at Begin.
    Swipe {
        phase: Phase,
        claim: Claim,
        app_id: Option<String>,
        dx: f64,
        dy: f64,
    },
}

/// Connects to the backend for the running desktop.
pub fn connect() -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    {
        if is_gnome(std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref()) {
            return Ok(Box::new(gnome::Gnome::connect()?));
        }
        // Plasma does not run the GNOME extension; a KWin script loaded on
        // demand provides the window list and focus. Its hotkeys go through the
        // portal, so the daemon is handled below like the other non-GNOME case.
        if kde::detect_from_env().detected() {
            return Ok(Box::new(kde::Kde::connect()?));
        }
        match wlroots::Wlroots::connect() {
            Ok(backend) => Ok(Box::new(backend)),
            Err(wlroots_error) => {
                // Only fall back to GNOME when nothing pointed at a wlroots
                // compositor; a stale SWAYSOCK or NIRI_SOCKET should say so.
                if wlroots::detect_from_env().is_ok() || wlroots::desktop_hint_from_env().is_some()
                {
                    return Err(wlroots_error);
                }
                match gnome::Gnome::connect() {
                    Ok(backend) => Ok(Box::new(backend)),
                    Err(_) => Err(wlroots_error),
                }
            }
        }
    }
    #[cfg(windows)]
    {
        Ok(Box::new(windows::Win32::new()))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        anyhow::bail!(
            "touchpad-snap supports GNOME, KDE Plasma, Sway/Hyprland/niri (Linux) and Windows only, for now"
        )
    }
}

/// What the daemon's event handler returns: `Some(config)` when the config
/// changed and the platform has to register its hotkeys and swipes again.
pub type Handler<'a> = dyn FnMut(&dyn Backend, Event) -> Option<Config> + 'a;

/// Environment variable choosing where Linux hotkeys come from. `portal`
/// selects the XDG GlobalShortcuts portal; unset keeps GNOME's extension
/// grabs when running GNOME, and tries the portal elsewhere.
#[cfg(target_os = "linux")]
pub const HOTKEY_SOURCE_ENV: &str = "TOUCHPAD_SNAP_HOTKEYS";

/// Where global hotkeys are registered on Linux.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeySource {
    /// The touchpad-snap GNOME Shell extension (`SetAccelerators`).
    Gnome,
    /// `org.freedesktop.portal.GlobalShortcuts`.
    Portal,
}

/// Chooses the hotkey source from [`HOTKEY_SOURCE_ENV`] and the desktop name
/// (`XDG_CURRENT_DESKTOP`). Pure so the decision is unit-tested.
#[cfg(target_os = "linux")]
pub fn hotkey_source(requested: Option<&str>, desktop: Option<&str>) -> HotkeySource {
    match requested.map(str::trim) {
        Some("portal") | Some("globalshortcuts") => HotkeySource::Portal,
        Some("extension") | Some("gnome") => HotkeySource::Gnome,
        _ if is_gnome(desktop) => HotkeySource::Gnome,
        _ => HotkeySource::Portal,
    }
}

/// Whether `XDG_CURRENT_DESKTOP` names GNOME (possibly as one of several
/// desktops, e.g. `ubuntu:GNOME` or `GNOME:sway`).
#[cfg(target_os = "linux")]
pub fn is_gnome(desktop: Option<&str>) -> bool {
    desktop.is_some_and(|desktop| {
        desktop
            .split([':', ';'])
            .any(|part| part.trim().eq_ignore_ascii_case("gnome"))
    })
}

/// The `when.desktop` name in an `XDG_CURRENT_DESKTOP` value. Pure so the
/// platform description is testable without touching the environment.
pub fn desktop_name(desktop: Option<&str>) -> Option<&'static str> {
    let desktop = desktop?;
    for part in desktop.split([':', ';']) {
        let part = part.trim();
        if part.eq_ignore_ascii_case("gnome") {
            return Some("gnome");
        }
        if part.eq_ignore_ascii_case("kde") || part.eq_ignore_ascii_case("plasma") {
            return Some("kde");
        }
        if part.eq_ignore_ascii_case("sway") {
            return Some("sway");
        }
        if part.eq_ignore_ascii_case("hyprland") {
            return Some("hyprland");
        }
        if part.eq_ignore_ascii_case("niri") {
            return Some("niri");
        }
    }
    None
}

fn current_os() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(windows) {
        "windows"
    } else {
        std::env::consts::OS
    }
}

/// What `when.os` and `when.desktop` are checked against on this machine.
pub fn current_platform() -> PlatformDescription {
    static CURRENT: std::sync::OnceLock<PlatformDescription> = std::sync::OnceLock::new();
    *CURRENT.get_or_init(detect_current_platform)
}

fn detect_current_platform() -> PlatformDescription {
    #[cfg(target_os = "linux")]
    {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        let name = if is_gnome(desktop.as_deref()) {
            Some("gnome")
        } else if kde::detect_from_env().detected() {
            Some("kde")
        } else if let Some(compositor) = wlroots::desktop_hint_from_env() {
            Some(match compositor {
                wlroots::Compositor::Sway => "sway",
                wlroots::Compositor::Hyprland => "hyprland",
                wlroots::Compositor::Niri => "niri",
            })
        } else if gnome::Gnome::extension_available() {
            // Some launchers (and test sessions) leave XDG_CURRENT_DESKTOP
            // unset; a reachable extension is as good a sign as run_daemon uses.
            Some("gnome")
        } else {
            desktop_name(desktop.as_deref())
        };
        PlatformDescription::new(current_os(), name)
    }
    #[cfg(not(target_os = "linux"))]
    {
        PlatformDescription::new(current_os(), None)
    }
}

/// Holds the daemon's session-bus name so a second daemon refuses to start.
#[cfg(target_os = "linux")]
pub(crate) fn claim_daemon_name(conn: &zbus::blocking::Connection) -> Result<()> {
    use zbus::fdo::RequestNameFlags;
    const DAEMON_NAME: &str = "io.github.hjosugi.TouchpadSnap.Daemon";
    match conn.request_name_with_flags(DAEMON_NAME, RequestNameFlags::DoNotQueue.into()) {
        Err(zbus::Error::NameTaken) => {
            anyhow::bail!("another touchpad-snap daemon is already running")
        }
        Err(e) => Err(e).context("claiming the daemon bus name"),
        Ok(_) => Ok(()),
    }
}

/// Registers the configured hotkeys and swipes, then calls `handle` for every
/// event until the process is stopped. `hotkeys` must be [`Config::hotkeys`]
/// for the same `cfg`. `config_path` is the file the Windows tray opens and
/// reloads.
pub fn run_daemon(
    cfg: &Config,
    hotkeys: &[Hotkey],
    config_path: &Path,
    verbose: bool,
    handle: &mut Handler,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let _ = config_path;
        let requested = std::env::var(HOTKEY_SOURCE_ENV)
            .or_else(|_| std::env::var(crate::legacy::HOTKEY_SOURCE_ENV))
            .ok();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        // A running extension means GNOME even when XDG_CURRENT_DESKTOP is
        // unset (headless sessions, odd launchers).
        let gnome = is_gnome(desktop.as_deref()) || gnome::Gnome::extension_available();
        let source = hotkey_source(
            requested.as_deref(),
            if gnome {
                Some("GNOME")
            } else {
                desktop.as_deref()
            },
        );
        if gnome {
            return match source {
                HotkeySource::Gnome => gnome::run_daemon(cfg, hotkeys, verbose, handle),
                // The extension still serves swipes and the HUD; only the
                // hotkey grabs move to the portal.
                HotkeySource::Portal => {
                    gnome::run_daemon_with_portal(cfg, hotkeys, verbose, handle)
                }
            };
        }
        // Plasma has no extension; the KDE backend lists and focuses windows
        // while the portal serves the hotkeys. Respect an explicit override,
        // even though there is no GNOME extension to talk to on Plasma.
        if kde::detect_from_env().detected() {
            return match source {
                HotkeySource::Portal => portal::run_hotkeys_with(cfg, hotkeys, verbose, handle),
                HotkeySource::Gnome => gnome::run_daemon(cfg, hotkeys, verbose, handle),
            };
        }
        // Hotkeys are normally bound in the compositor config; the daemon is
        // optional there, so say how rather than grabbing keys behind its back.
        if wlroots::detect_from_env().is_ok() || wlroots::desktop_hint_from_env().is_some() {
            return wlroots::run_daemon(cfg, verbose, handle);
        }
        match source {
            HotkeySource::Portal => portal::run_hotkeys_with(cfg, hotkeys, verbose, handle),
            HotkeySource::Gnome => gnome::run_daemon(cfg, hotkeys, verbose, handle),
        }
    }
    #[cfg(windows)]
    {
        windows::run_daemon(cfg, hotkeys, config_path, verbose, handle)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (cfg, hotkeys, config_path, verbose, handle);
        anyhow::bail!(
            "touchpad-snap supports GNOME, KDE Plasma, Sway/Hyprland/niri (Linux) and Windows only, for now"
        )
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn gnome_is_recognised_among_other_desktop_components() {
        assert!(is_gnome(Some("ubuntu:GNOME")));
        assert!(is_gnome(Some("GNOME")));
        assert!(is_gnome(Some("GNOME:sway")));
        assert!(!is_gnome(Some("sway")));
        assert!(!is_gnome(Some("Hyprland")));
        assert!(!is_gnome(None));
    }

    #[test]
    fn desktop_names_map_to_the_when_values() {
        assert_eq!(desktop_name(Some("ubuntu:GNOME")), Some("gnome"));
        assert_eq!(desktop_name(Some("KDE")), Some("kde"));
        assert_eq!(desktop_name(Some("plasma")), Some("kde"));
        assert_eq!(desktop_name(Some("sway")), Some("sway"));
        assert_eq!(desktop_name(Some("Hyprland")), Some("hyprland"));
        assert_eq!(desktop_name(Some("niri")), Some("niri"));
        assert_eq!(desktop_name(Some("XFCE")), None);
        assert_eq!(desktop_name(None), None);
    }

    #[test]
    fn portal_is_opt_in_on_gnome_and_the_default_elsewhere() {
        assert_eq!(hotkey_source(None, Some("GNOME")), HotkeySource::Gnome);
        assert_eq!(
            hotkey_source(None, Some("ubuntu:GNOME")),
            HotkeySource::Gnome
        );
        assert_eq!(hotkey_source(None, Some("KDE")), HotkeySource::Portal);
        assert_eq!(hotkey_source(None, None), HotkeySource::Portal);
    }

    #[test]
    fn the_environment_can_override_the_desktop_default() {
        assert_eq!(
            hotkey_source(Some("portal"), Some("GNOME")),
            HotkeySource::Portal
        );
        assert_eq!(
            hotkey_source(Some(" portal "), Some("GNOME")),
            HotkeySource::Portal
        );
        assert_eq!(
            hotkey_source(Some("extension"), Some("KDE")),
            HotkeySource::Gnome
        );
        assert_eq!(
            hotkey_source(Some("nonsense"), Some("GNOME")),
            HotkeySource::Gnome
        );
    }
}
