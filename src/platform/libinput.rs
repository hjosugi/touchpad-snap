//! Touchpad swipes read directly from libinput.
//!
//! On GNOME the shell extension forwards the swipes it claims. Elsewhere
//! (KDE, wlroots, X11 window managers) there is no portable gesture API, so
//! with the optional `libinput` cargo feature the daemon reads the touchpad
//! itself, like libinput-gestures does. libinput also sees keyboard events, so
//! the modifier state at the start of a swipe can be matched against the
//! configured claims — the same `(fingers, mods)` pairs the GNOME extension
//! uses.
//!
//! This is deliberately a separate thread: libinput's dispatch loop must never
//! block the daemon's timer or hotkey handling. Events reach the daemon over a
//! [`std::sync::mpsc`] channel.
//!
//! Reading `/dev/input` needs the user in the `input` group (or an equivalent
//! udev rule); `touchpad-snap doctor` reports whether it is readable. libinput
//! does not take the device away from the compositor, so the compositor still
//! acts on the same gesture. Use plain fingers for the compositor's gestures
//! and a modifier + fingers for touchpad-snap, or turn the compositor gesture
//! off.

use std::path::{Path, PathBuf};

/// The libinput seat desktop sessions normally use.
#[cfg(feature = "libinput")]
const SEAT: &str = "seat0";

/// Linux key codes (linux/input-event-codes.h). libinput reports these
/// directly, unlike X11 keycodes.
#[cfg(feature = "libinput")]
const KEY_LEFTCTRL: u32 = 29;
#[cfg(feature = "libinput")]
const KEY_LEFTSHIFT: u32 = 42;
#[cfg(feature = "libinput")]
const KEY_RIGHTSHIFT: u32 = 54;
#[cfg(feature = "libinput")]
const KEY_LEFTALT: u32 = 56;
#[cfg(feature = "libinput")]
const KEY_RIGHTCTRL: u32 = 97;
#[cfg(feature = "libinput")]
const KEY_RIGHTALT: u32 = 100;
#[cfg(feature = "libinput")]
const KEY_LEFTMETA: u32 = 125;
#[cfg(feature = "libinput")]
const KEY_RIGHTMETA: u32 = 126;

/// libinput's unaccelerated swipe deltas are already the scale the recognizer
/// documents (GNOME's `get_gesture_motion_delta_unaccelerated` uses the same
/// values), so they are passed through unchanged. The function exists to make
/// that choice explicit and testable.
#[cfg(feature = "libinput")]
fn motion_delta(dx: f64, dy: f64) -> (f64, f64) {
    (dx, dy)
}

/// Pure `(fingers, mods)` state for one libinput swipe. Kept free of libinput
/// types so the claim filtering and phases can be tested without hardware.
#[cfg(feature = "libinput")]
#[derive(Debug, Default)]
struct SwipeSession {
    claim: Option<crate::config::Claim>,
}

#[cfg(feature = "libinput")]
impl SwipeSession {
    /// Starts a swipe. Returns the claim when the configured bindings include
    /// this exact finger count and modifier combination; an unclaimed swipe
    /// is ignored until the next begin.
    fn begin(
        &mut self,
        fingers: u32,
        mods: crate::hotkey::Mods,
        claims: &[crate::config::Claim],
    ) -> Option<crate::config::Claim> {
        let claim = claims
            .iter()
            .find(|claim| claim.matches_gesture(fingers, mods))?
            .clone();
        self.claim = Some(claim.clone());
        Some(claim)
    }

    fn update(&mut self, dx: f64, dy: f64) -> Option<(crate::config::Claim, f64, f64)> {
        self.claim.clone().map(|claim| (claim, dx, dy))
    }

    /// Finishes a swipe, mapping libinput's cancelled flag onto [`Phase`].
    fn end(&mut self, cancelled: bool) -> Option<(crate::gesture::Phase, crate::config::Claim)> {
        let claim = self.claim.take()?;
        let phase = if cancelled {
            crate::gesture::Phase::Cancel
        } else {
            crate::gesture::Phase::End
        };
        Some((phase, claim))
    }

