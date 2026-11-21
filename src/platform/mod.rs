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

pub trait Backend {
    fn snapshot(&self) -> Result<Snapshot>;
    fn activate(&self, id: u64) -> Result<()>;
    /// One line for `window-jump doctor`.
    fn describe(&self) -> String;
}

/// Something the daemon reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The hotkey at this index of `Config::hotkeys` was pressed.
    Hotkey(usize),
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
        Ok(Box::new(windows::Win32))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        anyhow::bail!("window-jump supports GNOME (Linux) and Windows only, for now")
    }
}

/// Registers the configured hotkeys and swipes, then calls `handle` for every
/// event until the process is stopped.
pub fn run_daemon(
    cfg: &Config,
    hotkeys: &[Hotkey],
    handle: &mut dyn FnMut(&dyn Backend, Event),
) -> Result<()> {
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
