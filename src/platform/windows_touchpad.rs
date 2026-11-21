//! Native Precision Touchpad swipes for the Windows daemon.
//!
//! A hidden message-only window receives raw input from the HID collection
//! with usage page 0x0D (Digitizer) and usage 0x05 (Touch Pad) via
//! `RIDEV_INPUTSINK`, so swipes work while another window is focused. Each
//! report's contact count and X/Y are parsed with the HID parser API and fed
//! through [`TouchpadTracker`], producing the same begin/update/end motion
//! the GNOME extension sends.
//!
//! Windows' own three/four-finger gestures still reach the shell unless the
//! user turns them off in Settings → Touchpad → Advanced gestures; raw input
//! cannot intercept them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDP_BUTTON_CAPS, HIDP_CAPS, HIDP_DATA, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS,
    HidP_GetButtonCaps, HidP_GetCaps, HidP_GetData, HidP_GetUsageValue, HidP_GetUsages,
    HidP_GetValueCaps, HidP_Input, HidP_MaxDataListLength, HidP_MaxUsageListLength,
};
use windows_sys::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWHID, RAWINPUT,
    RAWINPUTDEVICE, RAWINPUTDEVICELIST, RAWINPUTHEADER, RID_DEVICE_INFO, RID_INPUT,
    RIDEV_INPUTSINK, RIDI_DEVICEINFO, RIDI_PREPARSEDDATA, RIM_TYPEHID, RegisterRawInputDevices,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_MESSAGE, RegisterClassW, WM_INPUT,
    WNDCLASSW,
};

use crate::gesture::{Phase, TouchpadTracker};

/// HID usage page and usage of a Windows Precision Touchpad's top-level
/// collection.
const DIGITIZER: u16 = 0x0D;
const TOUCH_PAD: u16 = 0x05;
const CONTACT_COUNT: u16 = 0x54;
const TIP_SWITCH: u16 = 0x42;
const X: u16 = 0x30;
const Y: u16 = 0x31;
// Contact Identifier (0x51) is deliberately not used: averaging the reported
// X/Y values needs no stable identity, and reports may reassign them.

const CLASS_NAME: &str = "touchpad-snap-touchpad";

/// One parsed report, already reduced to what the platform-neutral tracker
/// needs.
pub struct Sample {
    pub phase: Phase,
    pub dx: f64,
    pub dy: f64,
    pub fingers: u32,
}

thread_local! {
    static DEVICES: RefCell<HashMap<usize, Device>> = RefCell::new(HashMap::new());
    static SAMPLES: RefCell<Vec<Sample>> = const { RefCell::new(Vec::new()) };
}

/// The daemon's raw-input window. Created once on the daemon thread.
pub struct Source {
    /// Kept so the window outlives the registration; the process exit is the
    /// cleanup path.
    _hwnd: HWND,
}

