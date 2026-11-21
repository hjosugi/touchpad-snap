//! The long-running part: reacts to hotkeys and swipes.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{Claim, Config};
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
    }
    Ok(pick)
}

/// Turns events into jumps. Kept apart from the platform loop so it can be
/// tested with a fake backend.
pub struct Handler {
    cfg: Config,
    recognizer: SwipeRecognizer,
    claim: Option<Claim>,
    state_path: PathBuf,
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
            verbose,
        }
    }

    pub fn handle(&mut self, backend: &dyn Backend, event: Event) {
        match event {
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
                let Some(claim) = self.claim else { return };
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

pub fn run(cfg: Config, verbose: bool) -> Result<()> {
    let hotkeys = cfg.validate()?;
    let mut handler = Handler::new(cfg.clone(), session::default_path(), verbose);
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

        // Default hotkey 0 is "next".
        h.handle(&fake, Event::Hotkey(0));
        assert_eq!(*fake.activated.borrow(), vec![2]);
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
}
