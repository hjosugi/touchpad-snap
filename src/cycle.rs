//! Choosing which window of the focused app to jump to.
//!
//! Pure functions over a [`Snapshot`]; the backends only list and activate.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{Snapshot, Window};

/// Which way to move through the focused app's windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    Next,
    Prev,
}

impl std::str::FromStr for Step {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "next" => Ok(Step::Next),
            "prev" | "previous" => Ok(Step::Prev),
            other => Err(format!("unknown step `{other}` (expected next or prev)")),
        }
    }
}

/// How the windows of one app are ordered while cycling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    /// Fixed order (window creation order on GNOME): next/prev walk the
    /// whole list and always come back round to where they started.
    #[default]
    Stable,
    /// Left to right on screen, then top to bottom.
    Position,
    /// Most recently used first. A single jump toggles between the two most
    /// recent windows; repeats within one [`Session`] keep digging into older
    /// windows, like pressing Tab again while holding Alt.
    Mru,
}

/// Which windows are candidates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceScope {
    /// Windows on every workspace; jumping may switch workspace.
    #[default]
    All,
    /// Only windows on the active workspace (and sticky ones).
    Current,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CycleOptions {
    pub order: Order,
    pub workspace: WorkspaceScope,
    pub include_minimized: bool,
    pub mru_session_ms: u64,
    /// For this long after a jump, the jumped-to window counts as focused
    /// even if the backend does not report it yet. Lets fast repeats (a
    /// swipe that keeps going) step through windows instead of re-reading a
    /// focus change that is still in flight.
    pub focus_hint_ms: u64,
}

impl Default for CycleOptions {
    fn default() -> Self {
        Self {
            order: Order::Stable,
            workspace: WorkspaceScope::All,
            include_minimized: true,
            mru_session_ms: 1000,
            focus_hint_ms: 400,
        }
    }
}

/// Whether a jump continues the MRU session of the jump before it
/// (`Order::Mru` only).
///
/// A session is what Alt+Tab calls holding Alt: every press while it lasts
/// goes one window further back in the order frozen at its start, and only
/// the window it ends on counts as used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    /// Nobody can tell whether the keys are still down (`touchpad-snap next`
    /// run by another shortcut tool, a hotkey without modifiers): continue
    /// if the last jump was less than `mru_session_ms` ago.
    ByTime,
    /// The keys or fingers of the last jump are still down: continue.
    Held,
    /// The first jump of a new press: start a new session.
    Fresh,
}

/// State carried from one jump to the next.
///
/// Kept in a small file by the CLI and in memory by the daemon (see
/// [`crate::session`]), so the same rules apply to both.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CycleState {
    pub app_key: String,
    /// MRU order frozen at the start of the session (`Order::Mru` only),
    /// beginning with the window the session started from.
    pub order: Vec<u64>,
    /// The window that was current when the last jump was made.
    #[serde(default)]
    pub last_origin: u64,
    pub last_target: u64,
    /// Milliseconds since the Unix epoch.
    pub updated_ms: u64,
}

/// The outcome of [`pick`].
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    /// Activate this window.
    Jump(u64),
    /// The focused app has no other window to go to.
    Alone,
    /// Nothing is focused and nothing can be inferred.
    NoFocus,
}

