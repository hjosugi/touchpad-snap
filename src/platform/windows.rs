//! Windows 10/11 through plain Win32.
//!
//! Windows are listed the way Alt+Tab lists them (visible, not cloaked, not
//! tool windows, unowned), grouped by executable path. Hotkeys go through
//! `RegisterHotKey`. Touchpad swipes are left to Windows itself: Settings →
//! Bluetooth & devices → Touchpad → Advanced gestures can send any key
//! combination, so map a three-finger swipe to the configured hotkeys.

use std::ffi::c_void;
use std::ptr::null_mut;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, RECT,
};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, CreateMutexW, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumChildWindows, EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW,
    GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, MSG, SW_RESTORE, SetForegroundWindow, ShowWindow, WM_HOTKEY, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

use super::{Backend, Event};
use crate::config::Config;
use crate::hotkey::Hotkey;
use crate::model::{Snapshot, Window};

pub struct Win32;

impl Backend for Win32 {
    fn snapshot(&self) -> Result<Snapshot> {
        let mut handles: Vec<HWND> = Vec::new();
        // SAFETY: the callback only pushes into the Vec passed through lparam,
        // which outlives the call.
        unsafe {
            EnumWindows(Some(collect), &mut handles as *mut Vec<HWND> as LPARAM);
        }
        let foreground = unsafe { GetForegroundWindow() };

        // EnumWindows walks top-level windows in Z order, topmost first,
        // which is the closest thing Win32 has to an MRU list.
        let mut windows = Vec::new();
        for hwnd in handles {
            if !is_alt_tab_window(hwnd) {
                continue;
            }
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            let exe = app_executable(hwnd, pid).unwrap_or_default();
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe { GetWindowRect(hwnd, &mut rect) };
            windows.push(Window {
                id: hwnd as usize as u64,
                title: window_text(hwnd),
                app_id: exe.to_lowercase(),
                app_name: app_name(&exe),
                wm_class: class_name(hwnd),
                pid,
                workspace: 0,
                minimized: unsafe { IsIconic(hwnd) } != 0,
                focused: hwnd == foreground,
                mru: windows.len() as u32,
                x: rect.left,
                y: rect.top,
                width: rect.right - rect.left,
                height: rect.bottom - rect.top,
            });
        }
        Ok(Snapshot {
            active_workspace: 0,
            windows,
        })
    }

    fn activate(&self, id: u64) -> Result<()> {
        let hwnd = id as usize as HWND;
        unsafe {
            if IsWindow(hwnd) == 0 {
                bail!("window {id:#x} is gone");
            }
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd) != 0 && GetForegroundWindow() == hwnd {
                return Ok(());
            }
            // Windows only lets the process that received the last input
            // event change the foreground window. Joining the foreground
            // thread's input queue for the call lifts that restriction.
            let foreground = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(foreground, null_mut());
            let me = GetCurrentThreadId();
            let attached =
                fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            if attached {
                AttachThreadInput(me, fg_thread, 0);
            }
            if GetForegroundWindow() != hwnd {
                bail!("Windows refused to bring window {id:#x} to the foreground");
            }
        }
        Ok(())
    }

    fn describe(&self) -> String {
        "Win32 (EnumWindows / RegisterHotKey)".into()
    }
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: lparam is the &mut Vec<HWND> passed to EnumWindows.
    let handles = unsafe { &mut *(lparam as *mut Vec<HWND>) };
    handles.push(hwnd);
    1
}

/// Roughly the rules Alt+Tab uses.
fn is_alt_tab_window(hwnd: HWND) -> bool {
    unsafe {
        if IsWindowVisible(hwnd) == 0 || GetWindowTextLengthW(hwnd) == 0 {
            return false;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_NOACTIVATE != 0 {
            return false;
        }
        let app_window = ex & WS_EX_APPWINDOW != 0;
        if ex & WS_EX_TOOLWINDOW != 0 && !app_window {
            return false;
        }
        if !GetWindow(hwnd, GW_OWNER).is_null() && !app_window {
            return false;
        }
        // Cloaked: suspended UWP apps and windows on other virtual desktops.
        let mut cloaked: u32 = 0;
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        );
        if hr == 0 && cloaked != 0 {
            return false;
        }
    }
    true
}

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// The executable that owns the window. UWP apps all live in frames owned by
/// ApplicationFrameHost.exe; the real app is the process behind the child
/// CoreWindow, so look there or every UWP app would count as one app.
fn app_executable(hwnd: HWND, pid: u32) -> Option<String> {
    let path = process_path(pid)?;
    if !path.to_lowercase().ends_with("\\applicationframehost.exe") {
        return Some(path);
    }
    let mut found: (u32, u32) = (pid, 0);
    unsafe extern "system" fn child(hwnd: HWND, lparam: LPARAM) -> i32 {
        // SAFETY: lparam is the &mut (frame_pid, found_pid) below.
        let state = unsafe { &mut *(lparam as *mut (u32, u32)) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid != 0 && pid != state.0 {
            state.1 = pid;
            return 0;
        }
        1
    }
    unsafe {
        EnumChildWindows(hwnd, Some(child), &mut found as *mut (u32, u32) as LPARAM);
    }
    if found.1 != 0 {
        process_path(found.1).or(Some(path))
    } else {
        Some(path)
    }
}

fn app_name(exe: &str) -> String {
    std::path::Path::new(exe)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn run_daemon(
    cfg: &Config,
    hotkeys: &[Hotkey],
    handle: &mut dyn FnMut(&dyn Backend, Event),
) -> Result<()> {
    // One daemon per session: a second would fail every RegisterHotKey anyway.
    let name = wide("Local\\window-jump-daemon");
    unsafe {
        let mutex = CreateMutexW(null_mut(), 0, name.as_ptr());
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            bail!("another window-jump daemon is already running");
        }
        // Deliberately leaked: held until the process exits.
    }

    let mut registered = 0;
    for (i, hk) in hotkeys.iter().enumerate() {
        let (mods, vk) = hk.to_windows();
        // Hotkey ids are the binding index + 1 (0 is not a valid id).
        if unsafe { RegisterHotKey(null_mut(), i as i32 + 1, mods, vk) } == 0 {
            eprintln!("window-jump: could not register {hk}: another app already uses it");
        } else {
            registered += 1;
        }
    }
    eprintln!("window-jump: registered {registered} hotkey(s)");
    if !cfg.swipes.is_empty() {
        eprintln!(
            "window-jump: [[swipe]] bindings are GNOME-only. On Windows, map touchpad swipes to the hotkeys in \
             Settings → Bluetooth & devices → Touchpad → Advanced gestures."
        );
    }

    let backend = Win32;
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // Hotkeys registered with a null HWND are posted to this thread's queue.
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        if msg.message == WM_HOTKEY {
            let id = msg.wParam as usize;
            if id >= 1 {
                handle(&backend, Event::Hotkey(id - 1));
            }
        }
    }
    Ok(())
}
