//! Windows 10/11 through plain Win32.
//!
//! Windows are listed the way Alt+Tab lists them (visible, not tool windows,
//! unowned, not a suspended UWP frame), grouped by executable path, including
//! windows on other virtual desktops (see [`workspace_for_window`]). Hotkeys
//! go through `RegisterHotKey`. Precision Touchpad swipes arrive as `WM_INPUT`
//! from raw input (see windows_touchpad.rs), so the same `[[swipe]]` bindings
//! as on GNOME work here too; Windows' own three/four-finger gestures have to
//! be turned off in Settings, since raw input cannot intercept them.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::Once;
use std::time::Duration;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, RECT, RPC_E_CHANGED_MODE,
};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, CreateMutexW, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, RegisterHotKey,
    SendInput, UnregisterHotKey, VIRTUAL_KEY, VK_CONTROL, VK_LEFT, VK_LWIN, VK_MENU, VK_RIGHT,
    VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, DispatchMessageW, EnumChildWindows, EnumWindows, GW_OWNER, GWL_EXSTYLE,
    GetClassNameW, GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, KillTimer, MSG, PostQuitMessage, SW_RESTORE, SetForegroundWindow, SetTimer,
    ShowWindow, TranslateMessage, WM_HOTKEY, WM_TIMER, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};
use windows_sys::core::{BOOL, GUID, HRESULT};

use super::windows_hud::HudWindow;
use super::windows_touchpad::{Sample, Source};
use super::windows_tray::{self, Action, HotkeyStatus, Tray};
use super::{Backend, Event};
use crate::config::{Claim, Config};
use crate::gesture::Phase;
use crate::hotkey::{Hotkey, Mods};
use crate::model::{Snapshot, Window};

pub struct Win32 {
    /// The overlay; only the daemon has one (see windows_hud.rs).
    hud: Option<HudWindow>,
    /// `None` when Windows has no virtual-desktop service: windows on other
    /// desktops then stay hidden, as before.
    desktops: Option<VirtualDesktops>,
}

impl Win32 {
    pub fn new() -> Self {
        let desktops = VirtualDesktops::connect();
        if desktops.is_none() {
            static WARNED: Once = Once::new();
            WARNED.call_once(|| {
                eprintln!(
                    "touchpad-snap: virtual-desktop support is unavailable; windows on other desktops stay hidden"
                );
            });
        }
        Self {
            hud: None,
            desktops,
        }
    }
}

impl Default for Win32 {
    fn default() -> Self {
        Self::new()
    }
}

/// How far to walk virtual desktops looking for a window before giving up.
const MAX_DESKTOP_STEPS: usize = 8;
const DESKTOP_STEP_DELAY: Duration = Duration::from_millis(160);