/// Picks the window to activate, and updates `state` for the next call.
pub fn pick(
    snapshot: &Snapshot,
    step: Step,
    session: Session,
    opts: &CycleOptions,
    state: &mut Option<CycleState>,
    now_ms: u64,
) -> Pick {
    // Trust the last jump over the snapshot only while the snapshot still
    // shows the window we jumped from (or nothing): if focus has moved
    // somewhere else, the user did that and the snapshot wins.
    let focused_now = snapshot.focused().map(|w| w.id);
    let recent = state
        .as_ref()
        .filter(|s| now_ms.saturating_sub(s.updated_ms) <= opts.focus_hint_ms)
        .filter(|s| focused_now.is_none_or(|f| f == s.last_origin || f == s.last_target))
        .and_then(|s| snapshot.get(s.last_target));

    let Some(focused) = recent
        .or_else(|| snapshot.focused())
        .or_else(|| snapshot.windows.iter().min_by_key(|w| w.mru))
    else {
        return Pick::NoFocus;
    };

    let key = focused.app_key();
    let candidates = candidates(snapshot, focused, opts);
    if candidates.len() < 2 {
        return Pick::Alone;
    }

    let target = match opts.order {
        Order::Stable | Order::Position => {
            let idx = candidates.iter().position(|w| w.id == focused.id);
            let len = candidates.len();
            let next = match (idx, step) {
                (Some(i), Step::Next) => (i + 1) % len,
                (Some(i), Step::Prev) => (i + len - 1) % len,
                (None, Step::Next) => 0,
                (None, Step::Prev) => len - 1,
            };
            candidates[next].id
        }
        Order::Mru => {
            let live: Vec<u64> = candidates.iter().map(|w| w.id).collect();
            // The last session, if nothing has been focused since it ended.
            let previous = state
                .as_ref()
                .filter(|s| s.app_key == key && s.last_target == focused.id);
            let running = previous.filter(|s| match session {
                Session::Held => true,
                Session::Fresh => false,
                Session::ByTime => now_ms.saturating_sub(s.updated_ms) <= opts.mru_session_ms,
            });
            let mut order: Vec<u64> = match running {
                // Continue the frozen order, dropping windows that have gone
                // (new ones are appended below).
                Some(s) => s
                    .order
                    .iter()
                    .copied()
                    .filter(|id| live.contains(id))
                    .collect(),
                // A new session starts here. If the last one ended here too,
                // the window it started from comes next, not the ones it
                // passed on the way: as with Alt+Tab, those were never really
                // used, even though they had focus for a moment.
                None => {
                    let mut o = vec![focused.id];
                    if let Some(&origin) = previous.and_then(|s| s.order.first())
                        && origin != focused.id
                        && live.contains(&origin)
                    {
                        o.push(origin);
                    }
                    o
                }
            };
            for id in &live {
                if !order.contains(id) {
                    order.push(*id);
                }
            }
            let len = order.len();
            let i = order.iter().position(|&id| id == focused.id).unwrap_or(0);
            let next = match step {
                Step::Next => (i + 1) % len,
                Step::Prev => (i + len - 1) % len,
            };
            let target = order[next];
            *state = Some(CycleState {
                app_key: key,
                order,
                last_origin: focused.id,
                last_target: target,
                updated_ms: now_ms,
            });
            return Pick::Jump(target);
        }
    };

    *state = Some(CycleState {
        app_key: key,
        order: Vec::new(),
        last_origin: focused.id,
        last_target: target,
        updated_ms: now_ms,
    });
    Pick::Jump(target)
}

