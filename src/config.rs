//! `config.toml`: cycling options, hotkeys and swipe bindings.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cycle::{CycleOptions, Order, Step, WorkspaceScope};
use crate::gesture::SwipeDirection;
use crate::hotkey::{Hotkey, Modifier, Mods, mods_of};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub order: Order,
    pub workspace: WorkspaceScope,
    pub include_minimized: bool,
    pub mru_session_ms: u64,
    pub gesture: GestureConfig,
    pub hud: HudConfig,
    #[serde(rename = "hotkey")]
    pub hotkeys: Vec<HotkeyBinding>,
    #[serde(rename = "swipe")]
    pub swipes: Vec<SwipeBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GestureConfig {
    /// Finger travel before a swipe counts (GNOME's full workspace swipe is 400).
    pub threshold: f64,
    /// Further travel per extra step while the fingers stay down; 0 = one
    /// step per swipe.
    pub repeat_step: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HudConfig {
    /// Show the window title and "App · 2 / 5" after each jump (on Windows,
    /// from the daemon only).
    pub enabled: bool,
    pub duration_ms: u64,
}

impl Default for HudConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            duration_ms: 700,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeyBinding {
    pub keys: String,
    pub action: Step,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwipeBinding {
    pub fingers: u32,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    pub direction: SwipeDirection,
    pub action: Step,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            threshold: 120.0,
            repeat_step: 220.0,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let swipe = |direction, action| SwipeBinding {
            fingers: 3,
            modifiers: vec![Modifier::Ctrl],
            direction,
            action,
        };
        Self {
            order: Order::Stable,
            workspace: WorkspaceScope::All,
            include_minimized: true,
            mru_session_ms: 1000,
            gesture: GestureConfig::default(),
            hud: HudConfig::default(),
            hotkeys: vec![
                HotkeyBinding {
                    keys: "Super+Alt+N".into(),
                    action: Step::Next,
                },
                HotkeyBinding {
                    keys: "Super+Alt+P".into(),
                    action: Step::Prev,
                },
            ],
            // Content follows the fingers, as with GNOME's workspace swipe:
            // swiping left brings in the next window.
            swipes: vec![
                swipe(SwipeDirection::Left, Step::Next),
                swipe(SwipeDirection::Right, Step::Prev),
            ],
        }
    }
}

/// A swipe claim: which (fingers, modifiers) combinations the GNOME
/// extension should take away from the shell and forward to us.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Claim {
    pub fingers: u32,
    pub mods: Mods,
}

impl Config {
    pub fn cycle_options(&self) -> CycleOptions {
        CycleOptions {
            order: self.order,
            workspace: self.workspace,
            include_minimized: self.include_minimized,
            mru_session_ms: self.mru_session_ms,
            ..CycleOptions::default()
        }
    }

    /// Checks everything that serde cannot, and returns the parsed hotkeys in
    /// binding order.
    pub fn validate(&self) -> Result<Vec<Hotkey>> {
        let mut parsed = Vec::with_capacity(self.hotkeys.len());
        for b in &self.hotkeys {
            let h = Hotkey::parse(&b.keys).map_err(anyhow::Error::msg)?;
            if parsed.contains(&h) {
                bail!("hotkey `{}` is bound twice", b.keys);
            }
            parsed.push(h);
        }
        for s in &self.swipes {
            if !(3..=5).contains(&s.fingers) {
                bail!(
                    "swipe with {} fingers: only 3 to 5 fingers can be used (one and two are pointer motion and scrolling)",
                    s.fingers
                );
            }
        }
        if self.gesture.threshold <= 0.0 || self.gesture.repeat_step < 0.0 {
            bail!("gesture.threshold must be > 0 and gesture.repeat_step >= 0");
        }
        Ok(parsed)
    }

    /// The distinct swipe claims, sorted.
    pub fn claims(&self) -> Vec<Claim> {
        let mut c: Vec<Claim> = self
            .swipes
            .iter()
            .map(|s| Claim {
                fingers: s.fingers,
                mods: mods_of(&s.modifiers),
            })
            .collect();
        c.sort();
        c.dedup();
        c
    }

    /// The action bound to a recognised swipe, if any.
    pub fn swipe_action(&self, claim: Claim, direction: SwipeDirection) -> Option<Step> {
        self.swipes
            .iter()
            .find(|s| {
                s.fingers == claim.fingers
                    && mods_of(&s.modifiers) == claim.mods
                    && s.direction == direction
            })
            .map(|s| s.action)
    }
}

pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("window-jump")
        .join("config.toml")
}

/// Loads `path`, or the defaults if it does not exist.
pub fn load(path: &Path) -> Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let cfg: Config =
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
            cfg.validate()
                .with_context(|| format!("checking {}", path.display()))?;
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The commented starter file written by `window-jump config init`.
pub const TEMPLATE: &str = r#"# window-jump configuration.
# Every setting is optional; what is shown here are the defaults.
# A running daemon picks up changes within about two seconds.

# How the windows of the focused app are ordered when cycling:
#   "stable"   fixed order (creation order); next/prev walk every window
#   "position" left to right on screen, then top to bottom
#   "mru"      most recently used, like Alt+Tab: a single press toggles
#              between the two most recent windows; pressing again while
#              holding the modifiers (or while a swipe goes on) digs into
#              older ones, and releasing settles on the window reached
order = "stable"

# "all": windows on every workspace (jumping may switch workspace)
# "current": only windows on the active workspace
workspace = "all"

include_minimized = true

# With order = "mru", where it cannot be told whether keys are still held
# (`window-jump next` run by another shortcut tool, a hotkey without
# modifiers): jumps closer together than this count as one session.
mru_session_ms = 1000

[gesture]
# Finger travel before a swipe counts. GNOME's full workspace swipe is 400.
threshold = 120.0
# While the fingers stay down, every further `repeat_step` of travel jumps
# again (moving back jumps back). 0 = one jump per swipe.
repeat_step = 220.0

[hud]
# After each jump, briefly show the window title and "App · 2 / 5" in the
# middle of the window's screen. On Windows only the daemon shows it.
enabled = true
duration_ms = 700

# Global hotkeys. Modifiers: Super (Win), Ctrl, Alt, Shift.
# Keys: A-Z, 0-9, F1-F24, Left, Right, Up, Down, Tab, Space, Return, Escape,
# Home, End, PageUp, PageDown, grave, minus, equal, bracketleft, bracketright,
# semicolon, apostrophe, comma, period, slash, backslash.
# Set `hotkey = []` (instead of the tables below) to bind none.
[[hotkey]]
keys = "Super+Alt+N"
action = "next"

[[hotkey]]
keys = "Super+Alt+P"
action = "prev"

# Touchpad swipes, held together with modifier keys: on GNOME via the shell
# extension, on Windows from a Precision Touchpad's raw HID input. On GNOME a
# claimed swipe is taken away from the shell, so Ctrl + three fingers no
# longer switches workspace while plain three fingers still does. On Windows,
# turn the built-in three/four-finger gestures off first.
# Avoid Super alone: releasing it opens the Activities overview (or Start).
# Set `swipe = []` to bind none.
[[swipe]]
fingers = 3
modifiers = ["ctrl"]
direction = "left"
action = "next"

[[swipe]]
fingers = 3
modifiers = ["ctrl"]
direction = "right"
action = "prev"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_equals_defaults() {
        let cfg: Config = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(cfg, Config::default());
        cfg.validate().unwrap();
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let cfg: Config = toml::from_str("order = \"mru\"\nhotkey = []\n").unwrap();
        assert_eq!(cfg.order, Order::Mru);
        assert!(cfg.hotkeys.is_empty());
        assert_eq!(cfg.swipes, Config::default().swipes);
    }

    #[test]
    fn unknown_keys_are_errors() {
        assert!(toml::from_str::<Config>("ordr = \"mru\"").is_err());
    }

    #[test]
    fn validate_catches_bad_values() {
        let mut cfg = Config::default();
        cfg.hotkeys.push(HotkeyBinding {
            keys: "super+alt+n".into(),
            action: Step::Prev,
        });
        assert!(cfg.validate().unwrap_err().to_string().contains("twice"));

        let mut cfg = Config::default();
        cfg.swipes[0].fingers = 2;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn claims_and_lookup() {
        let cfg = Config::default();
        let claims = cfg.claims();
        assert_eq!(
            claims,
            vec![Claim {
                fingers: 3,
                mods: Mods::CTRL
            }]
        );
        assert_eq!(
            cfg.swipe_action(claims[0], SwipeDirection::Left),
            Some(Step::Next)
        );
        assert_eq!(cfg.swipe_action(claims[0], SwipeDirection::Up), None);
    }
}
