//! What an installation of window-jump left behind.
//!
//! This program was called window-jump before 0.5. Everything that still
//! knows the old name lives here, so an upgrade keeps its settings, and so
//! the day this module is deleted nothing else has to change.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The old name, for messages.
pub const NAME: &str = "window-jump";
/// The old GNOME Shell extension and the bus name it owns while enabled.
#[cfg(target_os = "linux")]
pub const EXTENSION_UUID: &str = "window-jump@hjosugi.github.io";
#[cfg(target_os = "linux")]
pub const BUS_NAME: &str = "org.hjosugi.WindowJump";
/// The old daemon's mutex and its value under `...\CurrentVersion\Run`.
#[cfg(windows)]
pub const DAEMON_MUTEX: &str = "Local\\window-jump-daemon";
#[cfg(windows)]
pub const RUN_VALUE: &str = "window-jump";

/// Where window-jump kept its config file.
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(NAME)
        .join("config.toml")
}

/// Copies window-jump's config file to `new` unless `new` already exists.
/// Returns whether it did; the old file stays where it is.
pub fn import_config(old: &Path, new: &Path) -> Result<bool> {
    if new.exists() || !old.exists() {
        return Ok(false);
    }
    if let Some(dir) = new.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::copy(old, new)
        .with_context(|| format!("copying {} to {}", old.display(), new.display()))?;
    Ok(true)
}

/// window-jump's XDG autostart entry.
#[cfg(not(windows))]
pub fn autostart_entry() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("autostart")
        .join("window-jump.desktop")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_imported_once_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("touchpad-snap-legacy-{}", std::process::id()));
        let old = dir.join("window-jump").join("config.toml");
        let new = dir.join("touchpad-snap").join("config.toml");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();

        // Nothing to import.
        assert!(!import_config(&old, &new).unwrap());
        assert!(!new.exists());

        std::fs::write(&old, "order = \"mru\"\n").unwrap();
        assert!(import_config(&old, &new).unwrap());
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "order = \"mru\"\n");
        assert!(old.exists());

        // A config of our own wins from then on.
        std::fs::write(&new, "order = \"stable\"\n").unwrap();
        std::fs::write(&old, "order = \"position\"\n").unwrap();
        assert!(!import_config(&old, &new).unwrap());
        assert_eq!(
            std::fs::read_to_string(&new).unwrap(),
            "order = \"stable\"\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