impl Source {
    /// Creates the hidden window and subscribes to touchpad raw input. Logs
    /// once when no Precision Touchpad is attached. `swipes_configured`
    /// silences the log when the user has no `[[swipe]]` bindings.
    pub fn new(swipes_configured: bool) -> Option<Self> {
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = wide(CLASS_NAME);
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                // SAFETY: every other field may be zero.
                ..std::mem::zeroed()
            };
            // Fails harmlessly if the class already exists (a second daemon
            // is refused earlier, but be safe).
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                if swipes_configured {
                    eprintln!("touchpad-snap: could not create the touchpad input window");
                }
                return None;
            }

            if swipes_configured && !touchpad_present() {
                eprintln!(
                    "touchpad-snap: no Windows Precision Touchpad found; [[swipe]] bindings will not fire"
                );
            }

            let device = RAWINPUTDEVICE {
                usUsagePage: DIGITIZER,
                usUsage: TOUCH_PAD,
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: hwnd,
            };
            let registered =
                RegisterRawInputDevices(&device, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
            if registered == 0 {
                if swipes_configured {
                    eprintln!("touchpad-snap: could not register for touchpad raw input");
                }
                DestroyWindow(hwnd);
                return None;
            }
            Some(Self { _hwnd: hwnd })
        }
    }

    /// The samples received since the last call, oldest first. The daemon
    /// turns them into `Event::Swipe` after dispatching the window messages.
    pub fn drain(&self) -> Vec<Sample> {
        SAMPLES.with(|samples| std::mem::take(&mut *samples.borrow_mut()))
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_INPUT {
        on_raw_input(lparam);
    }
    // WM_INPUT requires DefWindowProcW for the system to clean up.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn on_raw_input(lparam: LPARAM) {
    let hraw = lparam as *mut c_void;
    let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let mut size = 0u32;
    // SAFETY: asking user32 for the size of this WM_INPUT's buffer.
    let asked = unsafe { GetRawInputData(hraw, RID_INPUT, null_mut(), &mut size, header_size) };
    if asked == u32::MAX || size == 0 {
        return;
    }

    // RAWINPUT contains pointers and is 8-byte aligned; a Vec<u64> guarantees
    // that without read_unaligned.
    let mut buf = vec![0u64; size.div_ceil(8) as usize];
    let mut size = (buf.len() * 8) as u32;
    // SAFETY: buf is at least `size` bytes and properly aligned.
    let got = unsafe {
        GetRawInputData(
            hraw,
            RID_INPUT,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            header_size,
        )
    };
    if got == u32::MAX || got == 0 {
        return;
    }

    // SAFETY: GetRawInputData filled the buffer with a RAWINPUT whose header
    // is within it.
    let (device, report_size, report_count) = unsafe {
        let input = &*(buf.as_ptr() as *const RAWINPUT);
        if input.header.dwType != RIM_TYPEHID {
            return;
        }
        let hid = &input.data.hid;
        (
            input.header.hDevice,
            hid.dwSizeHid as usize,
            hid.dwCount as usize,
        )
    };
    if report_size == 0 || report_count == 0 {
        return;
    }
    let total = report_size.saturating_mul(report_count);
    let offset = std::mem::size_of::<RAWINPUTHEADER>() + std::mem::offset_of!(RAWHID, bRawData);
    if offset.saturating_add(total) > buf.len() * 8 {
        return;
    }
    // SAFETY: the reports follow the fixed-size RAWHID header in `buf`.
    let reports =
        unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>().add(offset), total) };
    for report in reports.chunks_mut(report_size) {
        process_report(device, report);
    }
}

fn process_report(handle: HANDLE, report: &mut [u8]) {
    let sample = DEVICES.with(|devices| {
        let mut devices = devices.borrow_mut();
        let key = handle as usize;
        let device = match devices.entry(key) {
            std::collections::hash_map::Entry::Occupied(slot) => slot.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => slot.insert(Device::new(handle)?),
        };
        let (contacts, x, y) = device.parse(report)?;
        let (phase, dx, dy) = device.tracker.feed(contacts, x, y)?;
        Some(Sample {
            phase,
            dx,
            dy,
            fingers: device.tracker.fingers(),
        })
    });
    if let Some(sample) = sample {
        SAMPLES.with(|samples| samples.borrow_mut().push(sample));
    }
}

/// The per-device preparsed HID data and the parts of it we need.
struct Device {
    preparsed: Vec<u8>,
    /// Data indices of X and Y values (a usage array has several).
    x_items: Vec<(u16, u16)>,
    y_items: Vec<(u16, u16)>,
    /// Link collections that contain Tip Switch buttons.
    tip_links: Vec<u16>,
    max_usages: usize,
    tracker: TouchpadTracker,
}