impl Win32 {
    /// Best effort: there is no documented API to switch desktop, so send the
    /// shortcut Windows itself uses and check after each step.
    fn switch_desktop_to(&self, hwnd: HWND) -> Result<()> {
        let Some(desktops) = &self.desktops else {
            bail!("virtual-desktop support is unavailable");
        };
        let origin = current_desktop_id(desktops);
        if sweep_to(hwnd, desktops, VK_RIGHT) {
            return Ok(());
        }
        rewind(desktops, origin, VK_LEFT);
        if sweep_to(hwnd, desktops, VK_LEFT) {
            return Ok(());
        }
        rewind(desktops, origin, VK_RIGHT);
        bail!(
            "window {:#x} is on another virtual desktop that Win+Ctrl+arrow could not reach",
            hwnd as usize
        )
    }
}

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
        let mut desktops = DesktopMap::default();
        for hwnd in handles {
            if !is_alt_tab_shape(hwnd) {
                continue;
            }
            let cloaked = is_cloaked(hwnd);
            let on_current = self.desktops.as_ref().and_then(|d| d.is_on_current(hwnd));
            let desktop = match on_current {
                Some(false) => self.desktops.as_ref().and_then(|d| d.desktop_id(hwnd)),
                _ => None,
            };
            let Some(workspace) =
                workspace_for_window(true, cloaked, on_current, desktop, &mut desktops)
            else {
                continue;
            };
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
                workspace,
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
            // The current desktop is always 0; other desktops get positive ids
            // so `WorkspaceScope::Current` can keep skipping them.
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
        }
        if bring_to_foreground(hwnd) {
            return Ok(());
        }
        // SetForegroundWindow alone does not move to a window's virtual
        // desktop. If the target lives on another one, switch there with the
        // shortcut users have (Win+Ctrl+arrow; there is no documented API to
        // switch desktops) and try focusing it again.
        if self
            .desktops
            .as_ref()
            .is_some_and(|d| d.is_on_current(hwnd) == Some(false))
        {
            self.switch_desktop_to(hwnd)?;
            if bring_to_foreground(hwnd) {
                return Ok(());
            }
        }
        bail!("Windows refused to bring window {id:#x} to the foreground")
    }

    fn show_hud(&self, id: u64, index: usize, total: usize, duration_ms: u64) -> Result<()> {
        let Some(hud) = &self.hud else {
            return Ok(());
        };
        let hwnd = id as usize as HWND;
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let name = app_name(&app_executable(hwnd, pid).unwrap_or_default());
        let detail = if total > 0 {
            format!("{name} · {index} / {total}")
        } else {
            name
        };
        // The overlay was created on the daemon's original desktop; move it
        // to the window's desktop so it is visible after a cross-desktop jump.
        if let Some(desktops) = &self.desktops
            && let Some(desktop) = desktops.desktop_id(hwnd)
        {
            desktops.move_to_desktop(hud.window(), &desktop);
        }
        hud.show(
            hwnd,
            &window_text(hwnd),
            &detail,
            duration_ms.min(u32::MAX as u64) as u32,
        );
        Ok(())
    }

    fn describe(&self) -> String {
        "Win32 (EnumWindows / RegisterHotKey / raw touchpad input)".into()
    }
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: lparam is the &mut Vec<HWND> passed to EnumWindows.
    let handles = unsafe { &mut *(lparam as *mut Vec<HWND>) };
    handles.push(hwnd);
    1
}

/// Roughly the rules Alt+Tab uses, apart from virtual desktops: those need
/// DWM cloaking and `IVirtualDesktopManager` (see [`workspace_for_window`]).
fn is_alt_tab_shape(hwnd: HWND) -> bool {
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
    }
    true
}

/// Cloaking hides suspended UWP frames and every window on another virtual
/// desktop. Only `IVirtualDesktopManager` can tell the two apart.
fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    hr == 0 && cloaked != 0
}

/// The reported workspace of a window, or `None` when it is not a candidate.
///
/// `on_current` is `IsWindowOnCurrentVirtualDesktop`'s answer (`None` when the
/// virtual-desktop service is unavailable) and `desktop` its desktop id. A
/// cloaked window on the current desktop is a suspended UWP frame and stays
/// hidden; a cloaked one elsewhere is a normal window on another desktop.
fn workspace_for_window(
    candidate: bool,
    cloaked: bool,
    on_current: Option<bool>,
    desktop: Option<GUID>,
    desktops: &mut DesktopMap,
) -> Option<i32> {
    if !candidate {
        return None;
    }
    match (cloaked, on_current) {
        (true, Some(false)) | (false, Some(false)) => Some(desktops.index(desktop)),
        (true, _) => None,
        (false, _) => Some(0),
    }
}

/// Gives every desktop other than the current one a small positive number.
/// The exact number does not matter: `WorkspaceScope::Current` only compares
/// against `active_workspace` (0), while `All` ignores it.
#[derive(Default)]
struct DesktopMap {
    others: Vec<Option<GUID>>,
}

impl DesktopMap {
    fn index(&mut self, desktop: Option<GUID>) -> i32 {
        let desktop = desktop.filter(|g| !is_null_guid(g));
        if let Some(i) = self.others.iter().position(|&d| same_desktop(d, desktop)) {
            return i as i32 + 1;
        }
        self.others.push(desktop);
        self.others.len() as i32
    }
}

fn same_desktop(a: Option<GUID>, b: Option<GUID>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
        }
        (None, None) => true,
        _ => false,
    }
}

fn is_null_guid(g: &GUID) -> bool {
    g.data1 == 0 && g.data2 == 0 && g.data3 == 0 && g.data4 == [0; 8]
}