    /// Abandons a swipe that cannot be continued (for example the touchpad
    /// was unplugged).
    fn cancel(&mut self) -> Option<(crate::gesture::Phase, crate::config::Claim)> {
        self.claim
            .take()
            .map(|claim| (crate::gesture::Phase::Cancel, claim))
    }
}

/// Which modifier keys are currently down. Left and right variants set the
/// same bit, as in [`crate::hotkey::Mods`].
#[cfg(feature = "libinput")]
#[derive(Debug, Default, Clone, Copy)]
struct ModifierState {
    shift: bool,
    ctrl: bool,
    alt: bool,
    sup: bool,
}

#[cfg(feature = "libinput")]
impl ModifierState {
    /// Applies a key event; returns whether the key was a modifier.
    fn update(&mut self, key: u32, pressed: bool) -> bool {
        let slot = match key {
            KEY_LEFTSHIFT | KEY_RIGHTSHIFT => &mut self.shift,
            KEY_LEFTCTRL | KEY_RIGHTCTRL => &mut self.ctrl,
            KEY_LEFTALT | KEY_RIGHTALT => &mut self.alt,
            KEY_LEFTMETA | KEY_RIGHTMETA => &mut self.sup,
            _ => return false,
        };
        *slot = pressed;
        true
    }

    fn mods(&self) -> crate::hotkey::Mods {
        let mut mods = crate::hotkey::Mods::default();
        if self.shift {
            mods |= crate::hotkey::Mods::SHIFT;
        }
        if self.ctrl {
            mods |= crate::hotkey::Mods::CTRL;
        }
        if self.alt {
            mods |= crate::hotkey::Mods::ALT;
        }
        if self.sup {
            mods |= crate::hotkey::Mods::SUPER;
        }
        mods
    }
}

#[cfg(feature = "libinput")]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(feature = "libinput")]
use std::os::unix::io::OwnedFd;
#[cfg(feature = "libinput")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "libinput")]
use std::sync::{Arc, Mutex, mpsc};
#[cfg(feature = "libinput")]
use std::time::Duration;

#[cfg(feature = "libinput")]
use input::event::EventTrait;
#[cfg(feature = "libinput")]
use input::event::device::DeviceEvent;
#[cfg(feature = "libinput")]
use input::event::gesture::{
    GestureEndEvent, GestureEvent, GestureEventCoordinates, GestureEventTrait, GestureHoldEvent,
    GesturePinchEvent, GesturePinchEventTrait, GestureSwipeEvent,
};
#[cfg(feature = "libinput")]
use input::event::keyboard::{KeyState, KeyboardEvent, KeyboardEventTrait};
#[cfg(feature = "libinput")]
use input::{DeviceCapability, Event as InputEvent, Libinput, LibinputInterface};

#[cfg(feature = "libinput")]
use super::Event;
#[cfg(feature = "libinput")]
use crate::config::Claim;

/// What went wrong opening input devices, for a single startup message.
#[cfg(feature = "libinput")]
#[derive(Debug, Default)]
struct OpenState {
    failure: Mutex<Option<(PathBuf, i32)>>,
}

/// Opens evdev nodes on libinput's behalf. libinput asks for them with the
/// flags it wants; failures (usually a missing `input` group membership) are
/// remembered so the thread can explain them once instead of spamming.
#[cfg(feature = "libinput")]
struct Interface {
    state: Arc<OpenState>,
}

#[cfg(feature = "libinput")]
impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        const O_ACCMODE: i32 = 0o3;
        const O_WRONLY: i32 = 0o1;
        const O_RDWR: i32 = 0o2;

        let access = flags & O_ACCMODE;
        let opened = std::fs::OpenOptions::new()
            .read(access != O_WRONLY)
            .write(access == O_WRONLY || access == O_RDWR)
            .custom_flags(flags & !O_ACCMODE)
            .open(path);
        match opened {
            Ok(file) => Ok(file.into()),
            Err(e) => {
                let errno = e.raw_os_error().unwrap_or(-1);
                if let Ok(mut failure) = self.state.failure.lock() {
                    failure.get_or_insert_with(|| (path.to_path_buf(), errno));
                }
                Err(errno)
            }
        }
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(fd);
    }
}