impl Device {
    fn new(handle: HANDLE) -> Option<Self> {
        // The preparsed data is needed for every parse but never changes while
        // the device is attached: fetch and keep it once.
        let mut size = 0u32;
        // SAFETY: two-call size query against this device.
        let asked =
            unsafe { GetRawInputDeviceInfoW(handle, RIDI_PREPARSEDDATA, null_mut(), &mut size) };
        if asked == u32::MAX || size == 0 {
            return None;
        }
        let mut preparsed = vec![0u8; size as usize];
        // SAFETY: the buffer is `size` bytes.
        let filled = unsafe {
            GetRawInputDeviceInfoW(
                handle,
                RIDI_PREPARSEDDATA,
                preparsed.as_mut_ptr() as *mut c_void,
                &mut size,
            )
        };
        if filled == u32::MAX {
            return None;
        }
        preparsed.truncate(size as usize);

        let prep = preparsed.as_ptr() as isize;
        let mut caps = HIDP_CAPS::default();
        // SAFETY: prep points at the preparsed data retained above.
        if unsafe { HidP_GetCaps(prep, &mut caps) } != HIDP_STATUS_SUCCESS
            || caps.UsagePage != DIGITIZER
            || caps.Usage != TOUCH_PAD
        {
            return None;
        }

        let value_caps = value_caps(prep, caps.NumberInputValueCaps);
        let button_caps = button_caps(prep, caps.NumberInputButtonCaps);

        let x_items: Vec<(u16, u16)> = value_caps
            .iter()
            .filter(|c| c.UsagePage == DIGITIZER && covers(cap_usage(c), X))
            .map(data_indices)
            .collect();
        let y_items: Vec<(u16, u16)> = value_caps
            .iter()
            .filter(|c| c.UsagePage == DIGITIZER && covers(cap_usage(c), Y))
            .map(data_indices)
            .collect();
        if x_items.is_empty() || y_items.is_empty() {
            return None;
        }

        let mut tip_links = Vec::new();
        for cap in &button_caps {
            if cap.UsagePage == DIGITIZER
                && covers(button_usage(cap), TIP_SWITCH)
                && !tip_links.contains(&cap.LinkCollection)
            {
                tip_links.push(cap.LinkCollection);
            }
        }

        let x_range = logical_range(&value_caps, X).unwrap_or(0.0);
        let y_range = logical_range(&value_caps, Y).unwrap_or(0.0);
        // SAFETY: prep is valid and the caps came from it.
        let max_usages = unsafe { HidP_MaxUsageListLength(HidP_Input, DIGITIZER, prep) } as usize;

        Some(Self {
            preparsed,
            x_items,
            y_items,
            tip_links,
            max_usages: max_usages.max(1),
            tracker: TouchpadTracker::new(x_range, y_range),
        })
    }

    /// `(contacts, x, y)` from one report, or `None` if it cannot be parsed.
    fn parse(&self, report: &mut [u8]) -> Option<(u32, i32, i32)> {
        let prep = self.preparsed.as_ptr() as isize;
        let mut contact_count = 0u32;
        // SAFETY: report is one full HID input report for this device.
        let status = unsafe {
            HidP_GetUsageValue(
                HidP_Input,
                DIGITIZER,
                0,
                CONTACT_COUNT,
                &mut contact_count,
                prep,
                report.as_ptr(),
                report.len() as u32,
            )
        };
        if status != HIDP_STATUS_SUCCESS {
            return None;
        }
        if contact_count == 0 {
            return Some((0, 0, 0));
        }

        // Tip Switch tells touching contacts from hovering ones; when the
        // device does not expose it, the contact count is all we have.
        let tips = self.tip_count(report);
        let contacts = if tips > 0 { tips } else { contact_count };

        let (x, y) = self.point(report).or_else(|| self.single_point(report))?;
        Some((contacts, x, y))
    }

