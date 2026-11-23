//! The platform-neutral core of touchpad-snap.
//!
//! Config model and loader ([`config`]), the triggers ([`gesture`],
//! [`hotkey`]), [`when`] conditions, the window model ([`model`]) and the
//! cycling logic ([`cycle`], [`session`]). No platform backends and no I/O
//! beyond the config and state files.

pub mod config;
pub mod cycle;
pub mod gesture;
pub mod hotkey;
pub mod legacy;
pub mod model;
pub mod session;
pub mod when;
