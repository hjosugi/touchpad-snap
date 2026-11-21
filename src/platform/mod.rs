//! Platform backends.
//!
//! A backend lists windows and activates one ([`Backend`]); the daemon side
//! ([`run_daemon`]) also turns the platform's hotkey and gesture plumbing into
//! [`Event`]s. Everything else — which window to pick, what a swipe means —
//! is platform-neutral and lives outside this module.

use anyhow::Result;

use crate::config::{Claim, Config};
use crate::gesture::Phase;
use crate::hotkey::Hotkey;
use crate::model::Snapshot;

#[cfg(target_os = "linux")]
pub mod gnome;
#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
mod windows_hud;

pub trait Backend {
    fn snapshot(&self) -> Result<Snapshot>;
    fn activate(&self, id: u64) -> Result<()>;
    /// Briefly shows which window was jumped to (`index` is 1-based within
    /// `total`). Best effort; backends without an overlay do nothing.
    fn show_hud(&self, _id: u64, _index: usize, _total: usize, _duration_ms: u64) -> Result<()> {
        Ok(())
    }
    /// One line for `window-jump doctor`.
    fn describe(&self) -> String;
}

/// How often the platform loops send [`Event::Tick`].
pub const TICK: std::time::Duration = std::time::Duration::from_secs(2);

/// Something the daemon reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Sent about every [`TICK`] for housekeeping (the config file check).
    Tick,
    /// The hotkey at this index of `Config::hotkeys` was pressed. With
    /// `tracked`, the platform sends [`Event::Released`] once the modifiers
    /// held for it are all up again, so presses in between form one session.
    Hotkey { index: usize, tracked: bool },
    /// The modifiers of the last tracked hotkey have all been released.
    Released,
    /// Motion of a claimed touchpad swipe.
    Swipe {
        phase: Phase,
        claim: Claim,
        dx: f64,
        dy: f64,
    },
}

/// Connects to the backend for the running desktop.
pub fn connect() -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(gnome::Gnome::connect()?))
    }
    #[cfg(windows)]
    {
        Ok(Box::new(windows::Win32::new()))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        anyhow::bail!("window-jump supports GNOME (Linux) and Windows only, for now")
    }
}

/// What the daemon's event handler returns: `Some(config)` when the config
/// changed and the platform has to register its hotkeys and swipes again.
pub type Handler<'a> = dyn FnMut(&dyn Backend, Event) -> Option<Config> + 'a;

/// Registers the configured hotkeys and swipes, then calls `handle` for every
/// event until the process is stopped. `cfg` must have passed
/// [`Config::validate`], which also produced `hotkeys`.
pub fn run_daemon(cfg: &Config, hotkeys: &[Hotkey], handle: &mut Handler) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        gnome::run_daemon(cfg, hotkeys, handle)
    }
    #[cfg(windows)]
    {
        windows::run_daemon(cfg, hotkeys, handle)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (cfg, hotkeys, handle);
        anyhow::bail!("window-jump supports GNOME (Linux) and Windows only, for now")
    }
}