    /// Mean X/Y over every contact in the report.
    fn point(&self, report: &mut [u8]) -> Option<(i32, i32)> {
        let prep = self.preparsed.as_ptr() as isize;
        let max = unsafe { HidP_MaxDataListLength(HidP_Input, prep) } as usize;
        if max == 0 {
            return None;
        }
        let mut data = vec![HIDP_DATA::default(); max];
        let mut len = max as u32;
        // SAFETY: report is one full HID input report for this device.
        let status = unsafe {
            HidP_GetData(
                HidP_Input,
                data.as_mut_ptr(),
                &mut len,
                prep,
                report.as_mut_ptr(),
                report.len() as u32,
            )
        };
        if status != HIDP_STATUS_SUCCESS {
            return None;
        }
        let data = &data[..len as usize];
        let x = mean_value(data, &self.x_items)?;
        let y = mean_value(data, &self.y_items)?;
        Some((x.round() as i32, y.round() as i32))
    }

    /// Fallback for descriptors whose X/Y are single values: ask the parser
    /// directly, which also handles a negative logical minimum.
    fn single_point(&self, report: &[u8]) -> Option<(i32, i32)> {
        let prep = self.preparsed.as_ptr() as isize;
        let mut x = 0u32;
        let mut y = 0u32;
        // SAFETY: report is one full HID input report for this device.
        unsafe {
            if HidP_GetUsageValue(
                HidP_Input,
                DIGITIZER,
                0,
                X,
                &mut x,
                prep,
                report.as_ptr(),
                report.len() as u32,
            ) != HIDP_STATUS_SUCCESS
            {
                return None;
            }
            if HidP_GetUsageValue(
                HidP_Input,
                DIGITIZER,
                0,
                Y,
                &mut y,
                prep,
                report.as_ptr(),
                report.len() as u32,
            ) != HIDP_STATUS_SUCCESS
            {
                return None;
            }
        }
        Some((x as i32, y as i32))
    }

    /// Number of Tip Switch buttons currently on.
    fn tip_count(&self, report: &mut [u8]) -> u32 {
        if self.tip_links.is_empty() {
            return 0;
        }
        let prep = self.preparsed.as_ptr() as isize;
        let mut usages = vec![0u16; self.max_usages];
        let mut total = 0u32;
        for &link in &self.tip_links {
            let mut len = usages.len() as u32;
            // SAFETY: report is one full HID input report for this device.
            let status = unsafe {
                HidP_GetUsages(
                    HidP_Input,
                    DIGITIZER,
                    link,
                    usages.as_mut_ptr(),
                    &mut len,
                    prep,
                    report.as_mut_ptr(),
                    report.len() as u32,
                )
            };
            if status != HIDP_STATUS_SUCCESS {
                continue;
            }
            let len = (len as usize).min(usages.len());
            total += usages[..len].iter().filter(|&&u| u == TIP_SWITCH).count() as u32;
        }
        total
    }
}

fn value_caps(prep: isize, count: u16) -> Vec<HIDP_VALUE_CAPS> {
    if count == 0 {
        return Vec::new();
    }
    let mut caps = vec![HIDP_VALUE_CAPS::default(); count as usize];
    let mut len = count;
    // SAFETY: prep is valid and caps has room for `len` entries.
    let status = unsafe { HidP_GetValueCaps(HidP_Input, caps.as_mut_ptr(), &mut len, prep) };
    if status != HIDP_STATUS_SUCCESS {
        return Vec::new();
    }
    caps.truncate(len as usize);
    caps
}

fn button_caps(prep: isize, count: u16) -> Vec<HIDP_BUTTON_CAPS> {
    if count == 0 {
        return Vec::new();
    }
    let mut caps = vec![HIDP_BUTTON_CAPS::default(); count as usize];
    let mut len = count;
    // SAFETY: prep is valid and caps has room for `len` entries.
    let status = unsafe { HidP_GetButtonCaps(HidP_Input, caps.as_mut_ptr(), &mut len, prep) };
    if status != HIDP_STATUS_SUCCESS {
        return Vec::new();
    }
    caps.truncate(len as usize);
    caps
}

