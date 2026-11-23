//! touchpad-snap: bind touchpad gestures and shortcuts to actions, on GNOME
//! (via a small shell extension), KDE Plasma (via a KWin script),
//! Sway/Hyprland/niri and Windows.
//!
//! The platform-neutral core — config, triggers, conditions and window
//! cycling — is the [`touchpad_snap_core`] crate; this crate adds the
//! platform backends and the daemon. The core modules are re-exported here so
//! `touchpad_snap::config` and friends keep working for the daemon, the
//! tests and anything that embedded the old single crate.

pub use touchpad_snap_core::{config, cycle, gesture, hotkey, legacy, model, session, when};

pub mod autostart;
pub mod cli;
pub mod control;
pub mod daemon;
pub mod exec;
pub mod platform;
pub mod systemd;