/// The focused window's app-mates that may be jumped to, in cycling order.
/// Always contains `focused` itself.
pub fn candidates<'a>(
    snapshot: &'a Snapshot,
    focused: &Window,
    opts: &CycleOptions,
) -> Vec<&'a Window> {
    let key = focused.app_key();
    let mut list: Vec<&Window> = snapshot
        .windows
        .iter()
        .filter(|w| w.id == focused.id || w.app_key() == key)
        .filter(|w| w.id == focused.id || opts.include_minimized || !w.minimized)
        .filter(|w| {
            w.id == focused.id
                || opts.workspace == WorkspaceScope::All
                || w.is_on_workspace(snapshot.active_workspace)
        })
        .collect();

    match opts.order {
        Order::Stable => list.sort_by_key(|w| w.id),
        Order::Position => list.sort_by_key(|w| (w.x, w.y, w.id)),
        Order::Mru => list.sort_by_key(|w| (w.mru, w.id)),
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: u64, app: &str, mru: u32) -> Window {
        Window {
            id,
            app_id: app.into(),
            mru,
            ..Default::default()
        }
    }

    fn snap(mut windows: Vec<Window>, focused: u64) -> Snapshot {
        for w in &mut windows {
            w.focused = w.id == focused;
        }
        Snapshot {
            active_workspace: 0,
            windows,
        }
    }

    fn jump(p: Pick) -> u64 {
        match p {
            Pick::Jump(id) => id,
            other => panic!("expected a jump, got {other:?}"),
        }
    }

    #[test]
    fn stable_walks_all_windows_and_wraps() {
        let opts = CycleOptions::default();
        let mut s = snap(
            vec![
                win(30, "a", 0),
                win(10, "a", 1),
                win(99, "b", 2),
                win(20, "a", 3),
            ],
            10,
        );
        let mut seen = vec![];
        let mut state = None;
        for t in 0..4 {
            // Far enough apart that the focus hint never kicks in.
            let id = jump(pick(
                &s,
                Step::Next,
                Session::ByTime,
                &opts,
                &mut state,
                t * 10_000,
            ));
            seen.push(id);
            for w in &mut s.windows {
                w.focused = w.id == id;
            }
        }
        assert_eq!(seen, vec![20, 30, 10, 20]);
    }

    #[test]
    fn stable_prev_goes_backwards() {
        let opts = CycleOptions::default();
        let s = snap(vec![win(1, "a", 1), win(2, "a", 0), win(3, "a", 2)], 2);
        assert_eq!(
            jump(pick(&s, Step::Prev, Session::ByTime, &opts, &mut None, 0)),
            1
        );
        let s = snap(vec![win(1, "a", 1), win(2, "a", 0), win(3, "a", 2)], 1);
        assert_eq!(
            jump(pick(&s, Step::Prev, Session::ByTime, &opts, &mut None, 0)),
            3
        );
    }

    #[test]
    fn other_apps_are_ignored() {
        let opts = CycleOptions::default();
        let s = snap(vec![win(1, "a", 0), win(2, "b", 1)], 1);
        assert_eq!(
            pick(&s, Step::Next, Session::ByTime, &opts, &mut None, 0),
            Pick::Alone
        );
    }

    #[test]
    fn no_windows_means_no_focus() {
        let opts = CycleOptions::default();
        let s = Snapshot::default();
        assert_eq!(
            pick(&s, Step::Next, Session::ByTime, &opts, &mut None, 0),
            Pick::NoFocus
        );
    }

    #[test]
    fn falls_back_to_most_recent_when_nothing_focused() {
        let opts = CycleOptions::default();
        let s = snap(vec![win(1, "a", 1), win(2, "a", 0)], 0);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::ByTime, &opts, &mut None, 0)),
            1
        );
    }

    #[test]
    fn minimized_and_other_workspaces_can_be_excluded() {
        let opts = CycleOptions {
            include_minimized: false,
            workspace: WorkspaceScope::Current,
            ..Default::default()
        };
        let mut a = win(1, "a", 0);
        a.workspace = 0;
        let mut b = win(2, "a", 1);
        b.minimized = true;
        let mut c = win(3, "a", 2);
        c.workspace = 1;
        let mut d = win(4, "a", 3);
        d.workspace = -1; // sticky
        let s = snap(vec![a, b, c, d], 1);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::ByTime, &opts, &mut None, 0)),
            4
        );
    }

    #[test]
    fn position_orders_left_to_right() {
        let opts = CycleOptions {
            order: Order::Position,
            ..Default::default()
        };
        let mut l = win(3, "a", 1);
        l.x = 0;
        let mut m = win(1, "a", 0);
        m.x = 500;
        let mut r = win(2, "a", 2);
        r.x = 900;
        let s = snap(vec![l, m, r], 1);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::ByTime, &opts, &mut None, 0)),
            2
        );
        assert_eq!(
            jump(pick(&s, Step::Prev, Session::ByTime, &opts, &mut None, 0)),
            3
        );
    }

    #[test]
    fn mru_toggles_on_isolated_jumps() {
        let opts = CycleOptions {
            order: Order::Mru,
            ..Default::default()
        };
        let mut state = None;
        let s = snap(vec![win(1, "a", 0), win(2, "a", 1), win(3, "a", 2)], 1);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::ByTime, &opts, &mut state, 0)),
            2
        );
        // Much later, focus is on 2 and the MRU order has changed.
        let s = snap(vec![win(1, "a", 1), win(2, "a", 0), win(3, "a", 2)], 2);
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::ByTime,
                &opts,
                &mut state,
                60_000
            )),
            1
        );
    }

    #[test]
    fn mru_session_digs_deeper_on_quick_repeats() {
        let opts = CycleOptions {
            order: Order::Mru,
            ..Default::default()
        };
        let mut state = None;
        let s = snap(vec![win(1, "a", 0), win(2, "a", 1), win(3, "a", 2)], 1);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::ByTime, &opts, &mut state, 0)),
            2
        );
        // The backend already reports 2 as focused and most recent, but the
        // session keeps the order frozen, so the next jump goes to 3.
        let s = snap(vec![win(1, "a", 1), win(2, "a", 0), win(3, "a", 2)], 2);
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::ByTime,
                &opts,
                &mut state,
                500
            )),
            3
        );
        assert_eq!(
            jump(pick(
                &s,
                Step::Prev,
                Session::ByTime,
                &opts,
                &mut state,
                900
            )),
            2
        );
    }

    /// The snapshot after `focused` was activated: it is focused and most
    /// recent, the others keep their relative MRU order.
    fn after(s: &Snapshot, focused: u64) -> Snapshot {
        let mut s = s.clone();
        let old = s.get(focused).map(|w| w.mru).unwrap_or(0);
        for w in &mut s.windows {
            w.focused = w.id == focused;
            if w.id == focused {
                w.mru = 0;
            } else if w.mru < old {
                w.mru += 1;
            }
        }
        s
    }

    fn mru_opts() -> CycleOptions {
        CycleOptions {
            order: Order::Mru,
            ..Default::default()
        }
    }

    #[test]
    fn held_session_outlasts_the_timeout() {
        let opts = mru_opts();
        let mut state = None;
        let s = snap(vec![win(1, "a", 0), win(2, "a", 1), win(3, "a", 2)], 1);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::Fresh, &opts, &mut state, 0)),
            2
        );
        // Five seconds later, keys still down: one further back, not a toggle.
        let s = after(&s, 2);
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::Held,
                &opts,
                &mut state,
                5_000
            )),
            3
        );
    }

    #[test]
    fn fresh_press_toggles_even_right_after_a_release() {
        let opts = mru_opts();
        let mut state = None;
        let s = snap(vec![win(1, "a", 0), win(2, "a", 1), win(3, "a", 2)], 1);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::Fresh, &opts, &mut state, 0)),
            2
        );
        let s = after(&s, 2);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::Fresh, &opts, &mut state, 100)),
            1
        );
    }

    #[test]
    fn windows_passed_in_a_session_do_not_count_as_used() {
        let opts = mru_opts();
        let mut state = None;
        let s = snap(vec![win(1, "a", 0), win(2, "a", 1), win(3, "a", 2)], 1);
        // Hold, press twice: 1 → 2 → 3, then release.
        assert_eq!(
            jump(pick(&s, Step::Next, Session::Fresh, &opts, &mut state, 0)),
            2
        );
        let s = after(&s, 2);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::Held, &opts, &mut state, 300)),
            3
        );
        // The shell now has 3, 2, 1 as its MRU order, but 2 was only passed
        // through: the next press goes back to 1.
        let s = after(&s, 3);
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::Fresh,
                &opts,
                &mut state,
                9_000
            )),
            1
        );
        // The same holds for a timed-out session of `touchpad-snap next`.
        let mut state = None;
        let s = snap(vec![win(1, "a", 0), win(2, "a", 1), win(3, "a", 2)], 1);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::ByTime, &opts, &mut state, 0)),
            2
        );
        let s = after(&s, 2);
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::ByTime,
                &opts,
                &mut state,
                500
            )),
            3
        );
        let s = after(&s, 3);
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::ByTime,
                &opts,
                &mut state,
                9_000
            )),
            1
        );
    }

    #[test]
    fn focusing_a_window_by_hand_ends_what_the_session_remembers() {
        let opts = mru_opts();
        let mut state = None;
        let s = snap(vec![win(1, "a", 0), win(2, "a", 1), win(3, "a", 2)], 1);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::Fresh, &opts, &mut state, 0)),
            2
        );
        let s = after(&s, 2);
        assert_eq!(
            jump(pick(&s, Step::Next, Session::Held, &opts, &mut state, 300)),
            3
        );
        // After the session, the user clicks 2: plain MRU order again.
        let s = after(&after(&s, 3), 2);
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::Fresh,
                &opts,
                &mut state,
                9_000
            )),
            3
        );
    }

    #[test]
    fn focus_hint_covers_a_lagging_backend() {
        let opts = CycleOptions::default();
        let s = snap(vec![win(1, "a", 0), win(2, "a", 1), win(3, "a", 2)], 1);
        let mut state = None;
        assert_eq!(
            jump(pick(&s, Step::Next, Session::ByTime, &opts, &mut state, 0)),
            2
        );
        // Same snapshot (focus change not visible yet), 100 ms later.
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::ByTime,
                &opts,
                &mut state,
                100
            )),
            3
        );
        // Long after: the snapshot is trusted again.
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::ByTime,
                &opts,
                &mut state,
                10_000
            )),
            2
        );
    }

    #[test]
    fn focus_hint_yields_to_a_manual_focus_change() {
        let opts = CycleOptions::default();
        let mut state = None;
        let s = snap(
            vec![
                win(1, "a", 0),
                win(2, "a", 1),
                win(3, "b", 2),
                win(4, "b", 3),
            ],
            1,
        );
        assert_eq!(
            jump(pick(&s, Step::Next, Session::ByTime, &opts, &mut state, 0)),
            2
        );
        // 100 ms later the user has clicked a window of app b: jump within b,
        // not from where the hint says app a is.
        let s = snap(
            vec![
                win(1, "a", 1),
                win(2, "a", 2),
                win(3, "b", 0),
                win(4, "b", 3),
            ],
            3,
        );
        assert_eq!(
            jump(pick(
                &s,
                Step::Next,
                Session::ByTime,
                &opts,
                &mut state,
                100
            )),
            4
        );
    }
}
