//! The long-running part: reacts to hotkeys and swipes, and picks up
//! changes to the config file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

use crate::config::{self, Action, Binding, Claim, Config, Source, Trigger};
use crate::control;
use crate::cycle::{self, Pick, Session, Step};
use crate::exec::{self, ActionContext};
use crate::gesture::{Phase, SwipeRecognizer};
use crate::hotkey::{Hotkey, Mods};
use crate::model::Snapshot;
use crate::platform::{self, Backend, Event};
use crate::session;
use crate::when::{AppContext, TriggerContext};

/// Which list a jump steps through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Windows,
    Apps,
}

/// Jumps once: list, pick, activate, remember.
pub fn jump(
    backend: &dyn Backend,
    cfg: &Config,
    step: Step,
    session: Session,
    state_path: &Path,
) -> Result<Pick> {
    jump_mode(backend, cfg, step, session, state_path, Mode::Windows)
}

/// Like [`jump`], but steps between apps instead of windows of one app.
pub fn jump_app(
    backend: &dyn Backend,
    cfg: &Config,
    step: Step,
    session: Session,
    state_path: &Path,
) -> Result<Pick> {
    jump_mode(backend, cfg, step, session, state_path, Mode::Apps)
}

fn jump_mode(
    backend: &dyn Backend,
    cfg: &Config,
    step: Step,
    session: Session,
    state_path: &Path,
    mode: Mode,
) -> Result<Pick> {
    let snapshot = backend.snapshot()?;
    let mut state = session::load(state_path);
    let opts = cfg.cycle_options();
    let now_ms = session::now_ms();
    let pick = match mode {
        Mode::Windows => cycle::pick(&snapshot, step, session, &opts, &mut state, now_ms),
        Mode::Apps => cycle::pick_app(&snapshot, step, session, &opts, &mut state, now_ms),
    };
    if let Pick::Jump(id) = pick {
        backend.activate(id)?;
        session::save(state_path, &state);
        let place = match mode {
            Mode::Windows => place_in_cycle(&snapshot, id, cfg),
            Mode::Apps => place_in_apps(&snapshot, id, cfg),
        };
        if cfg.hud.enabled
            && let Some((index, total)) = place
            && let Err(e) = backend.show_hud(id, index, total, cfg.hud.duration_ms)
        {
            // The jump itself worked; the overlay is decoration.
            eprintln!("touchpad-snap: could not show the overlay: {e:#}");
        }
    }
    Ok(pick)
}

/// 1-based position of `id` among its app's windows in cycling order, and
/// how many there are.
fn place_in_cycle(
    snapshot: &crate::model::Snapshot,
    id: u64,
    cfg: &Config,
) -> Option<(usize, usize)> {
    let target = snapshot.get(id)?;
    let list = cycle::candidates(snapshot, target, &cfg.cycle_options());
    let index = list.iter().position(|w| w.id == id)? + 1;
    Some((index, list.len()))
}

/// 1-based position of `id`'s app among the apps in cycling order, and how
/// many apps there are.
fn place_in_apps(
    snapshot: &crate::model::Snapshot,
    id: u64,
    cfg: &Config,
) -> Option<(usize, usize)> {
    let target = snapshot.get(id)?;
    let list = cycle::app_candidates(snapshot, target, &cfg.cycle_options());
    let key = target.app_key();
    let index = list.iter().position(|w| w.app_key() == key)? + 1;
    Some((index, list.len()))
}

/// Says which bindings were skipped and why, and how to finish a migration.
fn log_report(cfg: &Config) {
    for problem in &cfg.report().problems {
        eprintln!("touchpad-snap: config {}", problem.describe());
    }
    if let Source::LegacyToml(path) = &cfg.report().source {
        eprintln!(
            "touchpad-snap: using the legacy {} in memory; run `touchpad-snap config import` to write config.json",
            path.display()
        );
    }
}

