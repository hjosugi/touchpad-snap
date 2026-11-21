//! The Windows notification-area icon and its menu.
//!
//! `touchpad-snapw.exe` has no console, so the tray icon is the only way to see
//! whether the hotkeys were registered and to stop or reconfigure the daemon.
//! The icon belongs to a hidden window on the daemon's thread; its messages
//! are dispatched by the daemon's single `GetMessageW` loop. Menu commands are
//! queued from the window procedure and carried out by that loop, so they
//! never re-enter the event handler.
//!
//! Everything that can be decided without Win32 — the status lines, the
//! tooltip, command mapping — is kept apart and unit-tested on any host.

/// One configured hotkey and whether Windows let us register it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyStatus {
    pub keys: String,
    pub registered: bool,
}

/// A menu command, carried out by the daemon loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Reload,
    OpenConfig,
    ToggleAutostart,
    Quit,
}

const CMD_RELOAD: u32 = 1;
const CMD_OPEN_CONFIG: u32 = 2;
const CMD_AUTOSTART: u32 = 3;
const CMD_QUIT: u32 = 4;

/// Maps a `WM_COMMAND` id to what the daemon should do.
pub fn action_for_command(command: u32) -> Option<Action> {
    match command {
        CMD_RELOAD => Some(Action::Reload),
        CMD_OPEN_CONFIG => Some(Action::OpenConfig),
        CMD_AUTOSTART => Some(Action::ToggleAutostart),
        CMD_QUIT => Some(Action::Quit),
        _ => None,
    }
}

/// How many of the configured hotkeys are actually registered.
pub fn registered_count(statuses: &[HotkeyStatus]) -> usize {
    statuses.iter().filter(|s| s.registered).count()
}

/// The icon's tooltip, e.g. `touchpad-snap (2 hotkeys)`.
pub fn tooltip(statuses: &[HotkeyStatus]) -> String {
    let n = registered_count(statuses);
    let plural = if n == 1 { "" } else { "s" };
    format!("touchpad-snap ({n} hotkey{plural})")
}

/// The disabled menu entries summarising the hotkey situation: a count, then
/// every binding, with `(taken)` after the ones another app already holds.
pub fn status_lines(statuses: &[HotkeyStatus]) -> Vec<String> {
    if statuses.is_empty() {
        return vec!["No hotkeys configured".into()];
    }
    let mut lines = vec![format!(
        "{} of {} hotkey(s) registered",
        registered_count(statuses),
        statuses.len()
    )];
    lines.extend(statuses.iter().map(|s| {
        if s.registered {
            s.keys.clone()
        } else {
            format!("{} (taken)", s.keys)
        }
    }));
    lines
}

/// Whether the "Start at login" item should enable (rather than disable)
/// autostart, given its current state.
pub fn autostart_will_enable(currently_enabled: bool) -> bool {
    !currently_enabled
}

#[cfg(windows)]
mod win32 {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
        NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        GetCursorPos, IDI_APPLICATION, LoadIconW, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING,
        PostMessageW, RegisterClassW, SW_SHOWNORMAL, SetForegroundWindow, TPM_RIGHTBUTTON,
        TrackPopupMenu, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
        WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    use super::{
        Action, CMD_AUTOSTART, CMD_OPEN_CONFIG, CMD_QUIT, CMD_RELOAD, HotkeyStatus,
        action_for_command, status_lines, tooltip,
    };
    use crate::{autostart, config};

    const CLASS_NAME: &str = "touchpad-snap-tray";
    const TRAY_ID: u32 = 1;
    const TRAY_CALLBACK: u32 = WM_APP + 1;

    const RELOAD_LABEL: &str = "Reload config";
    const OPEN_CONFIG_LABEL: &str = "Open config file";
    const AUTOSTART_LABEL: &str = "Start at login";
    const QUIT_LABEL: &str = "Quit";

    #[derive(Default)]
    struct TrayState {
        statuses: Vec<HotkeyStatus>,
        actions: VecDeque<Action>,
    }

