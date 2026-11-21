//! The overlay shown after a jump on Windows: a small layered, click-through,
//! topmost window centred on the target window's monitor, with the window
//! title and "App · 2 / 5". It never takes focus and hides itself after the
//! configured time.
//!
//! It belongs to the daemon's thread, whose message loop dispatches its
//! paint and timer messages. One-shot `window-jump next` has no overlay on
//! Windows: the process ends right after the jump.

use std::cell::RefCell;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateRoundRectRgn,
    CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FW_BOLD, FW_NORMAL, FillRect,
    GetMonitorInfoW, HFONT, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromWindow, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor,
    SetWindowRgn, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HWND_TOPMOST, KillTimer, LWA_ALPHA, RegisterClassW, SW_HIDE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow,
    WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// The window class, also what tests look for (`FindWindow`).
pub const CLASS_NAME: &str = "window-jump-hud";

const WIDTH: i32 = 560;
const HEIGHT: i32 = 112;
const HIDE_TIMER: usize = 1;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

thread_local! {
    /// What the next WM_PAINT draws: (title, detail), NUL-free UTF-16.
    static TEXT: RefCell<(Vec<u16>, Vec<u16>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct HudWindow {
    hwnd: HWND,
}

impl HudWindow {
    /// Creates the (hidden) overlay window on this thread, or `None` if
    /// Windows refuses — the daemon then simply runs without an overlay.
    pub fn new() -> Option<Self> {
        let class = wide(CLASS_NAME);
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
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                WIDTH,
                HEIGHT,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return None;
            }
            SetLayeredWindowAttributes(hwnd, 0, 235, LWA_ALPHA);
            SetWindowRgn(
                hwnd,
                CreateRoundRectRgn(0, 0, WIDTH + 1, HEIGHT + 1, 28, 28),
                1,
            );
            Some(Self { hwnd })
        }
    }

    /// Shows `title` / `detail` centred on the monitor of `near` for
    /// `duration_ms`, replacing whatever was shown.
    pub fn show(&self, near: HWND, title: &str, detail: &str, duration_ms: u32) {
        TEXT.with(|t| {
            *t.borrow_mut() = (
                title.encode_utf16().collect(),
                detail.encode_utf16().collect(),
            );
        });
        unsafe {
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let monitor = MonitorFromWindow(near, MONITOR_DEFAULTTONEAREST);
            let area = if GetMonitorInfoW(monitor, &mut info) != 0 {
                info.rcWork
            } else {
                RECT {
                    left: 0,
                    top: 0,
                    right: 1280,
                    bottom: 720,
                }
            };
            let x = area.left + (area.right - area.left - WIDTH) / 2;
            let y = area.top + (area.bottom - area.top - HEIGHT) / 2;
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                x,
                y,
                WIDTH,
                HEIGHT,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.hwnd, null_mut(), 1);
            SetTimer(self.hwnd, HIDE_TIMER, duration_ms.max(100), None);
        }
    }
}

fn font(height: i32, weight: i32) -> HFONT {
    let face = wide("Segoe UI");
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            face.as_ptr(),
        )
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            // SAFETY: standard paint cycle on our own window.
            unsafe {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let background = CreateSolidBrush(rgb(36, 36, 36));
                let full = RECT {
                    left: 0,
                    top: 0,
                    right: WIDTH,
                    bottom: HEIGHT,
                };
                FillRect(dc, &full, background);
                DeleteObject(background);
                SetBkMode(dc, TRANSPARENT as i32);

                TEXT.with(|t| {
                    let (title, detail) = &mut *t.borrow_mut();
                    let flags =
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX;

                    let title_font = font(24, FW_BOLD as i32);
                    let old = SelectObject(dc, title_font);
                    SetTextColor(dc, rgb(255, 255, 255));
                    let mut r = RECT {
                        left: 24,
                        top: 18,
                        right: WIDTH - 24,
                        bottom: 62,
                    };
                    DrawTextW(dc, title.as_mut_ptr(), title.len() as i32, &mut r, flags);

                    let detail_font = font(17, FW_NORMAL as i32);
                    SelectObject(dc, detail_font);
                    SetTextColor(dc, rgb(200, 200, 200));
                    let mut r = RECT {
                        left: 24,
                        top: 62,
                        right: WIDTH - 24,
                        bottom: HEIGHT - 14,
                    };
                    DrawTextW(dc, detail.as_mut_ptr(), detail.len() as i32, &mut r, flags);

                    SelectObject(dc, old);
                    DeleteObject(title_font);
                    DeleteObject(detail_font);
                });
                EndPaint(hwnd, &ps);
            }
            0
        }
        WM_TIMER if wparam == HIDE_TIMER => {
            unsafe {
                KillTimer(hwnd, HIDE_TIMER);
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
