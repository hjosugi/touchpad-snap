//! window-jump: switch between windows of the same app with hotkeys and
//! touchpad swipes, on GNOME (via a small shell extension), KDE Plasma (via a
//! KWin script), Sway/Hyprland/niri and Windows.
//!
//! The platform-neutral core — [`cycle`] (which window to pick),
//! [`gesture`] (when a swipe counts), [`hotkey`] (key syntax) and
//! [`config`] — is pure and unit-tested; [`platform`] holds the backends.

pub mod autostart;
pub mod cli;
pub mod config;
pub mod cycle;
pub mod daemon;
pub mod gesture;
pub mod hotkey;
pub mod model;
pub mod platform;
pub mod session;
