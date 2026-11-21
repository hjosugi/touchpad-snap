//! The long-running part: reacts to hotkeys and swipes, and picks up
//! changes to the config file.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{self, Claim, Config};
use crate::cycle::{self, Pick, Step};
use crate::gesture::{Phase, SwipeRecognizer};
use crate::platform::{self, Backend, Event};
use crate::session;

/// Jumps once: list, pick, activate, remember.
pub fn jump(backend: &dyn Backend, cfg: &Config, step: Step, state_path: &Path) -> Result<Pick> {
    let snapshot = backend.snapshot()?;
    let mut state = session::load(state_path);
    let pick = cycle::pick(
        &snapshot,
        step,
        &cfg.cycle_options(),
        &mut state,
        session::now_ms(),
    );
    if let Pick::Jump(id) = pick {
        backend.activate(id)?;
        session::save(state_path, &state);
        if cfg.hud.enabled
            && let Some((index, total)) = place_in_cycle(&snapshot, id, cfg)
            && let Err(e) = backend.show_hud(id, index, total, cfg.hud.duration_ms)
        {
            // The jump itself worked; the overlay is decoration.
            eprintln!("window-jump: could not show the overlay: {e:#}");
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
        match config::load(&self.path) {
            Ok(cfg) => {
                eprintln!("window-jump: reloaded {}", self.path.display());
                Some(cfg)
            }
            Err(e) => {
                eprintln!("window-jump: keeping the previous config: {e:#}");
                None
            }
        }
    }
}

/// Turns events into jumps. Kept apart from the platform loop so it can be
/// tested with a fake backend.
pub struct Handler {
    cfg: Config,
    recognizer: SwipeRecognizer,
    claim: Option<Claim>,
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
                let cfg = self.watch.as_mut()?.poll()?;
                self.recognizer =
                    SwipeRecognizer::new(cfg.gesture.threshold, cfg.gesture.repeat_step);
                self.claim = None;
                self.cfg = cfg.clone();
                return Some(cfg);
            }
            Event::Hotkey(i) => {
                if let Some(binding) = self.cfg.hotkeys.get(i) {
                    let step = binding.action;
                    if self.verbose {
                        eprintln!("window-jump: hotkey {} → {step:?}", binding.keys);
                    }
                    self.run(backend, step);
                }
            }
            Event::Swipe {
                phase,
                claim,
                dx,
                dy,
            } => {
                if phase == Phase::Begin {
                    self.recognizer.begin();
                    self.claim = Some(claim);
                }
                let claim = self.claim?;
                if matches!(phase, Phase::Begin | Phase::Update) {
                    for direction in self.recognizer.update(dx, dy) {
                        if let Some(step) = self.cfg.swipe_action(claim, direction) {
                            if self.verbose {
                                eprintln!(
                                    "window-jump: swipe {direction:?} with {} fingers → {step:?}",
                                    claim.fingers
                                );
                            }
                            self.run(backend, step);
                        }
                    }
                }
                if matches!(phase, Phase::End | Phase::Cancel) {
                    self.recognizer.end();
                    self.claim = None;
                }
            }
        }
        None
    }

    fn run(&self, backend: &dyn Backend, step: Step) {
        match jump(backend, &self.cfg, step, &self.state_path) {
            Ok(pick) => {
                if self.verbose {
                    eprintln!("window-jump: {pick:?}");
                }
            }
            Err(e) => eprintln!("window-jump: {e:#}"),
        }
    }
}

