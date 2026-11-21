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
}