// IVirtualDesktopManager is not generated by windows-sys, so the documented
// interface (and only its first methods) is declared here.
const CLSID_VIRTUAL_DESKTOP_MANAGER: GUID = GUID::from_u128(0xaa509086_5ca9_4c25_8f95_589d3c07b48a);
const IID_IVIRTUAL_DESKTOP_MANAGER: GUID = GUID::from_u128(0xa5cd92ff_29be_454c_8d04_d82879fb3f1b);

#[repr(C)]
struct IVirtualDesktopManagerVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    is_window_on_current_virtual_desktop:
        unsafe extern "system" fn(*mut c_void, HWND, *mut BOOL) -> HRESULT,
    get_window_desktop_id: unsafe extern "system" fn(*mut c_void, HWND, *mut GUID) -> HRESULT,
    move_window_to_desktop: unsafe extern "system" fn(*mut c_void, HWND, *const GUID) -> HRESULT,
}

#[repr(C)]
struct IVirtualDesktopManager {
    vtable: *const IVirtualDesktopManagerVtbl,
}

struct VirtualDesktops {
    manager: *mut IVirtualDesktopManager,
    _apartment: ComApartment,
}

impl VirtualDesktops {
    fn connect() -> Option<Self> {
        let apartment = ComApartment::init()?;
        let mut manager: *mut c_void = null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_VIRTUAL_DESKTOP_MANAGER,
                null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IVIRTUAL_DESKTOP_MANAGER,
                &mut manager,
            )
        };
        if hr < 0 || manager.is_null() {
            return None;
        }
        Some(Self {
            manager: manager.cast(),
            _apartment: apartment,
        })
    }

    fn is_on_current(&self, hwnd: HWND) -> Option<bool> {
        let mut on: BOOL = 0;
        let vtable = unsafe { (*self.manager).vtable };
        let hr = unsafe {
            ((*vtable).is_window_on_current_virtual_desktop)(
                self.manager as *mut c_void,
                hwnd,
                &mut on,
            )
        };
        (hr >= 0).then_some(on != 0)
    }

    fn desktop_id(&self, hwnd: HWND) -> Option<GUID> {
        let mut id = GUID::default();
        let vtable = unsafe { (*self.manager).vtable };
        let hr = unsafe {
            ((*vtable).get_window_desktop_id)(self.manager as *mut c_void, hwnd, &mut id)
        };
        (hr >= 0).then_some(id)
    }

    /// Moves `hwnd` to `desktop`. Used for the overlay, which is created on
    /// whichever desktop the daemon started on.
    fn move_to_desktop(&self, hwnd: HWND, desktop: &GUID) -> bool {
        let vtable = unsafe { (*self.manager).vtable };
        let hr = unsafe {
            ((*vtable).move_window_to_desktop)(self.manager as *mut c_void, hwnd, desktop)
        };
        hr >= 0
    }
}

impl Drop for VirtualDesktops {
    fn drop(&mut self) {
        unsafe {
            let vtable = (*self.manager).vtable;
            ((*vtable).release)(self.manager as *mut c_void);
        }
    }
}

/// One COM apartment per backend. `RPC_E_CHANGED_MODE` means someone else
/// already initialised the thread differently; object creation still works,
/// but that call must not be balanced with `CoUninitialize`.
struct ComApartment {
    uninitialize: bool,
}

impl ComApartment {
    fn init() -> Option<Self> {
        let hr = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED as u32) };
        match hr {
            0 | 1 => Some(Self { uninitialize: true }),
            RPC_E_CHANGED_MODE => Some(Self {
                uninitialize: false,
            }),
            _ => None,
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

fn current_desktop_id(desktops: &VirtualDesktops) -> Option<GUID> {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return None;
    }
    desktops.desktop_id(foreground)
}

fn sweep_to(hwnd: HWND, desktops: &VirtualDesktops, key: VIRTUAL_KEY) -> bool {
    for _ in 0..MAX_DESKTOP_STEPS {
        if !send_win_ctrl(key) {
            return false;
        }
        std::thread::sleep(DESKTOP_STEP_DELAY);
        if desktops.is_on_current(hwnd) == Some(true) {
            return true;
        }
    }
    false
}

/// Walk back towards the desktop the jump started on after a failed sweep.
fn rewind(desktops: &VirtualDesktops, origin: Option<GUID>, key: VIRTUAL_KEY) {
    for _ in 0..MAX_DESKTOP_STEPS {
        if origin.is_some() && same_desktop(current_desktop_id(desktops), origin) {
            return;
        }
        if !send_win_ctrl(key) {
            return;
        }
        std::thread::sleep(DESKTOP_STEP_DELAY);
    }
}