pub fn run(cfg: Config, config_path: PathBuf, verbose: bool) -> Result<()> {
    let hotkeys = cfg.validate()?;
    let mut handler =
        Handler::new(cfg.clone(), session::default_path(), verbose).watch_config(config_path);
    platform::run_daemon(&cfg, &hotkeys, &mut |backend, event| {
        handler.handle(backend, event)
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::hotkey::Mods;
    use crate::model::{Snapshot, Window};

    /// Three windows of one app; activating one moves focus to it.
    struct Fake {
        snap: RefCell<Snapshot>,
        activated: RefCell<Vec<u64>>,
        huds: RefCell<Vec<(u64, usize, usize)>>,
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
            }
        }
    }

    impl Backend for Fake {
        fn snapshot(&self) -> Result<Snapshot> {
            Ok(self.snap.borrow().clone())
        }
        fn activate(&self, id: u64) -> Result<()> {
            for w in &mut self.snap.borrow_mut().windows {
                w.focused = w.id == id;
            }
            self.activated.borrow_mut().push(id);
            Ok(())
        }
        fn show_hud(&self, id: u64, index: usize, total: usize, _ms: u64) -> Result<()> {
            self.huds.borrow_mut().push((id, index, total));
            Ok(())
        }
        fn describe(&self) -> String {
            "fake".into()
        }
    }

    fn swipe(phase: Phase, dx: f64) -> Event {
        Event::Swipe {
            phase,
            claim: Claim {
                fingers: 3,
                mods: Mods::CTRL,
            },
            dx,
            dy: 0.0,
        }
    }

    #[test]
    fn hotkeys_and_swipes_jump() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("window-jump-test-{}", std::process::id()));
        let mut h = Handler::new(Config::default(), dir.join("state.json"), false);

        // Default hotkey 0 is "next", and the overlay names the target.
        h.handle(&fake, Event::Hotkey(0));
        assert_eq!(*fake.activated.borrow(), vec![2]);
        assert_eq!(*fake.huds.borrow(), vec![(2, 2, 3)]);
        // Unknown index: ignored.
        h.handle(&fake, Event::Hotkey(99));
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
                claim: Claim {
                    fingers: 3,
                    mods: Mods::CTRL,
                },
                dx: 0.0,
                dy: -300.0,
            },
        );
        assert_eq!(fake.activated.borrow().len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn overlay_can_be_turned_off() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("window-jump-hud-{}", std::process::id()));
        let mut cfg = Config::default();
        cfg.hud.enabled = false;
        jump(&fake, &cfg, Step::Prev, &dir.join("state.json")).unwrap();
        assert_eq!(*fake.activated.borrow(), vec![3]);
        assert!(fake.huds.borrow().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reloads_the_config_file_when_it_changes() {
        let fake = Fake::new();
        let dir = std::env::temp_dir().join(format!("window-jump-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "hotkey = []\n").unwrap();
        let cfg = config::load(&path).unwrap();
        let mut h = Handler::new(cfg, dir.join("state.json"), false).watch_config(path.clone());

        // Unchanged: nothing to do.
        assert_eq!(h.handle(&fake, Event::Tick), None);

        // A new binding is picked up and handed to the platform.
        std::fs::write(
            &path,
            "[[hotkey]]\nkeys = \"Ctrl+Alt+J\"\naction = \"prev\"\n",
        )
        .unwrap();
        let new = h.handle(&fake, Event::Tick).expect("reloaded");
        assert_eq!(new.hotkeys.len(), 1);
        assert_eq!(h.config().hotkeys[0].keys, "Ctrl+Alt+J");
        // ...and used: hotkey 0 is now "prev" (1 → 3, wrapping backwards).
        h.handle(&fake, Event::Hotkey(0));
        assert_eq!(*fake.activated.borrow(), vec![3]);

        // A broken file keeps the previous config, and is not re-reported.
        std::fs::write(&path, "order = \"sideways\"\n").unwrap();
        assert_eq!(h.handle(&fake, Event::Tick), None);
        assert_eq!(h.handle(&fake, Event::Tick), None);
        assert_eq!(h.config().hotkeys[0].keys, "Ctrl+Alt+J");

        // Removing the file means the defaults.
        std::fs::remove_file(&path).unwrap();
        let defaults = h.handle(&fake, Event::Tick).expect("defaults");
        assert_eq!(defaults, Config::default());

        // Without a watched path, ticks never reload.
        let mut plain = Handler::new(Config::default(), dir.join("state.json"), false);
        assert_eq!(plain.handle(&fake, Event::Tick), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