/// A running libinput gesture reader. Dropping it stops the thread.
#[cfg(feature = "libinput")]
pub struct GestureSource {
    claims: Arc<Mutex<Vec<Claim>>>,
    pinch: Arc<Mutex<Vec<(u32, crate::hotkey::Mods)>>>,
    hold: Arc<Mutex<Vec<(u32, crate::hotkey::Mods)>>>,
    stop: Arc<AtomicBool>,
}

#[cfg(feature = "libinput")]
impl GestureSource {
    /// Starts the reader thread. `claims` filters which gestures become
    /// [`Event::Swipe`]s; use [`GestureSource::set_claims`] after a config
    /// reload.
    pub fn spawn(
        claims: Vec<Claim>,
        pinch: Vec<(u32, crate::hotkey::Mods)>,
        hold: Vec<(u32, crate::hotkey::Mods)>,
        tx: mpsc::Sender<Event>,
        verbose: bool,
    ) -> Self {
        let claims = Arc::new(Mutex::new(claims));
        let pinch = Arc::new(Mutex::new(pinch));
        let hold = Arc::new(Mutex::new(hold));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_claims = Arc::clone(&claims);
        let thread_pinch = Arc::clone(&pinch);
        let thread_hold = Arc::clone(&hold);
        let thread_stop = Arc::clone(&stop);
        let spawned = std::thread::Builder::new()
            .name("touchpad-snap-libinput".into())
            .spawn(move || {
                run(
                    thread_claims,
                    thread_pinch,
                    thread_hold,
                    thread_stop,
                    tx,
                    verbose,
                )
            });
        if let Err(e) = spawned {
            eprintln!("touchpad-snap: could not start the libinput gesture thread: {e}");
        }
        Self {
            claims,
            pinch,
            hold,
            stop,
        }
    }

    /// Replaces the set of claims after a config reload.
    pub fn set_claims(&self, claims: Vec<Claim>) {
        if let Ok(mut current) = self.claims.lock() {
            *current = claims;
        }
    }

    /// Replaces the pinch/hold claims after a config reload.
    pub fn set_gesture_claims(
        &self,
        pinch: Vec<(u32, crate::hotkey::Mods)>,
        hold: Vec<(u32, crate::hotkey::Mods)>,
    ) {
        if let Ok(mut current) = self.pinch.lock() {
            *current = pinch;
        }
        if let Ok(mut current) = self.hold.lock() {
            *current = hold;
        }
    }
}

