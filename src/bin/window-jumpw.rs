//! `window-jumpw`: the daemon without a console window.
//!
//! On Windows a console program always opens a console window, which is not
//! what a login item should do; this binary uses the GUI subsystem instead and
//! runs `window-jump daemon` (or whatever arguments it is given). Errors have
//! nowhere to print, so they are shown in a message box. On other platforms it
//! is the same as `window-jump daemon`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.len() <= 1 {
        args.truncate(1);
        args.push("daemon".into());
    }
    let code = window_jump::cli::run(args);
    #[cfg(windows)]
    if code != ExitCode::SUCCESS {
        message_box("window-jump stopped. Run `window-jump daemon` in a terminal to see why.");
    }
    code
}

#[cfg(windows)]
fn message_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let wide = |s: &str| {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(text).as_ptr(),
            wide("window-jump").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
