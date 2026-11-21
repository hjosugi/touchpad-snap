//! Starting the daemon at login.
//!
//! Linux: an XDG autostart entry (~/.config/autostart/touchpad-snap.desktop),
//! which GNOME runs once the session (and so the shell extension) is up.
//! Windows: a value under HKCU\...\CurrentVersion\Run pointing at
//! touchpad-snapw.exe, the console-less build of the daemon.

#[cfg(not(windows))]
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::legacy;

/// The command that starts the daemon, quoted for the platform.
fn daemon_command() -> Result<String> {
    let exe = std::env::current_exe().context("locating the touchpad-snap executable")?;

    #[cfg(windows)]
    {
        let gui = exe.with_file_name("touchpad-snapw.exe");
        if gui.exists() {
            return Ok(format!("\"{}\"", gui.display()));
        }
        Ok(format!("\"{}\" daemon", exe.display()))
    }
    #[cfg(not(windows))]
    {
        // A Nix store path changes with every update and dies with garbage
        // collection; the profile puts the binary on PATH, so use the name.
        let exe_str = exe.display().to_string();
        if exe_str.starts_with("/nix/store/") {
            return Ok("touchpad-snap daemon".into());
        }
        Ok(format!("\"{}\" daemon", exe_str.replace('"', "\\\"")))
    }
}

#[cfg(not(windows))]
fn desktop_file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("autostart")
        .join("touchpad-snap.desktop")
}

/// Whether the daemon is currently set to start at login. The tray menu uses
/// this for its checkmark.
pub fn enabled() -> bool {
    #[cfg(windows)]
    {
        registry::run_value_exists(registry::VALUE)
    }
    #[cfg(not(windows))]
    {
        desktop_file().exists()
    }
}

pub fn enable() -> Result<String> {
    let command = daemon_command()?;

    #[cfg(windows)]
    {
        registry::set_run_value(&command)?;
        registry::delete_run_value(legacy::RUN_VALUE)?;
        Ok(format!(
            "touchpad-snap will start at login: HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\touchpad-snap = {command}"
        ))
    }
    #[cfg(not(windows))]
    {
        let path = desktop_file();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=touchpad-snap\n\
             Comment=Switch between windows of the same app with hotkeys and touchpad swipes\n\
             Exec={command}\n\
             NoDisplay=true\n\
             X-GNOME-Autostart-enabled=true\n\
             X-GNOME-Autostart-Phase=Applications\n"
        );
        std::fs::write(&path, entry).with_context(|| format!("writing {}", path.display()))?;
        remove(&legacy::autostart_entry())?;
        Ok(format!(
            "touchpad-snap will start at login ({})",
            path.display()
        ))
    }
}

pub fn disable() -> Result<String> {
    #[cfg(windows)]
    {
        registry::delete_run_value(registry::VALUE)?;
        registry::delete_run_value(legacy::RUN_VALUE)?;
        Ok("touchpad-snap will no longer start at login".into())
    }
    #[cfg(not(windows))]
    {
        let path = desktop_file();
        remove(&path)?;
        remove(&legacy::autostart_entry())?;
        Ok(format!("removed {}", path.display()))
    }
}

/// An autostart entry left by the old name (see [`legacy`]), which would
/// start a second daemon or one that is no longer installed: what it is and
/// how to get rid of it. `enable` and `disable` both remove it.
pub fn leftover() -> Option<String> {
    #[cfg(windows)]
    let (found, what) = (
        registry::run_value_exists(legacy::RUN_VALUE),
        format!("HKCU\\{}\\{}", registry::RUN_KEY, legacy::RUN_VALUE),
    );
    #[cfg(not(windows))]
    let (found, what) = {
        let path = legacy::autostart_entry();
        (path.exists(), path.display().to_string())
    };
    found.then(|| {
        format!(
            "{} still starts at login ({what}); replace it with: touchpad-snap autostart enable",
            legacy::NAME
        )
    })
}

/// Removes a file; one that is not there is fine.
#[cfg(not(windows))]
fn remove(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(windows)]
mod registry {
    use anyhow::{Result, bail};
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };

    pub const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    pub const VALUE: &str = "touchpad-snap";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn set_run_value(command: &str) -> Result<()> {
        let data = wide(command);
        let rc = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(VALUE).as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if rc != ERROR_SUCCESS {
            bail!("writing the Run registry value failed (error {rc})");
        }
        Ok(())
    }

    pub fn run_value_exists(value: &str) -> bool {
        let mut data = [0u16; 1024];
        let mut len = (data.len() * 2) as u32;
        let mut kind = 0u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(value).as_ptr(),
                RRF_RT_REG_SZ,
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut len,
            )
        };
        rc == ERROR_SUCCESS || rc == ERROR_MORE_DATA
    }

    pub fn delete_run_value(value: &str) -> Result<()> {
        let rc = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(value).as_ptr(),
            )
        };
        if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
            bail!("removing the Run registry value failed (error {rc})");
        }
        Ok(())
    }
}
