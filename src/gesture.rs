//! Turns a stream of touchpad swipe deltas into discrete swipe steps.
//!
//! The GNOME extension forwards the raw (unaccelerated) motion of a claimed
//! swipe; this recognizer decides when it amounts to a swipe and in which
//! direction. After the first step the axis locks, and with `repeat_step`
//! set, every further `repeat_step` of travel along it fires again — in
//! either direction, so moving back scrubs back through the windows.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwipeDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Phases as sent by the extension in the `Swipe` signal. Part of the
/// extension protocol: do not renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Begin,
    Update,
    End,
    Cancel,
}

impl Phase {
    pub fn from_u32(v: u32) -> Option<Phase> {
        match v {
            0 => Some(Phase::Begin),
            1 => Some(Phase::Update),
            2 => Some(Phase::End),
            3 => Some(Phase::Cancel),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub struct SwipeRecognizer {
    threshold: f64,
    repeat_step: f64,
    acc_x: f64,
    acc_y: f64,
    axis: Option<Axis>,
    active: bool,
}

impl SwipeRecognizer {
    /// `threshold`: travel (in libinput's unaccelerated units, roughly the
    /// same scale GNOME uses: a full workspace swipe is 400) before the first
    /// step. `repeat_step`: travel per further step; 0 disables repeats.
    pub fn new(threshold: f64, repeat_step: f64) -> Self {
        Self {
            threshold: threshold.max(1.0),
            repeat_step: repeat_step.max(0.0),
            acc_x: 0.0,
            acc_y: 0.0,
            axis: None,
            active: false,
        }
    }

    pub fn begin(&mut self) {
        self.acc_x = 0.0;
        self.acc_y = 0.0;
        self.axis = None;
        self.active = true;
    }

    pub fn end(&mut self) {
        self.active = false;
    }

    /// Feeds one motion delta; returns the steps it completes (usually none
    /// or one).
    pub fn update(&mut self, dx: f64, dy: f64) -> Vec<SwipeDirection> {
        let mut out = Vec::new();
        if !self.active {
            return out;
        }
        self.acc_x += dx;
        self.acc_y += dy;

        let axis = match self.axis {
            Some(a) => a,
            None => {
                if self.acc_x.hypot(self.acc_y) < self.threshold {
                    return out;
                }
                let axis = if self.acc_x.abs() >= self.acc_y.abs() {
                    Axis::Horizontal
                } else {
                    Axis::Vertical
                };
                let along = self.along(axis);
                out.push(direction(axis, along));
                self.axis = Some(axis);
                // Keep only the travel beyond the threshold, on the locked axis.
                self.set_along(axis, along - self.threshold.copysign(along));
                if self.repeat_step == 0.0 {
                    // One step per gesture: stop listening until the next begin.
                    self.active = false;
                    return out;
                }
                axis
            }
        };

        if self.repeat_step > 0.0 {
            // Cap per update so a wild delta cannot cycle dozens of windows.
            for _ in 0..8 {
                let along = self.along(axis);
                if along.abs() < self.repeat_step {
                    break;
                }
                out.push(direction(axis, along));
                self.set_along(axis, along - self.repeat_step.copysign(along));
            }
        }
        out
    }

    fn along(&self, axis: Axis) -> f64 {
        match axis {
            Axis::Horizontal => self.acc_x,
            Axis::Vertical => self.acc_y,
        }
    }

    fn set_along(&mut self, axis: Axis, v: f64) {
        match axis {
            Axis::Horizontal => {
                self.acc_x = v;
                self.acc_y = 0.0;
            }
            Axis::Vertical => {
                self.acc_y = v;
                self.acc_x = 0.0;
            }
        }
    }
}

fn direction(axis: Axis, along: f64) -> SwipeDirection {
    match (axis, along >= 0.0) {
        (Axis::Horizontal, true) => SwipeDirection::Right,
        (Axis::Horizontal, false) => SwipeDirection::Left,
        (Axis::Vertical, true) => SwipeDirection::Down,
        (Axis::Vertical, false) => SwipeDirection::Up,
    }
}

/// One full touchpad swipe, the same travel [`SwipeRecognizer`] calls a
/// workspace swipe on GNOME.
const TOUCHPAD_TRAVEL: f64 = 400.0;

/// A per-report motion cap: when a contact is reassigned the reported X/Y can
/// jump across the whole pad, which should not flick through ten windows.
const MAX_TOUCHPAD_STEP: f64 = TOUCHPAD_TRAVEL;

/// Turns absolute touchpad contact samples into the same phased, relative
/// motion the GNOME extension sends.
///
/// Windows Precision Touchpads report the contact count and absolute X/Y on
/// every report (HID digitizer page). `x_range`/`y_range` are the HID logical
/// ranges (`LogicalMax - LogicalMin`); movement across the whole pad is
/// scaled to [`TOUCHPAD_TRAVEL`], so the configured threshold and repeat_step
/// feel the same as under libinput. With no useful range the raw units are
/// passed through. Y is assumed to grow downwards, as HID digitizers report
/// it, which is also libinput's direction.
#[derive(Debug, Clone)]
pub struct TouchpadTracker {
    x_scale: f64,
    y_scale: f64,
    fingers: u32,
    last: Option<(f64, f64)>,
}

impl TouchpadTracker {
    pub fn new(x_range: f64, y_range: f64) -> Self {
        Self {
            x_scale: touchpad_scale(x_range),
            y_scale: touchpad_scale(y_range),
            fingers: 0,
            last: None,
        }
    }

    /// The contact count of the current (or last) gesture.
    pub fn fingers(&self) -> u32 {
        self.fingers
    }

    /// Feeds one sample. Returns the phase and the travel since the previous
    /// sample, in the units [`SwipeRecognizer`] expects.
    pub fn feed(&mut self, contacts: u32, x: i32, y: i32) -> Option<(Phase, f64, f64)> {
        if contacts == 0 {
            let was_active = self.last.take().is_some();
            self.fingers = 0;
            return was_active.then_some((Phase::End, 0.0, 0.0));
        }

        let point = (x as f64 * self.x_scale, y as f64 * self.y_scale);
        let previous = self.last.replace(point);
        let fingers_changed = contacts != self.fingers;
        self.fingers = contacts;

        match previous {
            None => Some((Phase::Begin, 0.0, 0.0)),
            // Fingers rarely land in one report. Starting over with the new
            // count lets the claim match, e.g., all three fingers; the lost
            // travel is a fraction of the threshold.
            Some(_) if fingers_changed => Some((Phase::Begin, 0.0, 0.0)),
            Some(last) => {
                let dx = (point.0 - last.0).clamp(-MAX_TOUCHPAD_STEP, MAX_TOUCHPAD_STEP);
                let dy = (point.1 - last.1).clamp(-MAX_TOUCHPAD_STEP, MAX_TOUCHPAD_STEP);
                (dx != 0.0 || dy != 0.0).then_some((Phase::Update, dx, dy))
            }
        }
    }
}

fn touchpad_scale(range: f64) -> f64 {
    if range > 0.0 && range.is_finite() {
        TOUCHPAD_TRAVEL / range
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SwipeDirection::*;

    fn feed(r: &mut SwipeRecognizer, deltas: &[(f64, f64)]) -> Vec<SwipeDirection> {
        deltas.iter().flat_map(|&(x, y)| r.update(x, y)).collect()
    }

    #[test]
    fn small_motion_does_nothing() {
        let mut r = SwipeRecognizer::new(100.0, 0.0);
        r.begin();
        assert!(feed(&mut r, &[(30.0, 0.0), (30.0, 5.0)]).is_empty());
    }

    #[test]
    fn single_step_per_gesture_without_repeat() {
        let mut r = SwipeRecognizer::new(100.0, 0.0);
        r.begin();
        assert_eq!(
            feed(&mut r, &[(-60.0, 0.0), (-60.0, 0.0), (-500.0, 0.0)]),
            vec![Left]
        );
        r.end();
        r.begin();
        assert_eq!(feed(&mut r, &[(0.0, 120.0)]), vec![Down]);
    }

    #[test]
    fn repeats_and_scrubs_back() {
        let mut r = SwipeRecognizer::new(100.0, 150.0);
        r.begin();
        // 100 to trigger, then 150 per step.
        let steps = feed(
            &mut r,
            &[(50.0, 0.0), (60.0, 0.0), (140.0, 0.0), (160.0, 0.0)],
        );
        assert_eq!(steps, vec![Right, Right, Right]);
        // Now move back: needs a full repeat_step of travel the other way.
        let back = feed(&mut r, &[(-100.0, 0.0), (-220.0, 0.0)]);
        assert_eq!(back, vec![Left, Left]);
    }

    #[test]
    fn axis_locks_after_first_step() {
        let mut r = SwipeRecognizer::new(100.0, 100.0);
        r.begin();
        assert_eq!(feed(&mut r, &[(0.0, -110.0)]), vec![Up]);
        // Sideways motion is ignored once locked to vertical.
        assert!(feed(&mut r, &[(300.0, 0.0)]).is_empty());
    }

    #[test]
    fn ignores_updates_outside_a_gesture() {
        let mut r = SwipeRecognizer::new(10.0, 10.0);
        assert!(r.update(100.0, 0.0).is_empty());
        r.begin();
        r.end();
        assert!(r.update(100.0, 0.0).is_empty());
    }

    #[test]
    fn huge_delta_is_capped() {
        let mut r = SwipeRecognizer::new(10.0, 10.0);
        r.begin();
        assert_eq!(r.update(1_000_000.0, 0.0).len(), 9);
    }

    #[test]
    fn touchpad_tracker_begins_updates_and_ends() {
        // A 1000-unit logical range becomes 0.4 gesture units per HID unit.
        let mut t = TouchpadTracker::new(1000.0, 1000.0);
        assert_eq!(t.feed(3, 500, 500), Some((Phase::Begin, 0.0, 0.0)));
        assert_eq!(t.fingers(), 3);

        let (phase, dx, dy) = t.feed(3, 600, 450).unwrap();
        assert_eq!(phase, Phase::Update);
        assert!((dx - 40.0).abs() < 1e-9);
        assert!((dy + 20.0).abs() < 1e-9);

        // No motion: nothing to feed to the recognizer.
        assert_eq!(t.feed(3, 600, 450), None);

        assert_eq!(t.feed(0, 0, 0), Some((Phase::End, 0.0, 0.0)));
        assert_eq!(t.fingers(), 0);
        assert_eq!(t.feed(0, 0, 0), None);
    }

    #[test]
    fn touchpad_tracker_restarts_when_fingers_land() {
        let mut t = TouchpadTracker::new(400.0, 400.0);
        assert_eq!(t.feed(1, 100, 100), Some((Phase::Begin, 0.0, 0.0)));
        // The other two fingers of a three-finger swipe arrive.
        assert_eq!(t.feed(3, 110, 100), Some((Phase::Begin, 0.0, 0.0)));
        assert_eq!(t.fingers(), 3);
        // No jump from the old centroid is reported.
        let (phase, dx, _) = t.feed(3, 120, 100).unwrap();
        assert_eq!(phase, Phase::Update);
        assert!((dx - 10.0).abs() < 1e-9);
    }

    #[test]
    fn touchpad_tracker_caps_a_reassigned_contact() {
        let mut t = TouchpadTracker::new(400.0, 400.0);
        t.feed(3, 0, 0);
        let (_, dx, dy) = t.feed(3, 1_000_000, -1_000_000).unwrap();
        assert_eq!(dx, TOUCHPAD_TRAVEL);
        assert_eq!(dy, -TOUCHPAD_TRAVEL);
    }

    #[test]
    fn touchpad_tracker_without_a_range_passes_raw_units() {
        let mut t = TouchpadTracker::new(0.0, 0.0);
        t.feed(3, 0, 0);
        let (_, dx, dy) = t.feed(3, 12, -7).unwrap();
        assert_eq!((dx, dy), (12.0, -7.0));
    }
}