fn send_win_ctrl(key: VIRTUAL_KEY) -> bool {
    let inputs = win_ctrl_chord(key);
    let accepted = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    accepted == inputs.len() as u32
}

fn win_ctrl_chord(key: VIRTUAL_KEY) -> [INPUT; 6] {
    let keys = [VK_LWIN, VK_CONTROL, key];
    let mut inputs = [INPUT::default(); 6];
    for (i, key) in keys.iter().enumerate() {
        inputs[i] = key_input(*key, false);
        inputs[2 * keys.len() - 1 - i] = key_input(*key, true);
    }
    inputs
}

fn key_input(key: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn bring_to_foreground(hwnd: HWND) -> bool {
    unsafe {
        if SetForegroundWindow(hwnd) != 0 && GetForegroundWindow() == hwnd {
            return true;
        }
        // Windows only lets the process that received the last input event
        // change the foreground window. Joining the foreground thread's input
        // queue for the call lifts that restriction.
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
        GetForegroundWindow() == hwnd
    }
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
    config_path: &Path,
    verbose: bool,
    handle: &mut super::Handler,
) -> Result<()> {
    let _ = verbose;
    // One daemon per session: a second would fail every RegisterHotKey anyway.
    let name = wide("Local\\touchpad-snap-daemon");
    unsafe {
        let mutex = CreateMutexW(null_mut(), 0, name.as_ptr());
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            bail!("another touchpad-snap daemon is already running");
        }
        // Deliberately leaked: held until the process exits.
    }

    let mut cfg = cfg.clone();
    let mut hotkeys = hotkeys.to_vec();
    let (mut registered, mut statuses) = register_hotkeys(&hotkeys, 0);

    // Thread timers: the tick, and while a hotkey's modifiers are held, the
    // check for their release. Both post WM_TIMER to this thread with the
    // id SetTimer returned.
    let tick = unsafe { SetTimer(null_mut(), 0, super::TICK.as_millis() as u32, None) };
    let mut release = Release::default();

    let mut backend = Win32::new();
    backend.hud = HudWindow::new();
    // Raw input needs a window to target; the touchpad module owns the hidden
    // message-only one. Without a Precision Touchpad it simply never fires.
    let touchpad = Source::new(!cfg.swipes.is_empty());
    let mut swipe_claim: Option<Claim> = None;
    // A tray that Windows refuses is not fatal: hotkeys must still work.
    let mut tray = Tray::new(&statuses);
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // Hotkeys registered with a null HWND, and the timers, are posted to
    // this thread's queue with a null HWND; the overlay's, touchpad's and
    // tray's own messages carry their windows and are dispatched to them.
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        let mut changed = None;
        if !msg.hwnd.is_null() {
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // The hidden window's WndProc parsed any WM_INPUT into samples.
            if let Some(touchpad) = &touchpad {
                for sample in touchpad.drain() {
                    if let Some(event) = swipe_event(&cfg, &mut swipe_claim, sample) {
                        changed = handle(&backend, event);
                    }
                }
            }
        } else {
            changed = match msg.message {
                WM_HOTKEY => match msg.wParam as usize {
                    0 => None,
                    id => {
                        let index = id - 1;
                        let mods = hotkeys.get(index).map(|h| h.mods).unwrap_or_default();
                        // Without modifiers there is nothing to hold.
                        let tracked = !mods.is_empty();
                        let changed = handle(&backend, Event::Hotkey { index, tracked });
                        if tracked {
                            release.watch(mods);
                        }
                        changed
                    }
                },
                WM_TIMER if msg.wParam == tick => handle(&backend, Event::Tick),
                WM_TIMER if release.is_timer(msg.wParam) => {
                    if release.poll() {
                        handle(&backend, Event::Released)
                    } else {
                        None
                    }
                }
                _ => None,
            };
        }
        if let Some(new) = changed {
            match new.validate() {
                Ok(parsed) => {
                    (registered, statuses) = register_hotkeys(&parsed, registered);
                    hotkeys = parsed;
                    cfg = new;
                    // A reload can change the claims; the next Begin picks
                    // them up.
                    swipe_claim = None;
                    if let Some(t) = tray.as_ref() {
                        t.set_status(&statuses);
                    }
                }
                Err(e) => eprintln!("touchpad-snap: ignoring an invalid config: {e:#}"),
            }
        }

        let Some(t) = tray.as_mut() else {
            continue;
        };
        let mut quit = false;
        for action in t.take_actions() {
            match action {
                Action::Reload => {
                    // Same path as the two-second file watcher, forced.
                    if let Some(new) = handle(&backend, Event::Reload) {
                        match new.validate() {
                            Ok(parsed) => {
                                let (n, s) = register_hotkeys(&parsed, registered);
                                registered = n;
                                statuses = s;
                                hotkeys = parsed;
                                cfg = new;
                                swipe_claim = None;
                                t.set_status(&statuses);
                            }
                            Err(e) => {
                                eprintln!("touchpad-snap: ignoring an invalid config: {e:#}")
                            }
                        }
                    }
                }
                Action::OpenConfig => windows_tray::open_config_file(config_path),
                Action::ToggleAutostart => windows_tray::toggle_autostart(),
                Action::Quit => {
                    t.remove();
                    unsafe { PostQuitMessage(0) };
                    quit = true;
                    break;
                }
            }
        }
        if quit {
            break;
        }
    }
    Ok(())
}

