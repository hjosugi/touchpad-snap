//! Starting the daemon at login.
//!
//! Linux: an XDG autostart entry (~/.config/autostart/window-jump.desktop),
//! which GNOME runs once the session (and so the shell extension) is up.
//! Windows: a value under HKCU\...\CurrentVersion\Run pointing at
//! window-jumpw.exe, the console-less build of the daemon.

#[cfg(not(windows))]
use std::path::PathBuf;

use anyhow::{Context, Result};

/// The command that starts the daemon, quoted for the platform.
fn daemon_command() -> Result<String> {
    let exe = std::env::current_exe().context("locating the window-jump executable")?;

    #[cfg(windows)]
    {
        let gui = exe.with_file_name("window-jumpw.exe");
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
            return Ok("window-jump daemon".into());
        }
        Ok(format!("\"{}\" daemon", exe_str.replace('"', "\\\"")))
    }
}

#[cfg(not(windows))]
fn desktop_file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("autostart")
        .join("window-jump.desktop")
}

/// Whether the daemon is currently set to start at login. The tray menu uses
/// this for its checkmark.
pub fn enabled() -> bool {
    #[cfg(windows)]
    {
        registry::run_value_exists()
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
        Ok(format!(
            "window-jump will start at login: HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\window-jump = {command}"
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
             Name=window-jump\n\
             Comment=Switch between windows of the same app with hotkeys and touchpad swipes\n\
             Exec={command}\n\
             NoDisplay=true\n\
             X-GNOME-Autostart-enabled=true\n\
             X-GNOME-Autostart-Phase=Applications\n"
        );
        std::fs::write(&path, entry).with_context(|| format!("writing {}", path.display()))?;
        Ok(format!(
            "window-jump will start at login ({})",
            path.display()
        ))
    }
}

pub fn disable() -> Result<String> {
    #[cfg(windows)]
    {
        registry::delete_run_value()?;
        Ok("window-jump will no longer start at login".into())
    }
    #[cfg(not(windows))]
    {
        let path = desktop_file();
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("removing {}", path.display()))
            }
            _ => Ok(format!("removed {}", path.display())),
        }
    }
}

#[cfg(windows)]
mod registry {
    use anyhow::{Result, bail};
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const VALUE: &str = "window-jump";

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

    pub fn run_value_exists() -> bool {
        let mut data = [0u16; 1024];
        let mut len = (data.len() * 2) as u32;
        let mut kind = 0u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(VALUE).as_ptr(),
                RRF_RT_REG_SZ,
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut len,
            )
        };
        rc == ERROR_SUCCESS || rc == ERROR_MORE_DATA
    }

    pub fn delete_run_value() -> Result<()> {
        let rc = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(VALUE).as_ptr(),
            )
        };
        if rc != ERROR_SUCCESS && rc != ERROR_FILE_NOT_FOUND {
            bail!("removing the Run registry value failed (error {rc})");
        }
        Ok(())
    }
}