    thread_local! {
        /// The status the next menu draws, and commands waiting for the loop.
        static STATE: RefCell<TrayState> = RefCell::new(TrayState::default());
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn set_tip(nid: &mut NOTIFYICONDATAW, text: &str) {
        let mut tip = [0u16; 128];
        for (slot, unit) in tip[..127].iter_mut().zip(text.encode_utf16()) {
            *slot = unit;
        }
        nid.szTip = tip;
    }

    fn icon_data(hwnd: HWND, flags: u32) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = flags;
        nid.uCallbackMessage = TRAY_CALLBACK;
        nid.hIcon = unsafe { LoadIconW(null_mut(), IDI_APPLICATION) };
        nid
    }

    pub struct Tray {
        hwnd: HWND,
        removed: bool,
    }

    impl Tray {
        /// Adds the icon, or returns `None` if Windows refuses; the daemon
        /// then keeps running without one (the hotkeys are what matter).
        pub fn new(statuses: &[HotkeyStatus]) -> Option<Self> {
            STATE.with(|s| s.borrow_mut().statuses = statuses.to_vec());
            let class = wide(CLASS_NAME);
            let title = wide("touchpad-snap");
            unsafe {
                let instance = GetModuleHandleW(null_mut());
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: instance,
                    lpszClassName: class.as_ptr(),
                    // SAFETY: every other field may be zero.
                    ..std::mem::zeroed()
                };
                // Fails harmlessly if the class already exists.
                RegisterClassW(&wc);
                let hwnd = CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    null_mut(),
                    null_mut(),
                    instance,
                    null_mut(),
                );
                if hwnd.is_null() {
                    return None;
                }
                let mut nid = icon_data(hwnd, NIF_ICON | NIF_MESSAGE | NIF_TIP);
                set_tip(&mut nid, &tooltip(statuses));
                if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                    eprintln!(
                        "touchpad-snap: could not add the notification-area icon; continuing without it"
                    );
                    DestroyWindow(hwnd);
                    return None;
                }
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &nid);
                Some(Self {
                    hwnd,
                    removed: false,
                })
            }
        }

        /// Updates the tooltip and what the next menu shows.
        pub fn set_status(&self, statuses: &[HotkeyStatus]) {
            STATE.with(|s| s.borrow_mut().statuses = statuses.to_vec());
            let mut nid = icon_data(self.hwnd, NIF_TIP);
            set_tip(&mut nid, &tooltip(statuses));
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
        }

        /// Commands chosen in the menu since the last call.
        pub fn take_actions(&self) -> Vec<Action> {
            STATE.with(|s| s.borrow_mut().actions.drain(..).collect())
        }

        /// Removes the icon. Idempotent, so quitting and dropping is fine.
        pub fn remove(&mut self) {
            if self.removed {
                return;
            }
            let nid = icon_data(self.hwnd, 0);
            unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
            self.removed = true;
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            self.remove();
            unsafe { DestroyWindow(self.hwnd) };
        }
    }

    /// Opens `path` with its default application, writing the commented
    /// template first if it does not exist yet.
    pub fn open_config_file(path: &Path) {
        if let Err(e) = config::ensure(path) {
            eprintln!("touchpad-snap: could not create {}: {e:#}", path.display());
            return;
        }
        let operation = wide("open");
        let file = wide(&path.to_string_lossy());
        let code = unsafe {
            ShellExecuteW(
                null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                null(),
                null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if code <= 32 {
            eprintln!(
                "touchpad-snap: could not open {} (ShellExecuteW returned {code})",
                path.display()
            );
        }
    }

    /// Enables or disables starting at login, depending on the current state.
    pub fn toggle_autostart() {
        let result = if super::autostart_will_enable(autostart::enabled()) {
            autostart::enable()
        } else {
            autostart::disable()
        };
        match result {
            Ok(message) => eprintln!("touchpad-snap: {message}"),
            Err(e) => eprintln!("touchpad-snap: {e:#}"),
        }
    }

    unsafe fn show_menu(hwnd: HWND) {
        let statuses = STATE.with(|s| s.borrow().statuses.clone());
        // SAFETY: creating and destroying our own popup menu.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        for line in status_lines(&statuses) {
            let text = wide(&line);
            unsafe { AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, text.as_ptr()) };
        }
        unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, null()) };

        let reload = wide(RELOAD_LABEL);
        unsafe { AppendMenuW(menu, MF_STRING, CMD_RELOAD as usize, reload.as_ptr()) };
        let open = wide(OPEN_CONFIG_LABEL);
        unsafe { AppendMenuW(menu, MF_STRING, CMD_OPEN_CONFIG as usize, open.as_ptr()) };
        let autostart_label = wide(AUTOSTART_LABEL);
        let mut autostart_flags = MF_STRING;
        if autostart::enabled() {
            autostart_flags |= MF_CHECKED;
        }
        unsafe {
            AppendMenuW(
                menu,
                autostart_flags,
                CMD_AUTOSTART as usize,
                autostart_label.as_ptr(),
            )
        };
        unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, null()) };
        let quit = wide(QUIT_LABEL);
        unsafe { AppendMenuW(menu, MF_STRING, CMD_QUIT as usize, quit.as_ptr()) };

        let mut point = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut point);
            // Without this the menu does not close when clicking elsewhere.
            SetForegroundWindow(hwnd);
            TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, 0, hwnd, null());
            DestroyMenu(menu);
            PostMessageW(hwnd, WM_NULL, 0, 0);
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            TRAY_CALLBACK => {
                // With NOTIFYICON_VERSION_4 the event is the low word; older
                // versions put the mouse message there as well.
                let event = (lparam as u32) & 0xFFFF;
                if matches!(
                    event,
                    WM_RBUTTONUP | WM_LBUTTONUP | WM_CONTEXTMENU | NIN_SELECT
                ) {
                    // SAFETY: our own hidden window.
                    unsafe { show_menu(hwnd) };
                }
                0
            }
            WM_COMMAND => {
                let command = (wparam & 0xFFFF) as u32;
                if let Some(action) = action_for_command(command) {
                    STATE.with(|s| s.borrow_mut().actions.push_back(action));
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }
}