#[cfg(feature = "libinput")]
impl Drop for GestureSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(feature = "libinput")]
fn run(
    claims: Arc<Mutex<Vec<Claim>>>,
    pinch_claims: Arc<Mutex<Vec<(u32, crate::hotkey::Mods)>>>,
    hold_claims: Arc<Mutex<Vec<(u32, crate::hotkey::Mods)>>>,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<Event>,
    verbose: bool,
) {
    let state = Arc::new(OpenState::default());
    let mut input = Libinput::new_with_udev(Interface {
        state: Arc::clone(&state),
    });
    if input.udev_assign_seat(SEAT).is_err() {
        eprintln!(
            "touchpad-snap: libinput could not be assigned seat {SEAT}; touchpad swipes are unavailable"
        );
        return;
    }

    let mut modifiers = ModifierState::default();
    let mut session = SwipeSession::default();
    let mut pinch: Option<Claim> = None;
    let mut hold: Option<Claim> = None;
    let mut found_touchpad = false;
    let mut reported_startup = false;
    let mut idle_rounds = 0u32;

    while !stop.load(Ordering::Relaxed) {
        let mut any_event = false;
        if input.dispatch().is_ok() {
            for event in input.by_ref() {
                any_event = true;
                match event {
                    InputEvent::Device(DeviceEvent::Added(added)) => {
                        let device = added.device();
                        if device.has_capability(DeviceCapability::Gesture) {
                            found_touchpad = true;
                            if verbose {
                                eprintln!("touchpad-snap: libinput touchpad: {}", device.name());
                            }
                        }
                    }
                    InputEvent::Device(DeviceEvent::Removed(_)) => {
                        if let Some((phase, claim)) = session.cancel()
                            && send(&tx, phase, claim, 0.0, 0.0).is_err()
                        {
                            return;
                        }
                    }
                    InputEvent::Keyboard(KeyboardEvent::Key(key)) => {
                        let pressed = match key.key_state() {
                            KeyState::Pressed => true,
                            KeyState::Released => false,
                        };
                        modifiers.update(key.key(), pressed);
                    }
                    InputEvent::Gesture(GestureEvent::Swipe(swipe)) => match swipe {
                        GestureSwipeEvent::Begin(begin) => {
                            let fingers = begin.finger_count().max(0) as u32;
                            let held = modifiers.mods();
                            let claims = claims.lock().map(|c| c.clone()).unwrap_or_default();
                            if let Some(claim) = session.begin(fingers, held, &claims) {
                                if verbose {
                                    eprintln!(
                                        "touchpad-snap: libinput swipe begins: {fingers} fingers, mods {:#x}",
                                        claim.mods.0
                                    );
                                }
                                if send(&tx, crate::gesture::Phase::Begin, claim, 0.0, 0.0).is_err()
                                {
                                    return;
                                }
                            }
                        }
                        GestureSwipeEvent::Update(update) => {
                            let (dx, dy) =
                                motion_delta(update.dx_unaccelerated(), update.dy_unaccelerated());
                            if let Some((claim, dx, dy)) = session.update(dx, dy)
                                && (dx != 0.0 || dy != 0.0)
                                && send(&tx, crate::gesture::Phase::Update, claim, dx, dy).is_err()
                            {
                                return;
                            }
                        }
                        GestureSwipeEvent::End(end) => {
                            if let Some((phase, claim)) = session.end(end.cancelled())
                                && send(&tx, phase, claim, 0.0, 0.0).is_err()
                            {
                                return;
                            }
                        }
                        _ => {}
                    },
                    InputEvent::Gesture(GestureEvent::Pinch(event)) => match event {
                        GesturePinchEvent::Begin(begin) => {
                            let fingers = begin.finger_count().max(0) as u32;
                            let mods = modifiers.mods();
                            if pinch_claims
                                .lock()
                                .map(|c| c.contains(&(fingers, mods)))
                                .unwrap_or(false)
                            {
                                let claim = Claim {
                                    fingers,
                                    mods,
                                    apps: Vec::new(),
                                };
                                pinch = Some(claim.clone());
                                if send_pinch(&tx, crate::gesture::Phase::Begin, claim, 1.0)
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        GesturePinchEvent::Update(update) => {
                            if let Some(claim) = &pinch
                                && send_pinch(
                                    &tx,
                                    crate::gesture::Phase::Update,
                                    claim.clone(),
                                    update.scale(),
                                )
                                .is_err()
                            {
                                return;
                            }
                        }
                        GesturePinchEvent::End(end) => {
                            if let Some(claim) = pinch.take()
                                && send_pinch(&tx, crate::gesture::Phase::End, claim, end.scale())
                                    .is_err()
                            {
                                return;
                            }
                        }
                        _ => {}
                    },
                    InputEvent::Gesture(GestureEvent::Hold(event)) => match event {
                        GestureHoldEvent::Begin(begin) => {
                            let fingers = begin.finger_count().max(0) as u32;
                            let mods = modifiers.mods();
                            if hold_claims
                                .lock()
                                .map(|c| c.contains(&(fingers, mods)))
                                .unwrap_or(false)
                            {
                                let claim = Claim {
                                    fingers,
                                    mods,
                                    apps: Vec::new(),
                                };
                                hold = Some(claim.clone());
                                if tx
                                    .send(Event::Hold {
                                        phase: crate::gesture::Phase::Begin,
                                        fingers,
                                        mods,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        }
                        GestureHoldEvent::End(_) => {
                            if let Some(claim) = hold.take()
                                && tx
                                    .send(Event::Hold {
                                        phase: crate::gesture::Phase::End,
                                        fingers: claim.fingers,
                                        mods: claim.mods,
                                    })
                                    .is_err()
                            {
                                return;
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        if !reported_startup {
            idle_rounds += 1;
            if found_touchpad {
                reported_startup = true;
            } else if idle_rounds >= 20 {
                reported_startup = true;
                let failure = state
                    .failure
                    .lock()
                    .ok()
                    .and_then(|failure| failure.clone());
                if let Some((path, errno)) = failure {
                    eprintln!(
                        "touchpad-snap: cannot open {}: {errno} ({}); touchpad swipes are \
                         unavailable. Add your user to the \"input\" group and log in again.",
                        path.display(),
                        std::io::Error::from_raw_os_error(errno)
                    );
                } else {
                    eprintln!(
                        "touchpad-snap: no touchpad with gesture support was found; \
                         touchpad swipes are unavailable"
                    );
                }
            }
        }

        if !any_event {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(feature = "libinput")]
fn send_pinch(
    tx: &mpsc::Sender<Event>,
    phase: crate::gesture::Phase,
    claim: Claim,
    scale: f64,
) -> Result<(), mpsc::SendError<Event>> {
    tx.send(Event::Pinch {
        phase,
        fingers: claim.fingers,
        mods: claim.mods,
        scale,
    })
}

#[cfg(feature = "libinput")]
fn send(
    tx: &mpsc::Sender<Event>,
    phase: crate::gesture::Phase,
    claim: Claim,
    dx: f64,
    dy: f64,
) -> Result<(), mpsc::SendError<Event>> {
    tx.send(Event::Swipe {
        phase,
        claim,
        app_id: None,
        dx,
        dy,
    })
}

/// Linux input capability bits used by the sysfs touchpad check.
const ABS_MT_POSITION_X: u32 = 0x35;
const ABS_MT_POSITION_Y: u32 = 0x36;
const BTN_TOOL_FINGER: u32 = 0x145;
const BTN_TOOL_TRIPLETAP: u32 = 0x14a;
const BTN_TOOL_QUADTAP: u32 = 0x14b;

/// Whether a capability file's space-separated hex words have `bit` set.
/// Each word holds 64 bits, least significant word first.
fn bit_is_set(words: &str, bit: u32) -> bool {
    let word = (bit / 64) as usize;
    let offset = bit % 64;
    words
        .split_whitespace()
        .nth(word)
        .and_then(|word| u64::from_str_radix(word, 16).ok())
        .is_some_and(|word| word & (1u64 << offset) != 0)
}

/// A multi-touch pointer: the MT position axes plus a finger tool. This is
/// what udev/libinput recognise as a touchpad; mice have relative axes but no
/// MT slots.
fn looks_like_touchpad(abs: &str, key: &str) -> bool {
    bit_is_set(abs, ABS_MT_POSITION_X)
        && bit_is_set(abs, ABS_MT_POSITION_Y)
        && (bit_is_set(key, BTN_TOOL_FINGER)
            || bit_is_set(key, BTN_TOOL_TRIPLETAP)
            || bit_is_set(key, BTN_TOOL_QUADTAP))
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
}

/// Touchpads advertised by sysfs, as `(name, /dev/input/eventN)`.
fn touchpads() -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let event = entry.file_name();
        let event = event.to_string_lossy();
        if !event.starts_with("event") {
            continue;
        }
        let device = entry.path().join("device");
        let abs = read_trimmed(&device.join("capabilities/abs")).unwrap_or_default();
        let key = read_trimmed(&device.join("capabilities/key")).unwrap_or_default();
        if looks_like_touchpad(&abs, &key) {
            let name = read_trimmed(&device.join("name")).unwrap_or_else(|| event.to_string());
            found.push((name, Path::new("/dev/input").join(event.as_ref())));
        }
    }
    found
}

/// Looks up a group name in `/etc/group` content.
fn group_name(contents: &str, gid: u32) -> Option<String> {
    contents.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.len() >= 3 && fields[2].parse::<u32>().ok() == Some(gid))
            .then(|| fields[0].to_string())
    })
}

fn first_event_path() -> Option<PathBuf> {
    std::fs::read_dir("/dev/input")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("event"))
        })
}

#[cfg(unix)]
fn device_group() -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    let path = touchpads()
        .into_iter()
        .map(|(_, path)| path)
        .next()
        .or_else(first_event_path)
        .unwrap_or_else(|| PathBuf::from("/dev/input"));
    let gid = std::fs::metadata(path).ok()?.gid();
    let groups = std::fs::read_to_string("/etc/group").ok()?;
    // /dev/input itself is often root-owned; never advise joining root.
    group_name(&groups, gid).filter(|name| name != "root")
}

#[cfg(not(unix))]
fn device_group() -> Option<String> {
    None
}

fn input_readable(touchpads: &[(String, PathBuf)]) -> bool {
    let candidates: Vec<PathBuf> = if touchpads.is_empty() {
        let Ok(entries) = std::fs::read_dir("/dev/input") else {
            return false;
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("event"))
            })
            .collect()
    } else {
        touchpads.iter().map(|(_, path)| path.clone()).collect()
    };
    candidates
        .iter()
        .any(|path| std::fs::File::open(path).is_ok())
}

/// One line for `touchpad-snap doctor`. Never fails and never needs the feature:
/// it reports whether this build can read gestures, whether a touchpad is
/// present, and whether `/dev/input` is readable.
pub fn doctor() -> String {
    let compiled = if cfg!(feature = "libinput") {
        "compiled"
    } else {
        "not compiled (build with --features libinput)"
    };
    let touchpads = touchpads();
    let device = match touchpads.first() {
        Some((name, _)) => format!("touchpad \"{name}\" found"),
        None => "no touchpad found".to_string(),
    };
    let access = if input_readable(&touchpads) {
        "/dev/input readable".to_string()
    } else {
        match device_group() {
            Some(group) => format!(
                "/dev/input not readable (add your user to the \"{group}\" group: \
                 sudo usermod -aG {group} $USER, then log in again)"
            ),
            None => "/dev/input not readable (add your user to the input group, then log in again)"
                .to_string(),
        }
    };
    format!("{compiled}; {device}; {access}")
}

#[cfg(all(test, feature = "libinput"))]
mod tests {
    use super::*;
    use crate::gesture::Phase;
    use crate::hotkey::Mods;

    fn claim(fingers: u32, mods: Mods) -> Claim {
        Claim::gesture(fingers, mods)
    }

    #[test]
    fn modifier_state_tracks_both_sides_and_non_modifiers() {
        let mut state = ModifierState::default();
        assert!(state.update(KEY_LEFTCTRL, true));
        assert!(state.update(KEY_RIGHTSHIFT, true));
        assert!(state.update(KEY_LEFTALT, true));
        assert!(state.update(KEY_LEFTMETA, true));
        assert_eq!(
            state.mods(),
            Mods::CTRL | Mods::SHIFT | Mods::ALT | Mods::SUPER
        );

        assert!(state.update(KEY_LEFTCTRL, false));
        assert_eq!(state.mods(), Mods::SHIFT | Mods::ALT | Mods::SUPER);
        assert!(!state.update(30, true));
        assert_eq!(state.mods(), Mods::SHIFT | Mods::ALT | Mods::SUPER);
    }

    #[test]
    fn begin_checks_claims_and_snapshots_modifiers() {
        let claims = [claim(3, Mods::CTRL), claim(4, Mods::CTRL | Mods::ALT)];
        let mut session = SwipeSession::default();

        assert_eq!(
            session.begin(3, Mods::CTRL, &claims),
            Some(claim(3, Mods::CTRL))
        );
        assert_eq!(
            session.update(12.0, -3.0),
            Some((claim(3, Mods::CTRL), 12.0, -3.0))
        );
        assert_eq!(session.end(false), Some((Phase::End, claim(3, Mods::CTRL))));

        // Four fingers without Alt does not match the four-finger claim.
        assert_eq!(session.begin(4, Mods::CTRL, &claims), None);
        assert_eq!(session.update(10.0, 10.0), None);
        assert_eq!(session.end(false), None);
    }

    #[test]
    fn finger_count_must_match_a_claim() {
        let claims = [claim(3, Mods::default()), claim(5, Mods::default())];
        for (fingers, expected) in [(3, true), (4, false), (5, true), (2, false)] {
            let mut session = SwipeSession::default();
            assert_eq!(
                session.begin(fingers, Mods::default(), &claims).is_some(),
                expected,
                "{fingers} fingers"
            );
        }
    }

    #[test]
    fn phases_end_and_cancel_take_the_claim_once() {
        let claims = [claim(3, Mods::default())];
        let mut session = SwipeSession::default();
        session.begin(3, Mods::default(), &claims);
        assert_eq!(
            session.end(true),
            Some((Phase::Cancel, claim(3, Mods::default())))
        );
        assert_eq!(session.end(false), None);
        assert_eq!(session.cancel(), None);

        session.begin(3, Mods::default(), &claims);
        assert_eq!(
            session.cancel(),
            Some((Phase::Cancel, claim(3, Mods::default())))
        );
        assert_eq!(session.update(1.0, 1.0), None);
    }

    #[test]
    fn libinput_deltas_pass_through_unchanged() {
        // Positive X is right and positive Y is down, like GNOME's deltas;
        // the recognizer turns these into Right/Down.
        assert_eq!(motion_delta(3.5, -2.25), (3.5, -2.25));
        assert_eq!(motion_delta(-400.0, 0.0), (-400.0, 0.0));
    }
}

#[cfg(test)]
mod doctor_tests {
    use super::*;

    #[test]
    fn capability_words_are_little_endian_bit_masks() {
        // 0x0000000000000030 sets bits 4 and 5; the next word starts at 64.
        assert!(bit_is_set("30", 4));
        assert!(bit_is_set("30", 5));
        assert!(!bit_is_set("30", 6));
        assert!(bit_is_set("0 1", 64));
        assert!(!bit_is_set("0 1", 65));
        assert!(!bit_is_set("", 0));
        assert!(!bit_is_set("zz", 0));
    }

    #[test]
    fn touchpads_are_multi_touch_pointers() {
        // ABS_MT_POSITION_X (53) and Y (54) live in the first word; BTN_TOOL_FINGER
        // (325) is bit 5 of the sixth word.
        let abs = "60000000000000";
        let key = "0 0 0 0 0 20";
        assert!(looks_like_touchpad(abs, key));

        // A mouse has keys but no multi-touch position axes.
        assert!(!looks_like_touchpad("3", "0"));
        // A touchscreen-like device without BTN_TOOL_FINGER is not a touchpad.
        assert!(!looks_like_touchpad(abs, "0"));
    }

    #[test]
    fn group_names_come_from_the_gid_field() {
        let groups = "root:x:0:\ninput:x:174:alice\nwheel:x:10:alice\n";
        assert_eq!(group_name(groups, 174).as_deref(), Some("input"));
        assert_eq!(group_name(groups, 0).as_deref(), Some("root"));
        assert_eq!(group_name(groups, 999), None);
        assert_eq!(group_name("broken\n", 174), None);
    }

    #[test]
    fn doctor_always_answers() {
        let line = doctor();
        assert!(!line.is_empty());
        assert!(
            line.contains("compiled")
                || line.contains("not compiled (build with --features libinput)")
        );
    }
}