/// How often to look at the modifiers while a hotkey's are held.
const RELEASE_POLL_MS: u32 = 25;

/// Watches for the release of a hotkey's modifiers, like Alt+Tab waits for
/// Alt. Polled with `GetAsyncKeyState`, and only while they are held: no
/// keyboard hook.
#[derive(Default)]
struct Release {
    timer: usize,
    mods: Mods,
}

impl Release {
    fn watch(&mut self, mods: Mods) {
        self.mods = mods;
        if self.timer == 0 {
            self.timer = unsafe { SetTimer(null_mut(), 0, RELEASE_POLL_MS, None) };
        }
    }

    fn is_timer(&self, id: usize) -> bool {
        self.timer != 0 && id == self.timer
    }

    /// True once none of the modifiers is down any more (and the watch is over).
    fn poll(&mut self) -> bool {
        if modifiers_down(self.mods) {
            return false;
        }
        unsafe { KillTimer(null_mut(), self.timer) };
        self.timer = 0;
        true
    }
}

fn modifiers_down(mods: Mods) -> bool {
    let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } < 0;
    (mods.contains(Mods::SHIFT) && down(VK_SHIFT))
        || (mods.contains(Mods::CTRL) && down(VK_CONTROL))
        || (mods.contains(Mods::ALT) && down(VK_MENU))
        || (mods.contains(Mods::SUPER) && (down(VK_LWIN) || down(VK_RWIN)))
}

/// Turns a parsed touchpad sample into a swipe event, but only for a
/// (finger count, modifiers) combination the config claims. Modifiers are
/// sampled at Begin, as on GNOME.
fn swipe_event(cfg: &Config, active: &mut Option<Claim>, sample: Sample) -> Option<Event> {
    match sample.phase {
        Phase::Begin => {
            let claim = Claim {
                fingers: sample.fingers,
                mods: held_mods(),
            };
            if cfg.claims().contains(&claim) {
                *active = Some(claim);
                Some(Event::Swipe {
                    phase: Phase::Begin,
                    claim,
                    dx: 0.0,
                    dy: 0.0,
                })
            } else {
                *active = None;
                None
            }
        }
        Phase::Update => {
            let claim = (*active)?;
            Some(Event::Swipe {
                phase: Phase::Update,
                claim,
                dx: sample.dx,
                dy: sample.dy,
            })
        }
        Phase::End | Phase::Cancel => {
            let claim = active.take()?;
            Some(Event::Swipe {
                phase: sample.phase,
                claim,
                dx: 0.0,
                dy: 0.0,
            })
        }
    }
}

/// Modifiers held right now, in the same bits the GNOME extension sends.
fn held_mods() -> Mods {
    let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) < 0 };
    let mut mods = Mods::default();
    if down(VK_SHIFT) {
        mods |= Mods::SHIFT;
    }
    if down(VK_CONTROL) {
        mods |= Mods::CTRL;
    }
    if down(VK_MENU) {
        mods |= Mods::ALT;
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        mods |= Mods::SUPER;
    }
    mods
}