fn cap_usage(cap: &HIDP_VALUE_CAPS) -> (u16, u16) {
    if cap.IsRange {
        // SAFETY: IsRange selects the Range arm of the union.
        unsafe { (cap.Anonymous.Range.UsageMin, cap.Anonymous.Range.UsageMax) }
    } else {
        // SAFETY: !IsRange selects the NotRange arm of the union.
        let usage = unsafe { cap.Anonymous.NotRange.Usage };
        (usage, usage)
    }
}

fn button_usage(cap: &HIDP_BUTTON_CAPS) -> (u16, u16) {
    if cap.IsRange {
        // SAFETY: IsRange selects the Range arm of the union.
        unsafe { (cap.Anonymous.Range.UsageMin, cap.Anonymous.Range.UsageMax) }
    } else {
        // SAFETY: !IsRange selects the NotRange arm of the union.
        let usage = unsafe { cap.Anonymous.NotRange.Usage };
        (usage, usage)
    }
}

fn covers((lo, hi): (u16, u16), usage: u16) -> bool {
    (lo..=hi).contains(&usage)
}

/// The range of `HidP_GetData` indices holding one value. A usage array
/// reports several values under the same usage.
fn data_indices(cap: &HIDP_VALUE_CAPS) -> (u16, u16) {
    if cap.IsRange {
        // SAFETY: IsRange selects the Range arm of the union.
        unsafe {
            (
                cap.Anonymous.Range.DataIndexMin,
                cap.Anonymous.Range.DataIndexMax,
            )
        }
    } else {
        // SAFETY: !IsRange selects the NotRange arm of the union.
        let first = unsafe { cap.Anonymous.NotRange.DataIndex };
        let count = cap.ReportCount.max(1);
        (first, first.saturating_add(count - 1))
    }
}

fn logical_range(caps: &[HIDP_VALUE_CAPS], usage: u16) -> Option<f64> {
    caps.iter().find_map(|cap| {
        if cap.UsagePage != DIGITIZER || !covers(cap_usage(cap), usage) {
            return None;
        }
        let range = cap.LogicalMax as f64 - cap.LogicalMin as f64;
        (range > 0.0).then_some(range)
    })
}

fn mean_value(data: &[HIDP_DATA], items: &[(u16, u16)]) -> Option<f64> {
    let mut sum = 0.0;
    let mut count = 0u32;
    for item in data {
        if items
            .iter()
            .any(|&(lo, hi)| (lo..=hi).contains(&item.DataIndex))
        {
            // SAFETY: value data carries the raw value in the RawValue arm.
            sum += unsafe { item.Anonymous.RawValue } as i32 as f64;
            count += 1;
        }
    }
    (count > 0).then(|| sum / count as f64)
}

/// Whether any attached raw-input HID device is a Precision Touchpad. Used
/// only to explain an otherwise silent lack of swipes.
fn touchpad_present() -> bool {
    let elem = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut count = 0u32;
    // SAFETY: first call only asks for the number of devices.
    if unsafe { GetRawInputDeviceList(null_mut(), &mut count, elem) } == u32::MAX || count == 0 {
        return false;
    }
    let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
    // SAFETY: list has room for `count` entries.
    let found = unsafe { GetRawInputDeviceList(list.as_mut_ptr(), &mut count, elem) };
    if found == u32::MAX {
        return false;
    }
    list.truncate(found as usize);
    list.iter().any(|device| {
        if device.dwType != RIM_TYPEHID {
            return false;
        }
        let mut info = RID_DEVICE_INFO {
            cbSize: std::mem::size_of::<RID_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        let mut size = info.cbSize;
        // SAFETY: info is the expected size and type.
        let ok = unsafe {
            GetRawInputDeviceInfoW(
                device.hDevice,
                RIDI_DEVICEINFO,
                &mut info as *mut _ as *mut c_void,
                &mut size,
            )
        };
        if ok == u32::MAX {
            return false;
        }
        // SAFETY: RIM_TYPEHID devices carry the HID arm of the union.
        let hid = unsafe { info.Anonymous.hid };
        hid.usUsagePage == DIGITIZER && hid.usUsage == TOUCH_PAD
    })
}