/// One line for the verbose log, e.g. `Next` or `delay 100 ms + Prev`.
fn action_label(binding: &Binding) -> String {
    binding
        .actions
        .iter()
        .map(|action| match action {
            Action::Next => "Next".to_string(),
            Action::Prev => "Prev".to_string(),
            Action::AppNext => "AppNext".to_string(),
            Action::AppPrev => "AppPrev".to_string(),
            Action::Delay { ms } => format!("delay {ms} ms"),
            Action::Exec { command, shell } => {
                if *shell {
                    format!("exec (shell) {}", command.display())
                } else {
                    format!("exec {}", command.display())
                }
            }
            Action::Keys { keys } => format!("keys {keys}"),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Watches the config file by content: the daemon reads it on every tick
/// (it is tiny), which needs no platform file-watching API and does not
/// depend on mtime granularity.
struct ConfigWatch {
    path: PathBuf,
    /// What the file held when last looked at; `None` if it did not exist.
    seen: Option<Vec<u8>>,
}

impl ConfigWatch {
    fn new(path: PathBuf) -> Self {
        let seen = std::fs::read(&path).ok();
        Self { path, seen }
    }

    /// The new config if the file changed since the last call. An invalid
    /// file is reported once and otherwise ignored; a removed file means
    /// the defaults.
    fn poll(&mut self) -> Option<Config> {
        let now = std::fs::read(&self.path).ok();
        if now == self.seen {
            return None;
        }
        self.seen = now;
        self.load()
    }

    /// Re-reads the file even if it did not change; the tray's
    /// "Reload config" item.
    fn reload(&mut self) -> Option<Config> {
        self.seen = std::fs::read(&self.path).ok();
        self.load()
    }

    fn load(&self) -> Option<Config> {
        match config::load(&self.path) {
            Ok(cfg) => {
                eprintln!("touchpad-snap: reloaded {}", self.path.display());
                Some(cfg)
            }
            Err(e) => {
                eprintln!("touchpad-snap: keeping the previous config: {e:#}");
                None
            }
        }
    }
}

/// The focused window at the start of a trigger. `app_id` is what GNOME's
/// extension matched the claim against (protocol 4+); when absent, the
/// snapshot's focused window supplies all identifiers.
fn capture_context(backend: &dyn Backend, app_id: Option<&str>) -> TriggerContext {
    let app = backend
        .snapshot()
        .ok()
        .and_then(|snapshot| snapshot.focused().map(AppContext::from_window))
        .map(|app| app.with_app_id(app_id));
    TriggerContext::new(crate::platform::current_platform(), app)
}

/// Turns events into jumps. Kept apart from the platform loop so it can be
/// tested with a fake backend.
pub struct Handler {
    cfg: Config,
    recognizer: SwipeRecognizer,
    claim: Option<Claim>,
    /// The focused app and platform captured when the current trigger
    /// started; reused for the whole gesture.
    context: Option<TriggerContext>,
    /// Jumps made by the swipe in progress.
    swipe_jumps: usize,
    /// Non-repeat swipe bindings that already fired during this gesture.
    fired: HashSet<usize>,
    /// The window list when the current swipe began; `exec` actions use the
    /// focused window from there, not one snapshot per action.
    gesture_snapshot: Option<Snapshot>,
    /// The modifiers of the last (tracked) hotkey are still down.
    held: bool,
    state_path: PathBuf,
    watch: Option<ConfigWatch>,
    verbose: bool,
}

impl Handler {
    pub fn new(cfg: Config, state_path: PathBuf, verbose: bool) -> Self {
        let recognizer = SwipeRecognizer::new(cfg.gesture.threshold, cfg.gesture.repeat_step);
        Self {
            cfg,
            recognizer,
            claim: None,
            context: None,
            swipe_jumps: 0,
            fired: HashSet::new(),
            gesture_snapshot: None,
            held: false,
            state_path,
            watch: None,
            verbose,
        }
    }

    /// Reload the config from `path` when it changes (checked on
    /// [`Event::Tick`]).
    pub fn watch_config(mut self, path: PathBuf) -> Self {
        self.watch = Some(ConfigWatch::new(path));
        self
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Handles one event. Returns the new config when it was reloaded, so the
    /// platform can register the new hotkeys and swipes.
    pub fn handle(&mut self, backend: &dyn Backend, event: Event) -> Option<Config> {
        match event {
            Event::Tick => {
                let force = control::take_reload();
                let cfg = if force {
                    self.watch.as_mut()?.reload()?
                } else {
                    self.watch.as_mut()?.poll()?
                };
                return self.adopt(cfg);
            }
            Event::Reload => {
                let cfg = self.watch.as_mut()?.reload()?;
                return self.adopt(cfg);
            }
            Event::Hotkey { index, tracked } => {
                let session = match (tracked, self.held) {
                    (false, _) => Session::ByTime,
                    (true, true) => Session::Held,
                    (true, false) => Session::Fresh,
                };
                self.held = tracked;
                let context = capture_context(backend, None);
                if let Some(binding) = self.cfg.hotkey_binding_for(index, &context) {
                    let snapshot = if binding.has_exec() {
                        self.context_snapshot(backend)
                    } else {
                        None
                    };
                    if self.verbose {
                        let keys = match &binding.trigger {
                            Trigger::Hotkey { keys } => keys.as_str(),
                            Trigger::Swipe { .. } => "",
                        };
                        eprintln!(
                            "touchpad-snap: hotkey {} → {} ({})",
                            keys,
                            action_label(binding),
                            match session {
                                Session::ByTime => "session by time",
                                Session::Held => "keys still held",
                                Session::Fresh => "new press",
                            }
                        );
                    }
                    let held = match &binding.trigger {
                        Trigger::Hotkey { keys } => {
                            Hotkey::parse(keys).map(|h| h.mods).unwrap_or_default()
                        }
                        Trigger::Swipe { .. } => Mods::default(),
                    };
                    control::publish(serde_json::json!({
                        "event": "hotkey",
                        "binding": binding.name,
                        "trigger": binding.trigger.label(),
                        "actions": action_label(binding),
                    }));
                    self.run_binding(backend, binding, session, snapshot.as_ref(), held);
                }
            }
            Event::Released => {
                if self.held && self.verbose {
                    eprintln!("touchpad-snap: hotkey modifiers released");
                }
                self.held = false;
            }
            Event::Swipe {
                phase,
                claim,
                app_id,
                dx,
                dy,
            } => {
                if phase == Phase::Begin {
                    self.recognizer.begin();
                    self.claim = Some(claim);
                    self.context = Some(capture_context(backend, app_id.as_deref()));
                    self.swipe_jumps = 0;
                    self.fired.clear();
                    self.gesture_snapshot = self.context_snapshot(backend);
                }
                let claim = self.claim.clone()?;
                if matches!(phase, Phase::Begin | Phase::Update) {
                    let context = self.context.clone().unwrap_or_else(|| TriggerContext {
                        platform: crate::platform::current_platform(),
                        app: None,
                    });
                    for direction in self.recognizer.update(dx, dy) {
                        let Some((index, binding)) =
                            self.cfg
                                .swipe_binding_for(claim.clone(), direction, &context)
                        else {
                            continue;
                        };
                        if !binding.repeats() && !self.fired.insert(index) {
                            continue;
                        }
                        control::publish(serde_json::json!({
                            "event": "swipe",
                            "phase": format!("{phase:?}").to_lowercase(),
                            "fingers": claim.fingers,
                            "mods": claim.mods.0,
                            "direction": format!("{direction:?}").to_lowercase(),
                            "binding": binding.name,
                            "trigger": binding.trigger.label(),
                            "actions": action_label(binding),
                        }));
                        if self.verbose {
                            eprintln!(
                                "touchpad-snap: swipe {direction:?} with {} fingers → {}",
                                claim.fingers,
                                action_label(binding)
                            );
                        }
                        // One swipe is one session: keep going and it
                        // digs further back; lift the fingers to commit.
                        let session = if self.swipe_jumps == 0 {
                            Session::Fresh
                        } else {
                            Session::Held
                        };
                        self.swipe_jumps += 1;
                        self.run_binding(
                            backend,
                            binding,
                            session,
                            self.gesture_snapshot.as_ref(),
                            claim.mods,
                        );
                    }
                }
                if matches!(phase, Phase::End | Phase::Cancel) {
                    self.recognizer.end();
                    // A claimed swipe is not a key: leaving Alt or Super as
                    // the only thing the system saw before the release would
                    // open a menu, the overview or Start. The backend taps a
                    // harmless key if the modifier is still down.
                    if let Err(e) = backend.neutralize(claim.mods) {
                        eprintln!("touchpad-snap: could not neutralize the lone modifier: {e:#}");
                    }
                    self.claim = None;
                    self.context = None;
                    self.gesture_snapshot = None;
                }
            }
        }
        None
    }

    /// Takes over a reloaded config and resets gesture state that belonged to
    /// the old one.
    fn adopt(&mut self, cfg: Config) -> Option<Config> {
        log_report(&cfg);
        self.recognizer = SwipeRecognizer::new(cfg.gesture.threshold, cfg.gesture.repeat_step);
        self.claim = None;
        self.context = None;
        self.fired.clear();
        self.gesture_snapshot = None;
        control::config_changed(&cfg);
        self.cfg = cfg.clone();
        Some(cfg)
    }

    /// The window list for `exec` context, taken once when the trigger
    /// starts. A failure is not fatal: the command still runs, without the
    /// window variables.
    fn context_snapshot(&self, backend: &dyn Backend) -> Option<Snapshot> {
        match backend.snapshot() {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                if self.verbose {
                    eprintln!(
                        "touchpad-snap: could not read the focused window for the action context: {error:#}"
                    );
                }
                None
            }
        }
    }

    /// Runs a binding's action chain in order. `session` applies to the first
    /// window action; further ones in the same chain continue that session.
    ///
    /// `held` are the modifiers the trigger is held with (the swipe's, or the
    /// hotkey's): a `keys` action releases them while it types. `snapshot` is
    /// the focused window at trigger start, for `exec`.
    fn run_binding(
        &self,
        backend: &dyn Backend,
        binding: &Binding,
        session: Session,
        snapshot: Option<&Snapshot>,
        held: Mods,
    ) {
        run_actions(
            backend,
            &self.cfg,
            &binding.actions,
            &binding.trigger,
            session,
            snapshot,
            held,
            &self.state_path,
            self.verbose,
        );
    }
}

/// Runs an action chain with the session and context captured for a trigger.
///
/// Shared by the event loop and the control channel's `run` request. The
/// session state file is the only state; a caller running an action directly
/// starts with [`Session::ByTime`] like the CLI.
#[allow(clippy::too_many_arguments)]
pub fn run_actions(
    backend: &dyn Backend,
    cfg: &Config,
    actions: &[Action],
    trigger: &Trigger,
    session: Session,
    snapshot: Option<&Snapshot>,
    held: Mods,
    state_path: &Path,
    verbose: bool,
) {
    let mut session = session;
    for action in actions {
        match action {
            Action::Next => {
                let result = jump(backend, cfg, Step::Next, session, state_path);
                log_pick(&result, verbose);
                session = Session::Held;
            }
            Action::Prev => {
                let result = jump(backend, cfg, Step::Prev, session, state_path);
                log_pick(&result, verbose);
                session = Session::Held;
            }
            Action::AppNext => {
                let result = jump_app(backend, cfg, Step::Next, session, state_path);
                log_pick(&result, verbose);
                session = Session::Held;
            }
            Action::AppPrev => {
                let result = jump_app(backend, cfg, Step::Prev, session, state_path);
                log_pick(&result, verbose);
                session = Session::Held;
            }
            Action::Delay { ms } => {
                // Deliberately blocks the daemon thread for now; a delay is
                // short and this keeps the action chain in order.
                std::thread::sleep(Duration::from_millis(*ms));
            }
            Action::Exec { command, shell } => {
                let context = ActionContext::at(trigger, snapshot);
                exec::spawn(command, *shell, &context, verbose);
            }
            Action::Keys { keys } => match Hotkey::parse(keys) {
                Ok(hotkey) => {
                    if let Err(e) = backend.send_keys(hotkey, held) {
                        eprintln!("touchpad-snap: keys {keys}: {e:#}");
                    }
                }
                // Checked when the config is loaded; only a hand-built
                // Config gets here.
                Err(e) => eprintln!("touchpad-snap: keys: {e}"),
            },
        }
    }
}

fn log_pick(result: &Result<Pick>, verbose: bool) {
    match result {
        Ok(pick) => {
            if verbose {
                eprintln!("touchpad-snap: {pick:?}");
            }
        }
        Err(e) => eprintln!("touchpad-snap: {e:#}"),
    }
}

pub fn run(cfg: Config, config_path: PathBuf, verbose: bool) -> Result<()> {
    log_report(&cfg);
    let hotkeys = cfg.hotkeys();
    let state_path = session::default_path();

    let started = std::time::Instant::now();
    let (backend_name, protocol) = match platform::connect() {
        Ok(backend) => (backend.describe(), backend.protocol()),
        Err(e) => (format!("unavailable: {e:#}"), None),
    };
    let status = control::Status::build(
        &cfg,
        &platform::current_platform(),
        backend_name,
        protocol,
        started,
        Vec::new(),
    );
    control::start(status, config_path.clone(), state_path.clone(), verbose)?;

    let mut handler =
        Handler::new(cfg.clone(), state_path, verbose).watch_config(config_path.clone());
    platform::run_daemon(
        &cfg,
        &hotkeys,
        &config_path,
        verbose,
        &mut |backend, event| handler.handle(backend, event),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::hotkey::Mods;
    use crate::model::{Snapshot, Window};

    /// Three windows of one app; activating one moves focus to it and makes
    /// it the most recently used.
    struct Fake {
        snap: RefCell<Snapshot>,
        activated: RefCell<Vec<u64>>,
        huds: RefCell<Vec<(u64, usize, usize)>>,
        /// What `send_keys` was asked to type: the shortcut and the
        /// modifiers held for the trigger.
        typed: RefCell<Vec<(String, Mods)>>,
        /// What `neutralize` was asked to tap around.
        neutralized: RefCell<Vec<Mods>>,
    }

    impl Fake {
        fn new() -> Self {
            let windows = (1..=3)
                .map(|id| Window {
                    id,
                    app_id: "a".into(),
                    focused: id == 1,
                    mru: id as u32 - 1,
                    ..Default::default()
                })
                .collect();
            Self {
                snap: RefCell::new(Snapshot {
                    active_workspace: 0,
                    windows,
                }),
                activated: RefCell::new(vec![]),
                huds: RefCell::new(vec![]),
                typed: RefCell::new(vec![]),
                neutralized: RefCell::new(vec![]),
            }
        }
    }

    impl Backend for Fake {
        fn snapshot(&self) -> Result<Snapshot> {
            Ok(self.snap.borrow().clone())
        }
        fn activate(&self, id: u64) -> Result<()> {
            let mut snap = self.snap.borrow_mut();
            let old = snap.get(id).map(|w| w.mru).unwrap_or(0);
            for w in &mut snap.windows {
                w.focused = w.id == id;
                if w.id == id {
                    w.mru = 0;
                } else if w.mru < old {
                    w.mru += 1;
                }
            }
            self.activated.borrow_mut().push(id);
            Ok(())
        }
        fn show_hud(&self, id: u64, index: usize, total: usize, _ms: u64) -> Result<()> {
            self.huds.borrow_mut().push((id, index, total));
            Ok(())
        }
        fn send_keys(&self, keys: Hotkey, held: Mods) -> Result<()> {
            self.typed.borrow_mut().push((keys.to_string(), held));
            Ok(())
        }
        fn neutralize(&self, mods: Mods) -> Result<()> {
            self.neutralized.borrow_mut().push(mods);
            Ok(())
        }
        fn describe(&self) -> String {
            "fake".into()
        }
    }

    fn hotkey(index: usize) -> Event {
        Event::Hotkey {
            index,
            tracked: true,
        }
    }

    fn swipe(phase: Phase, dx: f64) -> Event {
        Event::Swipe {
            phase,
            claim: Claim::gesture(3, Mods::CTRL),
            app_id: None,
            dx,
            dy: 0.0,
        }
    }

    #[test]
    fn hotkeys_and_swipes_jump() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("touchpad-snap-test-{}", std::process::id()));
        let mut h = Handler::new(Config::default(), dir.join("state.json"), false);

        // Default hotkey 0 is "next", and the overlay names the target.
        h.handle(&fake, hotkey(0));
        assert_eq!(*fake.activated.borrow(), vec![2]);
        assert_eq!(*fake.huds.borrow(), vec![(2, 2, 3)]);
        // Unknown index: ignored.
        h.handle(&fake, hotkey(99));
        assert_eq!(fake.activated.borrow().len(), 1);

        // Ctrl + 3 fingers left = next; keep going for a second step.
        h.handle(&fake, swipe(Phase::Begin, 0.0));
        h.handle(&fake, swipe(Phase::Update, -130.0));
        h.handle(&fake, swipe(Phase::Update, -230.0));
        h.handle(&fake, swipe(Phase::End, 0.0));
        assert_eq!(*fake.activated.borrow(), vec![2, 3, 1]);

        // Updates after the end are ignored.
        h.handle(&fake, swipe(Phase::Update, -500.0));
        assert_eq!(fake.activated.borrow().len(), 3);

        // A claim with no binding for the direction does nothing.
        h.handle(&fake, swipe(Phase::Begin, 0.0));
        h.handle(
            &fake,
            Event::Swipe {
                phase: Phase::Update,
                claim: Claim::gesture(3, Mods::CTRL),
                app_id: None,
                dx: 0.0,
                dy: -300.0,
            },
        );
        assert_eq!(fake.activated.borrow().len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn app_actions_jump_between_apps_and_count_apps_in_the_hud() {
        let fake = Fake::new();
        {
            let mut snap = fake.snap.borrow_mut();
            snap.windows[0].app_id = "a".into();
            snap.windows[1].app_id = "a".into();
            snap.windows[2].app_id = "b".into();
            snap.windows.push(Window {
                id: 4,
                app_id: "c".into(),
                mru: 3,
                ..Default::default()
            });
        }
        let dir = std::env::temp_dir().join(format!("touchpad-snap-apps-{}", std::process::id()));
        let cfg = config_with(serde_json::json!([
            {"trigger": {"type": "hotkey", "keys": "Ctrl+Alt+J"},
             "actions": [{"type": "app.next"}]},
            {"trigger": {"type": "hotkey", "keys": "Ctrl+Alt+K"},
             "actions": [{"type": "app.prev"}]}
        ]));
        let mut h = Handler::new(cfg, dir.join("state.json"), false);

        // The apps are a, b, c in MRU order; the HUD counts apps only.
        h.handle(&fake, hotkey(0));
        assert_eq!(*fake.activated.borrow(), vec![3]);
        assert_eq!(*fake.huds.borrow(), vec![(3, 2, 3)]);

        // Keys still down: one further app, not a toggle back.
        h.handle(&fake, hotkey(0));
        assert_eq!(*fake.activated.borrow(), vec![3, 4]);
        assert_eq!(fake.huds.borrow()[1], (4, 3, 3));

        // Released and stepping back: the older app, b.
        h.handle(&fake, Event::Released);
        h.handle(&fake, hotkey(1));
        assert_eq!(*fake.activated.borrow(), vec![3, 4, 3]);
        assert_eq!(fake.huds.borrow()[2], (3, 2, 3));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn config_with(bindings: serde_json::Value) -> Config {
        let bindings: Vec<Binding> = serde_json::from_value(bindings).unwrap();
        Config {
            bindings,
            ..Config::default()
        }
    }

    #[test]
    fn keys_actions_type_the_shortcut_and_name_the_held_modifiers() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("touchpad-snap-keys-{}", std::process::id()));
        let cfg = config_with(serde_json::json!([
            {"trigger": {"type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "down"},
             "actions": [{"type": "keys", "keys": "Ctrl+Shift+Tab"}]},
            {"trigger": {"type": "hotkey", "keys": "Super+Alt+K"},
             "actions": [{"type": "keys", "keys": "Ctrl+N"}, {"type": "delay", "ms": 1}, {"type": "keys", "keys": "Escape"}]}
        ]));
        let mut h = Handler::new(cfg, dir.join("state.json"), false);
        let alt = |phase, dy| Event::Swipe {
            phase,
            claim: Claim::gesture(3, Mods::ALT),
            app_id: None,
            dx: 0.0,
            dy,
        };

        // A swipe down with Alt held: the keys go out, Alt is named as held,
        // and no window was jumped to.
        h.handle(&fake, alt(Phase::Begin, 0.0));
        h.handle(&fake, alt(Phase::Update, 130.0));
        h.handle(&fake, alt(Phase::End, 0.0));
        assert_eq!(
            *fake.typed.borrow(),
            vec![("Ctrl+Shift+tab".to_string(), Mods::ALT)]
        );
        assert!(fake.activated.borrow().is_empty());

        // A hotkey: its own modifiers are the held ones, and the chain runs
        // in order.
        fake.typed.borrow_mut().clear();
        h.handle(&fake, hotkey(0));
        assert_eq!(
            *fake.typed.borrow(),
            vec![
                ("Ctrl+N".to_string(), Mods::SUPER | Mods::ALT),
                ("escape".to_string(), Mods::SUPER | Mods::ALT),
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_claimed_swipe_asks_for_neutralizing_only_at_its_end() {
        let fake = Fake::new();
        let dir =
            std::env::temp_dir().join(format!("touchpad-snap-neutral-{}", std::process::id()));
        let mut h = Handler::new(Config::default(), dir.join("state.json"), false);
        let alt = |phase| Event::Swipe {
            phase,
            claim: Claim::gesture(3, Mods::ALT),
            app_id: None,
            dx: 0.0,
            dy: 0.0,
        };

        h.handle(&fake, alt(Phase::Begin));
        h.handle(&fake, alt(Phase::Update));
        assert!(fake.neutralized.borrow().is_empty());

        h.handle(&fake, alt(Phase::End));
        assert_eq!(*fake.neutralized.borrow(), vec![Mods::ALT]);

        // A cancelled swipe also leaves the modifier held.
        h.handle(&fake, alt(Phase::Begin));
        h.handle(&fake, alt(Phase::Cancel));
        assert_eq!(*fake.neutralized.borrow(), vec![Mods::ALT, Mods::ALT]);

        // Hotkeys press a real key, so they never need the tap.
        h.handle(&fake, hotkey(0));
        assert_eq!(fake.neutralized.borrow().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn alt_swipes_up_and_down_jump() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("touchpad-snap-alt-{}", std::process::id()));
        let mut h = Handler::new(Config::default(), dir.join("state.json"), false);
        let alt = |phase, dy| Event::Swipe {
            phase,
            claim: Claim::gesture(3, Mods::ALT),
            app_id: None,
            dx: 0.0,
            dy,
        };

        // Up (negative dy, as libinput reports it) = next: 1 → 2.
        h.handle(&fake, alt(Phase::Begin, 0.0));
        h.handle(&fake, alt(Phase::Update, -130.0));
        h.handle(&fake, alt(Phase::End, 0.0));
        assert_eq!(*fake.activated.borrow(), vec![2]);

        // Down = prev: back to 1, and on to 3 while the fingers stay down.
        h.handle(&fake, alt(Phase::Begin, 0.0));
        h.handle(&fake, alt(Phase::Update, 130.0));
        h.handle(&fake, alt(Phase::Update, 230.0));
        h.handle(&fake, alt(Phase::End, 0.0));
        assert_eq!(*fake.activated.borrow(), vec![2, 1, 3]);

        // Alt + left/right is claimed with it but bound to nothing.
        h.handle(&fake, alt(Phase::Begin, 0.0));
        h.handle(
            &fake,
            Event::Swipe {
                phase: Phase::Update,
                claim: Claim::gesture(3, Mods::ALT),
                app_id: None,
                dx: -300.0,
                dy: 0.0,
            },
        );
        assert_eq!(fake.activated.borrow().len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_app_is_captured_at_swipe_begin_for_the_whole_gesture() {
        let fake = Fake::new();
        {
            let mut snap = fake.snap.borrow_mut();
            for window in &mut snap.windows {
                window.app_id = "one".into();
                window.app_name = "One".into();
                window.wm_class = "one".into();
            }
        }
        let dir = std::env::temp_dir().join(format!("touchpad-snap-begin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"bindings":[
                {"trigger":{"type":"swipe","fingers":3,"modifiers":["ctrl"],"direction":"left"},
                 "when":{"app":["one"]},"actions":[{"type":"window.next"}]},
                {"trigger":{"type":"swipe","fingers":3,"modifiers":["ctrl"],"direction":"left"},
                 "when":{"app":["two"]},"actions":[{"type":"delay","ms":0}]}
            ]}"#,
        )
        .unwrap();
        let cfg = config::load(&path).unwrap();
        let mut h = Handler::new(cfg, dir.join("state.json"), false);

        let event = |phase, dx| Event::Swipe {
            phase,
            claim: Claim::gesture(3, Mods::CTRL),
            app_id: None,
            dx,
            dy: 0.0,
        };
        h.handle(&fake, event(Phase::Begin, 0.0));
        assert_eq!(
            h.context
                .as_ref()
                .and_then(|context| context.app.as_ref())
                .map(|app| app.app_id.as_str()),
            Some("one")
        );

        // Focus (and even the app id) changes mid-gesture; the binding still
        // comes from the app the gesture started in. (Keeping focus on the
        // same window isolates the app condition from the jump target.)
        {
            let mut snap = fake.snap.borrow_mut();
            for window in &mut snap.windows {
                window.app_id = "two".into();
                window.app_name = "Two".into();
                window.wm_class = "two".into();
            }
        }
        h.handle(&fake, event(Phase::Update, -130.0));
        assert_eq!(*fake.activated.borrow(), vec![2]);
        h.handle(&fake, event(Phase::End, 0.0));

        // A new gesture in the other app picks its binding instead (a no-op
        // delay, so focus would not change).
        for window in &mut fake.snap.borrow_mut().windows {
            window.focused = window.id == 2;
        }
        h.handle(&fake, event(Phase::Begin, 0.0));
        h.handle(&fake, event(Phase::Update, -130.0));
        h.handle(&fake, event(Phase::End, 0.0));
        assert_eq!(*fake.activated.borrow(), vec![2]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn held_hotkeys_and_swipes_are_sessions() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("touchpad-snap-held-{}", std::process::id()));
        let mut cfg = Config::default();
        cfg.cycle.order = crate::cycle::Order::Mru;
        let mut h = Handler::new(cfg, dir.join("state.json"), false);

        // Hold the modifiers and press twice: 1 → 2 → 3. Release, press
        // again: back to 1, where the session started.
        h.handle(&fake, hotkey(0));
        h.handle(&fake, hotkey(0));
        h.handle(&fake, Event::Released);
        h.handle(&fake, hotkey(0));
        assert_eq!(*fake.activated.borrow(), vec![2, 3, 1]);
        // A release on its own changes nothing.
        h.handle(&fake, Event::Released);
        h.handle(&fake, Event::Released);

        // One swipe that keeps going is one session: 1 → 3 → 2. The next
        // swipe starts a new one, which goes back to 1, where the last one
        // started.
        h.handle(&fake, swipe(Phase::Begin, 0.0));
        h.handle(&fake, swipe(Phase::Update, -130.0));
        h.handle(&fake, swipe(Phase::Update, -230.0));
        h.handle(&fake, swipe(Phase::End, 0.0));
        assert_eq!(fake.activated.borrow()[3..], [3, 2]);
        h.handle(&fake, swipe(Phase::Begin, 0.0));
        h.handle(&fake, swipe(Phase::Update, -130.0));
        h.handle(&fake, swipe(Phase::End, 0.0));
        assert_eq!(fake.activated.borrow()[5..], [1]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn exec_swipe_fires_once_unless_repeat() {
        fn lines(path: &std::path::Path) -> usize {
            std::fs::read_to_string(path)
                .map(|text| text.lines().count())
                .unwrap_or(0)
        }
        fn wait_for_lines(path: &std::path::Path, expected: usize) {
            for _ in 0..100 {
                if lines(path) >= expected {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!(
                "{} has {} lines, wanted {expected}",
                path.display(),
                lines(path)
            );
        }

        let fake = Fake::new();
        let dir =
            std::env::temp_dir().join(format!("touchpad-snap-exec-swipe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("marker");
        let config_for = |repeat: bool| {
            let path = dir.join(format!("config-{repeat}.json"));
            let repeat = if repeat { r#", "repeat": true"# } else { "" };
            std::fs::write(
                &path,
                format!(
                    r#"{{"bindings":[{{"trigger":{{"type":"swipe","fingers":3,"modifiers":["ctrl"],"direction":"left"}},"actions":[{{"type":"exec","command":["sh","-c","echo x >> {}"]}}]{repeat}}}]}}"#,
                    marker.display()
                ),
            )
            .unwrap();
            config::load(&path).unwrap()
        };

        let mut once = Handler::new(config_for(false), dir.join("state.json"), false);
        once.handle(&fake, swipe(Phase::Begin, 0.0));
        once.handle(&fake, swipe(Phase::Update, -130.0));
        once.handle(&fake, swipe(Phase::Update, -230.0));
        once.handle(&fake, swipe(Phase::End, 0.0));
        wait_for_lines(&marker, 1);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(lines(&marker), 1);

        std::fs::remove_file(&marker).unwrap();
        let mut repeat = Handler::new(config_for(true), dir.join("state.json"), false);
        repeat.handle(&fake, swipe(Phase::Begin, 0.0));
        repeat.handle(&fake, swipe(Phase::Update, -130.0));
        repeat.handle(&fake, swipe(Phase::Update, -230.0));
        repeat.handle(&fake, swipe(Phase::End, 0.0));
        wait_for_lines(&marker, 2);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn untracked_hotkeys_go_by_time() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("touchpad-snap-bytime-{}", std::process::id()));
        let mut cfg = Config::default();
        cfg.cycle.order = crate::cycle::Order::Mru;
        let mut h = Handler::new(cfg, dir.join("state.json"), false);
        let untracked = Event::Hotkey {
            index: 0,
            tracked: false,
        };
        // Two presses well within mru_session_ms: one session, 1 → 2 → 3.
        h.handle(&fake, untracked.clone());
        h.handle(&fake, untracked);
        assert_eq!(*fake.activated.borrow(), vec![2, 3]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn overlay_can_be_turned_off() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("touchpad-snap-hud-{}", std::process::id()));
        let mut cfg = Config::default();
        cfg.hud.enabled = false;
        jump(
            &fake,
            &cfg,
            Step::Prev,
            Session::ByTime,
            &dir.join("state.json"),
        )
        .unwrap();
        assert_eq!(*fake.activated.borrow(), vec![3]);
        assert!(fake.huds.borrow().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reloads_the_config_file_when_it_changes() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("touchpad-snap-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"bindings":[]}"#).unwrap();
        let cfg = config::load(&path).unwrap();
        let mut h = Handler::new(cfg, dir.join("state.json"), false).watch_config(path.clone());

        // Unchanged: nothing to do.
        assert_eq!(h.handle(&fake, Event::Tick), None);
        // ...but an explicit reload still re-reads it.
        assert_eq!(
            h.handle(&fake, Event::Reload).map(|c| c.hotkeys().len()),
            Some(0)
        );

        // A new binding is picked up and handed to the platform.
        std::fs::write(
            &path,
            r#"{"bindings":[{"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"actions":[{"type":"window.prev"}]}]}"#,
        )
        .unwrap();
        let new = h.handle(&fake, Event::Tick).expect("reloaded");
        assert_eq!(new.hotkeys().len(), 1);
        assert_eq!(
            h.config().hotkey_binding(0).unwrap().trigger,
            Trigger::Hotkey {
                keys: "Ctrl+Alt+J".into()
            }
        );
        // ...and used: hotkey 0 is now "prev" (1 → 3, wrapping backwards).
        h.handle(&fake, hotkey(0));
        assert_eq!(*fake.activated.borrow(), vec![3]);

        // A broken file keeps the previous config, and is not re-reported.
        std::fs::write(&path, r#"{"cycle":{"order":"sideways"}}"#).unwrap();
        assert_eq!(h.handle(&fake, Event::Tick), None);
        assert_eq!(h.handle(&fake, Event::Tick), None);
        assert_eq!(h.config().hotkeys().len(), 1);

        // Removing the file means the defaults.
        std::fs::remove_file(&path).unwrap();
        let defaults = h.handle(&fake, Event::Tick).expect("defaults");
        assert_eq!(defaults.bindings, Config::default().bindings);

        // Without a watched path, ticks never reload.
        let mut plain = Handler::new(Config::default(), dir.join("state.json"), false);
        assert_eq!(plain.handle(&fake, Event::Tick), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