#[cfg(windows)]
pub use win32::{Tray, open_config_file, toggle_autostart};

#[cfg(test)]
mod tests {
    use super::*;

    fn status(keys: &str, registered: bool) -> HotkeyStatus {
        HotkeyStatus {
            keys: keys.into(),
            registered,
        }
    }

    #[test]
    fn counts_and_tooltip() {
        let statuses = [
            status("Super+Alt+N", true),
            status("Super+Alt+P", false),
            status("Ctrl+Alt+J", true),
        ];
        assert_eq!(registered_count(&statuses), 2);
        assert_eq!(tooltip(&statuses), "touchpad-snap (2 hotkeys)");
        assert_eq!(
            tooltip(&[status("Super+Alt+N", true)]),
            "touchpad-snap (1 hotkey)"
        );
        assert_eq!(tooltip(&[]), "touchpad-snap (0 hotkeys)");
    }

    #[test]
    fn status_lines_name_the_taken_hotkeys() {
        let statuses = [status("Super+Alt+N", true), status("Super+Alt+P", false)];
        assert_eq!(
            status_lines(&statuses),
            vec![
                "1 of 2 hotkey(s) registered",
                "Super+Alt+N",
                "Super+Alt+P (taken)",
            ]
        );
        assert_eq!(status_lines(&[]), vec!["No hotkeys configured"]);
    }

    #[test]
    fn menu_commands_map_to_actions() {
        assert_eq!(action_for_command(1), Some(Action::Reload));
        assert_eq!(action_for_command(2), Some(Action::OpenConfig));
        assert_eq!(action_for_command(3), Some(Action::ToggleAutostart));
        assert_eq!(action_for_command(4), Some(Action::Quit));
        assert_eq!(action_for_command(0), None);
        assert_eq!(action_for_command(99), None);
    }

    #[test]
    fn autostart_toggle_flips_the_current_state() {
        assert!(autostart_will_enable(false));
        assert!(!autostart_will_enable(true));
    }
}