/// Drops the `previous` hotkey ids and registers `hotkeys` as ids 1..=n (the
/// binding index + 1; 0 is not a valid id). Returns n, the ids now in use,
/// and whether each binding made it (for the tray's status).
fn register_hotkeys(hotkeys: &[Hotkey], previous: usize) -> (usize, Vec<HotkeyStatus>) {
    for id in 1..=previous {
        unsafe { UnregisterHotKey(null_mut(), id as i32) };
    }
    let mut statuses = Vec::with_capacity(hotkeys.len());
    for (i, hk) in hotkeys.iter().enumerate() {
        let (mods, vk) = hk.to_windows();
        let registered = unsafe { RegisterHotKey(null_mut(), i as i32 + 1, mods, vk) } != 0;
        if !registered {
            eprintln!("touchpad-snap: could not register {hk}: another app already uses it");
        }
        statuses.push(HotkeyStatus {
            keys: hk.to_string(),
            registered,
        });
    }
    let ok = windows_tray::registered_count(&statuses);
    eprintln!("touchpad-snap: registered {ok} hotkey(s)");
    (hotkeys.len(), statuses)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn guid(n: u128) -> GUID {
        GUID::from_u128(n)
    }

    #[test]
    fn current_desktop_is_zero() {
        let mut map = DesktopMap::default();
        assert_eq!(
            workspace_for_window(true, false, Some(true), None, &mut map),
            Some(0)
        );
        assert_eq!(
            workspace_for_window(true, false, Some(true), Some(guid(1)), &mut map),
            Some(0)
        );
    }

    #[test]
    fn other_desktops_get_distinct_stable_ids() {
        let mut map = DesktopMap::default();
        let a = guid(10);
        let b = guid(20);
        assert_eq!(
            workspace_for_window(true, true, Some(false), Some(a), &mut map),
            Some(1)
        );
        assert_eq!(
            workspace_for_window(true, true, Some(false), Some(b), &mut map),
            Some(2)
        );
        assert_eq!(
            workspace_for_window(true, true, Some(false), Some(a), &mut map),
            Some(1)
        );
    }

    #[test]
    fn cloaked_windows_on_the_current_desktop_stay_hidden() {
        let mut map = DesktopMap::default();
        assert_eq!(
            workspace_for_window(true, true, Some(true), Some(guid(1)), &mut map),
            None
        );
        assert_eq!(
            workspace_for_window(true, true, None, Some(guid(1)), &mut map),
            None
        );
    }

    #[test]
    fn without_virtual_desktops_todays_behaviour_stays() {
        let mut map = DesktopMap::default();
        assert_eq!(
            workspace_for_window(true, false, None, None, &mut map),
            Some(0)
        );
        assert_eq!(
            workspace_for_window(true, false, Some(true), None, &mut map),
            Some(0)
        );
        assert_eq!(workspace_for_window(true, true, None, None, &mut map), None);
    }

    #[test]
    fn non_candidates_are_not_counted() {
        let mut map = DesktopMap::default();
        assert_eq!(
            workspace_for_window(false, false, Some(false), Some(guid(1)), &mut map),
            None
        );
    }

    #[test]
    fn unknown_desktops_share_one_other_bucket() {
        let mut map = DesktopMap::default();
        assert_eq!(
            workspace_for_window(true, true, Some(false), None, &mut map),
            Some(1)
        );
        assert_eq!(
            workspace_for_window(true, true, Some(false), Some(GUID::default()), &mut map),
            Some(1)
        );
    }

    #[test]
    fn win_ctrl_chord_presses_and_releases_in_order() {
        let inputs = win_ctrl_chord(VK_LEFT);
        let vk = |i: usize| unsafe { inputs[i].Anonymous.ki.wVk };
        let down = |i: usize| unsafe { inputs[i].Anonymous.ki.dwFlags } == 0;

        assert_eq!(vk(0), VK_LWIN);
        assert_eq!(vk(1), VK_CONTROL);
        assert_eq!(vk(2), VK_LEFT);
        assert!(down(0) && down(1) && down(2));

        assert_eq!(vk(5), VK_LWIN);
        assert_eq!(vk(4), VK_CONTROL);
        assert_eq!(vk(3), VK_LEFT);
        assert!(!down(5) && !down(4) && !down(3));
    }
}
