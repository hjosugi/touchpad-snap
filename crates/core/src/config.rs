//! `config.json`: bindings (trigger → actions), cycling options and the
//! settings shared by the daemon and the CLI.
//!
//! The file is strict inside objects this build knows, but one bad binding
//! never discards the others: [`Config::load_report`] returns the valid
//! bindings and a [`LoadReport`] naming every skipped one. Unknown trigger or
//! action types are warnings (a feature of a newer build); a malformed known
//! binding is an error (a typo to fix), but only in that binding.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cycle::{CycleOptions, Order, Step, WorkspaceScope};
use crate::gesture::SwipeDirection;
use crate::hotkey::{Hotkey, Modifier, Mods, mods_of};
use crate::when::{PlatformDescription, TriggerContext, When};

/// Where the committed schema lives; `config schema` prints the same JSON.
pub const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/hjosugi/touchpad-snap/main/schema/config.schema.json";
/// The only config file version this build understands.
pub const VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// What the binding is for; shown by `config check`, the tray and logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub trigger: Trigger,
    /// Where the binding applies: app, OS, desktop. Empty/absent means
    /// everywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<When>,
    /// Fire again every further `gesture.repeat_step`. Defaults to true when
    /// the chain contains a window cycle action, false otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<bool>,
    /// Run in order.
    pub actions: Vec<Action>,
    /// Index in the file's `bindings` array, for reports. Not part of the
    /// file or the schema.
    #[serde(skip)]
    #[schemars(skip)]
    pub index: usize,
}

impl Binding {
    pub fn repeats(&self) -> bool {
        self.repeat.unwrap_or_else(|| {
            self.actions.iter().any(|a| {
                matches!(
                    a,
                    Action::Next | Action::Prev | Action::AppNext | Action::AppPrev
                )
            })
        })
    }

    /// Whether the chain runs a program, and so needs the trigger window.
    pub fn has_exec(&self) -> bool {
        self.actions
            .iter()
            .any(|a| matches!(a, Action::Exec { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Trigger {
    #[serde(rename = "swipe")]
    Swipe {
        fingers: u32,
        #[serde(default)]
        modifiers: Vec<Modifier>,
        direction: SwipeDirection,
    },
    #[serde(rename = "hotkey")]
    Hotkey { keys: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Action {
    /// Focus the next window of the focused app.
    #[serde(rename = "window.next")]
    Next,
    /// Focus the previous window of the focused app.
    #[serde(rename = "window.prev")]
    Prev,
    /// Focus the most recently used window of the next app.
    #[serde(rename = "app.next")]
    AppNext,
    /// Focus the most recently used window of the previous app.
    #[serde(rename = "app.prev")]
    AppPrev,
    /// Wait before the next action in the chain.
    #[serde(rename = "delay")]
    Delay { ms: u64 },
    /// Run a program, detached from the daemon.
    #[serde(rename = "exec")]
    Exec {
        command: ExecCommand,
        /// Run a command string through the shell. Required for a string,
        /// rejected for an argument list.
        #[serde(default)]
        shell: bool,
    },
    /// Send a keyboard shortcut to the focused window, e.g. `"Ctrl+W"`. The
    /// modifiers the trigger itself is held with are released for it.
    #[serde(rename = "keys")]
    Keys { keys: String },
}

impl Action {
    /// Checks the combinations that serde cannot express.
    fn validate(&self) -> Result<(), String> {
        match self {
            Action::Keys { keys } => {
                Hotkey::parse(keys).map_err(|e| format!("action `keys`: {e}"))?;
            }
            Action::Exec { command, shell } => match command {
                ExecCommand::Args(args) => {
                    if *shell {
                        return Err(
                            "`exec`: a command list runs directly; remove \"shell\": true".into(),
                        );
                    }
                    if args.is_empty() {
                        return Err("`exec`: \"command\" must not be an empty list".into());
                    }
                    if args[0].trim().is_empty() {
                        return Err("`exec`: the program in \"command\" must not be empty".into());
                    }
                }
                ExecCommand::Shell(script) => {
                    if !*shell {
                        return Err(
                            "`exec`: a command string needs \"shell\": true; use an argument \
                                 list to run a program directly"
                                .into(),
                        );
                    }
                    if script.trim().is_empty() {
                        return Err("`exec`: \"command\" must not be empty".into());
                    }
                }
            },
            Action::Next
            | Action::Prev
            | Action::AppNext
            | Action::AppPrev
            | Action::Delay { .. } => {}
        }
        Ok(())
    }
}

/// The `command` of an [`Action::Exec`]: a program with its arguments, or a
/// single string for the shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ExecCommand {
    /// One string, run by `sh -c` (Unix) or `cmd /C` (Windows). Needs
    /// `"shell": true`.
    Shell(String),
    /// A program and its arguments, run directly without a shell. The first
    /// item is the program.
    Args(Vec<String>),
}

impl ExecCommand {
    /// How the command is shown in logs.
    pub fn display(&self) -> String {
        match self {
            ExecCommand::Shell(script) => script.clone(),
            ExecCommand::Args(args) => args.join(" "),
        }
    }

    /// The program and arguments when it runs directly.
    pub fn args(&self) -> Option<&[String]> {
        match self {
            ExecCommand::Args(args) => Some(args),
            ExecCommand::Shell(_) => None,
        }
    }
}

impl Trigger {
    /// A stable label for `TOUCHPAD_SNAP_TRIGGER`: `hotkey-Super+Alt+N` or
    /// `swipe-3-alt-up`.
    pub fn label(&self) -> String {
        match self {
            Trigger::Hotkey { keys } => format!("hotkey-{keys}"),
            Trigger::Swipe {
                fingers,
                modifiers,
                direction,
            } => {
                let direction = match direction {
                    SwipeDirection::Left => "left",
                    SwipeDirection::Right => "right",
                    SwipeDirection::Up => "up",
                    SwipeDirection::Down => "down",
                };
                let mods = modifiers
                    .iter()
                    .map(|m| match m {
                        Modifier::Shift => "shift",
                        Modifier::Ctrl => "ctrl",
                        Modifier::Alt => "alt",
                        Modifier::Super => "super",
                    })
                    .collect::<Vec<_>>()
                    .join("-");
                if mods.is_empty() {
                    format!("swipe-{fingers}-{direction}")
                } else {
                    format!("swipe-{fingers}-{mods}-{direction}")
                }
            }
        }
    }
}

impl From<Step> for Action {
    fn from(step: Step) -> Self {
        match step {
            Step::Next => Action::Next,
            Step::Prev => Action::Prev,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CycleConfig {
    pub order: Order,
    pub workspace: WorkspaceScope,
    pub include_minimized: bool,
    pub mru_session_ms: u64,
}

impl Default for CycleConfig {
    fn default() -> Self {
        Self {
            order: Order::Stable,
            workspace: WorkspaceScope::All,
            include_minimized: true,
            mru_session_ms: 1000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GestureConfig {
    /// Finger travel before a swipe counts (GNOME's full workspace swipe is 400).
    pub threshold: f64,
    /// Further travel per extra step while the fingers stay down; 0 = one
    /// step per swipe.
    pub repeat_step: f64,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            threshold: 120.0,
            repeat_step: 220.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(rename = "$schema", default = "default_schema")]
    pub schema: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub cycle: CycleConfig,
    #[serde(default)]
    pub gesture: GestureConfig,
    #[serde(default)]
    pub hud: HudConfig,
    #[serde(default = "default_bindings")]
    pub bindings: Vec<Binding>,
    #[serde(skip)]
    #[schemars(skip)]
    pub report: LoadReport,
}

fn default_true() -> bool {
    true
}

fn default_version() -> u32 {
    VERSION
}

fn default_schema() -> String {
    SCHEMA_URL.into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: SCHEMA_URL.into(),
            version: VERSION,
            cycle: CycleConfig::default(),
            gesture: GestureConfig::default(),
            hud: HudConfig::default(),
            bindings: default_bindings(),
            report: LoadReport::default(),
        }
    }
}

/// The bindings that make up today's default behaviour.
fn default_bindings() -> Vec<Binding> {
    let hotkey = |name: &str, keys: &str, action: Action| Binding {
        name: Some(name.into()),
        enabled: true,
        trigger: Trigger::Hotkey { keys: keys.into() },
        when: None,
        repeat: None,
        actions: vec![action],
        index: 0,
    };
    let swipe = |name: &str, modifier, direction, action| Binding {
        name: Some(name.into()),
        enabled: true,
        trigger: Trigger::Swipe {
            fingers: 3,
            modifiers: vec![modifier],
            direction,
        },
        when: None,
        repeat: None,
        actions: vec![action],
        index: 0,
    };
    // Content follows the fingers, as with GNOME's workspace swipe: swiping
    // left (or up) brings in the next window. Two modifiers, one per axis, so
    // that either hand position works: Ctrl for the axis GNOME switches
    // workspace on, Alt for the one it opens the overview on.
    let mut bindings = vec![
        hotkey("Next window of this app", "Super+Alt+N", Action::Next),
        hotkey("Previous window of this app", "Super+Alt+P", Action::Prev),
        swipe(
            "Next window (Ctrl + swipe left)",
            Modifier::Ctrl,
            SwipeDirection::Left,
            Action::Next,
        ),
        swipe(
            "Previous window (Ctrl + swipe right)",
            Modifier::Ctrl,
            SwipeDirection::Right,
            Action::Prev,
        ),
        swipe(
            "Next window (Alt + swipe up)",
            Modifier::Alt,
            SwipeDirection::Up,
            Action::Next,
        ),
        swipe(
            "Previous window (Alt + swipe down)",
            Modifier::Alt,
            SwipeDirection::Down,
            Action::Prev,
        ),
    ];
    for (index, binding) in bindings.iter_mut().enumerate() {
        binding.index = index;
    }
    bindings
}

fn default_binding_values() -> Vec<Value> {
    default_bindings()
        .into_iter()
        .map(|binding| serde_json::to_value(binding).expect("a binding serializes"))
        .collect()
}

/// A swipe claim: which (fingers, modifiers) combinations the GNOME
/// extension should take away from the shell and forward to us.
///
/// `apps` narrows the claim to the focused app on GNOME (empty means every
/// app). The daemon still chooses the binding by `when`, so another backend
/// can use a claim for its finger/modifier filter alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Claim {
    pub fingers: u32,
    pub mods: Mods,
    pub apps: Vec<String>,
}

impl Claim {
    /// A claim that applies everywhere.
    pub fn gesture(fingers: u32, mods: Mods) -> Self {
        Self {
            fingers,
            mods,
            apps: Vec::new(),
        }
    }

    pub fn matches_gesture(&self, fingers: u32, mods: Mods) -> bool {
        self.fingers == fingers && self.mods == mods
    }
}

/// Where a loaded config came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Defaults,
    Json(PathBuf),
    LegacyToml(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Warning => f.write_str("warning"),
            Severity::Error => f.write_str("error"),
        }
    }
}

/// One binding that was skipped, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingProblem {
    /// Index into the file's `bindings` array.
    pub index: usize,
    pub name: Option<String>,
    pub severity: Severity,
    pub message: String,
}

impl BindingProblem {
    /// `bindings[2] (name): warning: ...`
    pub fn describe(&self) -> String {
        let name = self
            .name
            .as_deref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        format!(
            "bindings[{}]{}: {}: {}",
            self.index, name, self.severity, self.message
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadReport {
    pub source: Source,
    pub problems: Vec<BindingProblem>,
}

impl LoadReport {
    pub fn has_errors(&self) -> bool {
        self.problems.iter().any(|p| p.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &BindingProblem> {
        self.problems
            .iter()
            .filter(|p| p.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &BindingProblem> {
        self.problems
            .iter()
            .filter(|p| p.severity == Severity::Warning)
    }
}

/// What the file on disk looks like before the bindings are decoded one by
/// one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(rename = "$schema", default)]
    schema: Option<String>,
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    cycle: CycleConfig,
    #[serde(default)]
    gesture: GestureConfig,
    #[serde(default)]
    hud: HudConfig,
    #[serde(default = "default_binding_values")]
    bindings: Vec<Value>,
}

/// The old `config.toml` shape. Its fields use the same names and defaults as
/// version 0.5, so an upgrade behaves as before until `config import` runs.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LegacyConfig {
    order: Order,
    workspace: WorkspaceScope,
    include_minimized: bool,
    mru_session_ms: u64,
    gesture: GestureConfig,
    hud: HudConfig,
    #[serde(rename = "hotkey")]
    hotkeys: Vec<LegacyHotkey>,
    #[serde(rename = "swipe")]
    swipes: Vec<LegacySwipe>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyHotkey {
    keys: String,
    action: Step,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySwipe {
    fingers: u32,
    #[serde(default)]
    modifiers: Vec<Modifier>,
    direction: SwipeDirection,
    action: Step,
}

impl Default for LegacyConfig {
    fn default() -> Self {
        Self {
            order: Order::default(),
            workspace: WorkspaceScope::default(),
            include_minimized: true,
            mru_session_ms: 1000,
            gesture: GestureConfig::default(),
            hud: HudConfig::default(),
            hotkeys: vec![
                LegacyHotkey {
                    keys: "Super+Alt+N".into(),
                    action: Step::Next,
                },
                LegacyHotkey {
                    keys: "Super+Alt+P".into(),
                    action: Step::Prev,
                },
            ],
            swipes: vec![
                LegacySwipe {
                    fingers: 3,
                    modifiers: vec![Modifier::Ctrl],
                    direction: SwipeDirection::Left,
                    action: Step::Next,
                },
                LegacySwipe {
                    fingers: 3,
                    modifiers: vec![Modifier::Ctrl],
                    direction: SwipeDirection::Right,
                    action: Step::Prev,
                },
                LegacySwipe {
                    fingers: 3,
                    modifiers: vec![Modifier::Alt],
                    direction: SwipeDirection::Up,
                    action: Step::Next,
                },
                LegacySwipe {
                    fingers: 3,
                    modifiers: vec![Modifier::Alt],
                    direction: SwipeDirection::Down,
                    action: Step::Prev,
                },
            ],
        }
    }
}

impl LegacyConfig {
    fn into_parts(self) -> (CycleConfig, GestureConfig, HudConfig, Vec<(usize, Binding)>) {
        let mut candidates = Vec::new();
        for hotkey in self.hotkeys {
            let index = candidates.len();
            candidates.push((
                index,
                Binding {
                    name: None,
                    enabled: true,
                    trigger: Trigger::Hotkey { keys: hotkey.keys },
                    when: None,
                    repeat: None,
                    actions: vec![hotkey.action.into()],
                    index,
                },
            ));
        }
        for swipe in self.swipes {
            let index = candidates.len();
            candidates.push((
                index,
                Binding {
                    name: None,
                    enabled: true,
                    trigger: Trigger::Swipe {
                        fingers: swipe.fingers,
                        modifiers: swipe.modifiers,
                        direction: swipe.direction,
                    },
                    when: None,
                    repeat: None,
                    actions: vec![swipe.action.into()],
                    index,
                },
            ));
        }
        (
            CycleConfig {
                order: self.order,
                workspace: self.workspace,
                include_minimized: self.include_minimized,
                mru_session_ms: self.mru_session_ms,
            },
            self.gesture,
            self.hud,
            candidates,
        )
    }
}

impl Config {
    /// Loads `path`, or the default config when it does not exist. If only the
    /// old `config.toml` exists next to it, that file is read in memory and is
    /// reported as [`Source::LegacyToml`] so callers can suggest
    /// `touchpad-snap config import`.
    ///
    /// Only problems with the file as a whole (syntax, unknown top-level
    /// fields, an unsupported version, invalid gesture numbers) are errors.
    /// Problems inside a binding are collected in the returned config's
    /// [`LoadReport`]; the other bindings still load.
    pub fn load_report(path: &Path) -> Result<Config> {
        // An explicit `--config something.toml` still means the legacy file.
        if path.extension().and_then(|ext| ext.to_str()) == Some("toml") && path.exists() {
            let mut cfg = load_legacy(path)?;
            cfg.report.source = Source::LegacyToml(path.to_path_buf());
            return Ok(cfg);
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let raw: RawConfig = serde_json::from_str(&text)
                    .with_context(|| format!("parsing {}", path.display()))?;
                if raw.version != VERSION {
                    bail!(
                        "{}: unsupported config version {} (this build reads version {})",
                        path.display(),
                        raw.version,
                        VERSION
                    );
                }
                check_gesture(&raw.gesture)
                    .with_context(|| format!("checking {}", path.display()))?;
                let mut report = LoadReport {
                    source: Source::Json(path.to_path_buf()),
                    problems: Vec::new(),
                };
                let candidates = decode_bindings(raw.bindings, &mut report);
                Ok(Config {
                    schema: raw.schema.unwrap_or_else(|| SCHEMA_URL.into()),
                    version: raw.version,
                    cycle: raw.cycle,
                    gesture: raw.gesture,
                    hud: raw.hud,
                    bindings: compile_bindings(candidates, &mut report),
                    report,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let legacy = legacy_path(path);
                if legacy.exists() {
                    let mut cfg = load_legacy(&legacy)?;
                    cfg.report.source = Source::LegacyToml(legacy);
                    Ok(cfg)
                } else {
                    Ok(Config::default())
                }
            }
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The problems found the last time this config was loaded or validated.
    pub fn validate_report(&self) -> &LoadReport {
        &self.report
    }

    pub fn report(&self) -> &LoadReport {
        &self.report
    }

    pub fn cycle_options(&self) -> CycleOptions {
        CycleOptions {
            order: self.cycle.order,
            workspace: self.cycle.workspace,
            include_minimized: self.cycle.include_minimized,
            mru_session_ms: self.cycle.mru_session_ms,
            ..CycleOptions::default()
        }
    }

    /// The enabled hotkeys, in binding order. A key bound by several
    /// conditional bindings appears once. Their indices are what the platform
    /// backends send back in [`crate::platform::Event::Hotkey`].
    pub fn hotkeys(&self) -> Vec<Hotkey> {
        let mut hotkeys = Vec::new();
        for binding in self.bindings.iter().filter(|b| b.enabled) {
            if let Trigger::Hotkey { keys } = &binding.trigger
                && let Ok(hotkey) = Hotkey::parse(keys)
                && !hotkeys.contains(&hotkey)
            {
                hotkeys.push(hotkey);
            }
        }
        hotkeys
    }

    fn hotkey_candidates(&self, hotkey: Hotkey) -> Vec<(usize, &Binding)> {
        self.bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.enabled
                    && matches!(&b.trigger, Trigger::Hotkey { keys } if Hotkey::parse(keys).ok() == Some(hotkey))
            })
            .collect()
    }

    /// The binding behind hotkey `index` from [`Config::hotkeys`], ignoring
    /// `when` conditions. Used where no window is available (portal
    /// descriptions).
    pub fn hotkey_binding(&self, index: usize) -> Option<&Binding> {
        let hotkey = self.hotkeys().into_iter().nth(index)?;
        self.hotkey_candidates(hotkey)
            .into_iter()
            .map(|(_, binding)| binding)
            .next()
    }

    /// The binding behind hotkey `index` that applies in `context`:
    /// conditional bindings that match beat unconditional ones, and ties go
    /// to the first in the file.
    pub fn hotkey_binding_for(&self, index: usize, context: &TriggerContext) -> Option<&Binding> {
        let hotkey = self.hotkeys().into_iter().nth(index)?;
        choose(self.hotkey_candidates(hotkey), context).map(|(_, binding)| binding)
    }

    /// The distinct swipe claims, sorted. Claims include their app filter for
    /// GNOME; other backends only use fingers and modifiers.
    pub fn claims(&self) -> Vec<Claim> {
        self.claims_for(&PlatformDescription::current())
    }

    /// The claims that can fire on `platform`.
    pub fn claims_for(&self, platform: &PlatformDescription) -> Vec<Claim> {
        // (fingers, mods, app patterns, everywhere?)
        let mut groups: Vec<(u32, Mods, Vec<String>, bool)> = Vec::new();
        for binding in self.bindings.iter().filter(|b| b.enabled) {
            let Trigger::Swipe {
                fingers, modifiers, ..
            } = &binding.trigger
            else {
                continue;
            };
            let fingers = *fingers;
            let mods = mods_of(modifiers);
            match &binding.when {
                Some(when) if !when.matches_platform(platform) => continue,
                Some(when) => {
                    let apps = &when.app;
                    let everywhere = apps.is_empty();
                    let entry = match groups
                        .iter_mut()
                        .find(|(f, m, _, _)| *f == fingers && *m == mods)
                    {
                        Some(entry) => entry,
                        None => {
                            groups.push((fingers, mods, Vec::new(), false));
                            groups.last_mut().expect("just pushed")
                        }
                    };
                    if everywhere {
                        entry.3 = true;
                    } else {
                        for app in apps {
                            if !entry.2.iter().any(|seen| seen.eq_ignore_ascii_case(app)) {
                                entry.2.push(app.clone());
                            }
                        }
                    }
                }
                None => {
                    let entry = match groups
                        .iter_mut()
                        .find(|(f, m, _, _)| *f == fingers && *m == mods)
                    {
                        Some(entry) => entry,
                        None => {
                            groups.push((fingers, mods, Vec::new(), false));
                            groups.last_mut().expect("just pushed")
                        }
                    };
                    entry.3 = true;
                }
            }
        }
        let mut claims: Vec<Claim> = groups
            .into_iter()
            .map(|(fingers, mods, apps, everywhere)| Claim {
                fingers,
                mods,
                apps: if everywhere { Vec::new() } else { apps },
            })
            .collect();
        claims.sort();
        claims.dedup();
        claims
    }

    /// The enabled bindings for a recognised swipe, with the index needed to
    /// remember which non-repeat ones already fired during this gesture.
    pub fn swipe_bindings_indexed(
        &self,
        claim: Claim,
        direction: SwipeDirection,
    ) -> Vec<(usize, &Binding)> {
        self.bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.enabled
                    && match &b.trigger {
                        Trigger::Swipe {
                            fingers,
                            modifiers,
                            direction: bound,
                        } => {
                            claim.matches_gesture(*fingers, mods_of(modifiers))
                                && *bound == direction
                        }
                        Trigger::Hotkey { .. } => false,
                    }
            })
            .collect()
    }

    /// The enabled bindings for a recognised swipe.
    pub fn swipe_bindings(&self, claim: Claim, direction: SwipeDirection) -> Vec<&Binding> {
        self.swipe_bindings_indexed(claim, direction)
            .into_iter()
            .map(|(_, binding)| binding)
            .collect()
    }

    /// `exec` commands that will not be found when the binding fires. Shell
    /// strings cannot be checked and only direct argument lists are reported.
    pub fn missing_exec_commands(&self) -> Vec<BindingProblem> {
        let mut problems = Vec::new();
        for (index, binding) in self
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.enabled)
        {
            for action in &binding.actions {
                let Action::Exec { command, .. } = action else {
                    continue;
                };
                let Some(program) = command.args().and_then(|args| args.first()) else {
                    continue;
                };
                if !executable_on_path(program) {
                    problems.push(BindingProblem {
                        index,
                        name: binding.name.clone(),
                        severity: Severity::Warning,
                        message: format!(
                            "exec command `{program}` was not found on PATH (or as a file); \
                             the binding will fail when it fires"
                        ),
                    });
                }
            }
        }
        problems
    }

    /// Whether any action runs a string through the shell, which
    /// [`Config::missing_exec_commands`] cannot check.
    pub fn has_shell_exec(&self) -> bool {
        self.bindings
            .iter()
            .filter(|binding| binding.enabled)
            .any(|binding| {
                binding.actions.iter().any(|action| {
                    matches!(
                        action,
                        Action::Exec {
                            command: ExecCommand::Shell(_),
                            ..
                        }
                    )
                })
            })
    }

    /// The one binding that fires for this swipe in `context`.
    pub fn swipe_binding_for(
        &self,
        claim: Claim,
        direction: SwipeDirection,
        context: &TriggerContext,
    ) -> Option<(usize, &Binding)> {
        choose(self.swipe_bindings_indexed(claim, direction), context)
    }

    /// Enabled bindings whose `when` cannot match this machine.
    pub fn inactive_on(&self, platform: &PlatformDescription) -> Vec<InactiveBinding> {
        self.bindings
            .iter()
            .filter(|b| b.enabled)
            .filter_map(|binding| {
                let when = binding.when.as_ref()?;
                let reason = when.inactive_reason(platform)?;
                Some(InactiveBinding {
                    index: binding.index,
                    name: binding.name.clone(),
                    reason,
                })
            })
            .collect()
    }
}

/// Whether `program` names an executable file, searching `PATH` for a bare
/// name (with `PATHEXT` on Windows).
pub fn executable_on_path(program: &str) -> bool {
    if program.trim().is_empty() {
        return false;
    }
    let path = Path::new(program);
    let has_separator = program.contains('/') || program.contains('\\');
    if path.is_absolute() || has_separator {
        return is_executable(path);
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions = executable_extensions();
    std::env::split_paths(&paths).any(|dir| {
        let direct = dir.join(program);
        is_executable(&direct)
            || extensions
                .iter()
                .any(|extension| is_executable(&dir.join(format!("{program}{extension}"))))
    })
}

#[cfg(windows)]
fn executable_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(not(windows))]
fn executable_extensions() -> Vec<String> {
    vec![String::new()]
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// A binding that can never fire on the machine running the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InactiveBinding {
    pub index: usize,
    pub name: Option<String>,
    pub reason: String,
}

impl InactiveBinding {
    pub fn describe(&self) -> String {
        let name = self
            .name
            .as_deref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        format!(
            "bindings[{}]{}: inactive on this machine: {}",
            self.index, name, self.reason
        )
    }
}

/// Picks the binding for a trigger: the first `when` that matches wins over
/// unconditional bindings; if none matches, the first unconditional one.
fn choose<'a>(
    candidates: impl IntoIterator<Item = (usize, &'a Binding)>,
    context: &TriggerContext,
) -> Option<(usize, &'a Binding)> {
    let mut fallback = None;
    for (index, binding) in candidates {
        match &binding.when {
            Some(when) if when.matches(context) => return Some((index, binding)),
            Some(_) => {}
            None => {
                if fallback.is_none() {
                    fallback = Some((index, binding));
                }
            }
        }
    }
    fallback
}

fn check_gesture(gesture: &GestureConfig) -> Result<()> {
    if gesture.threshold <= 0.0 || gesture.repeat_step < 0.0 {
        bail!("gesture.threshold must be > 0 and gesture.repeat_step >= 0");
    }
    Ok(())
}

struct DecodeIssue {
    severity: Severity,
    message: String,
}

impl DecodeIssue {
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// Decodes each binding on its own. The returned candidates keep their file
/// index so later validation can report the right place.
fn decode_bindings(values: Vec<Value>, report: &mut LoadReport) -> Vec<(usize, Binding)> {
    let mut candidates = Vec::new();
    for (index, value) in values.into_iter().enumerate() {
        let name = value.get("name").and_then(Value::as_str).map(str::to_owned);
        match decode_binding(&value) {
            Ok(mut binding) => {
                binding.index = index;
                candidates.push((index, binding));
            }
            Err(issue) => report.problems.push(BindingProblem {
                index,
                name,
                severity: issue.severity,
                message: issue.message,
            }),
        }
    }
    candidates
}

fn decode_binding(value: &Value) -> Result<Binding, DecodeIssue> {
    if let Some(message) = unsupported_when_key(value) {
        return Err(DecodeIssue::warning(message));
    }
    match serde_json::from_value::<Binding>(value.clone()) {
        Ok(binding) => {
            for action in &binding.actions {
                action.validate().map_err(DecodeIssue::error)?;
            }
            Ok(binding)
        }
        Err(e) => match unsupported_reason(value) {
            Some(message) => Err(DecodeIssue::warning(message)),
            None => Err(DecodeIssue::error(format!("{e}"))),
        },
    }
}

/// A `when` key this build does not know is a feature of a newer build, not
/// a typo to fail on.
fn unsupported_when_key(value: &Value) -> Option<String> {
    let when = value.get("when")?.as_object()?;
    for key in when.keys() {
        if !matches!(key.as_str(), "app" | "not_app" | "os" | "desktop") {
            return Some(format!(
                "`when.{key}` is not supported by this build; the binding was skipped"
            ));
        }
    }
    None
}

/// Whether a binding that failed to decode uses a feature this build does not
/// implement, rather than containing a typo.
fn unsupported_reason(value: &Value) -> Option<String> {
    if let Some(kind) = value
        .get("trigger")
        .and_then(|trigger| trigger.get("type"))
        .and_then(Value::as_str)
        && !matches!(kind, "swipe" | "hotkey")
    {
        return Some(format!(
            "trigger type `{kind}` is not supported by this build; the binding was skipped"
        ));
    }
    if let Some(actions) = value.get("actions").and_then(Value::as_array) {
        for action in actions {
            if let Some(kind) = action.get("type").and_then(Value::as_str)
                && !matches!(
                    kind,
                    "window.next"
                        | "window.prev"
                        | "app.next"
                        | "app.prev"
                        | "delay"
                        | "exec"
                        | "keys"
                )
            {
                return Some(format!(
                    "action type `{kind}` is not supported by this build; the binding was skipped"
                ));
            }
        }
    }
    None
}

/// Checks what serde cannot, one binding at a time. A bad binding is reported
/// and dropped; the rest stay.
fn compile_bindings(candidates: Vec<(usize, Binding)>, report: &mut LoadReport) -> Vec<Binding> {
    let mut bindings = Vec::new();
    for (index, binding) in candidates {
        if binding.enabled {
            if let Some(when) = &binding.when {
                let problems = when.problems();
                if !problems.is_empty() {
                    for message in problems {
                        report.problems.push(BindingProblem {
                            index,
                            name: binding.name.clone(),
                            severity: Severity::Error,
                            message,
                        });
                    }
                    continue;
                }
            }
            if let Err(message) = check_binding(&binding) {
                report.problems.push(BindingProblem {
                    index,
                    name: binding.name.clone(),
                    severity: Severity::Error,
                    message,
                });
                continue;
            }
        }
        bindings.push(binding);
    }
    bindings
}

fn check_binding(binding: &Binding) -> Result<(), String> {
    match &binding.trigger {
        Trigger::Swipe { fingers, .. } => {
            if !(3..=5).contains(fingers) {
                return Err(format!(
                    "swipe with {fingers} fingers: only 3 to 5 fingers can be used \
                     (one and two are pointer motion and scrolling)"
                ));
            }
        }
        Trigger::Hotkey { keys } => {
            Hotkey::parse(keys)?;
        }
    }
    Ok(())
}

/// The old TOML file next to `path` (`config.json` → `config.toml`).
pub fn legacy_path(path: &Path) -> PathBuf {
    path.with_file_name("config.toml")
}

/// Reads an old `config.toml` and converts it to the version 2 model.
pub fn load_legacy(path: &Path) -> Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let old: LegacyConfig =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    check_gesture(&old.gesture).with_context(|| format!("checking {}", path.display()))?;
    let (cycle, gesture, hud, candidates) = old.into_parts();
    let mut report = LoadReport {
        source: Source::LegacyToml(path.to_path_buf()),
        problems: Vec::new(),
    };
    let bindings = compile_bindings(candidates, &mut report);
    Ok(Config {
        schema: SCHEMA_URL.into(),
        version: VERSION,
        cycle,
        gesture,
        hud,
        bindings,
        report,
    })
}

pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("touchpad-snap")
        .join("config.json")
}

/// Loads `path` (or its legacy TOML, or the defaults).
pub fn load(path: &Path) -> Result<Config> {
    Config::load_report(path)
}

/// Pretty JSON for `cfg`, with a trailing newline.
pub fn to_json(cfg: &Config) -> String {
    let mut text = serde_json::to_string_pretty(cfg).expect("the config serializes");
    text.push('\n');
    text
}

/// The file written by `touchpad-snap config init`.
pub fn default_json() -> String {
    to_json(&Config::default())
}

/// Creates `path` from [`default_json`] when it does not exist, creating its
/// directory too. Returns whether it wrote the file. The tray's "Open config
/// file" item uses this so the item always has something to open.
pub fn ensure(path: &Path) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, default_json()).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Writes `cfg` as pretty JSON.
pub fn write(path: &Path, cfg: &Config) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, to_json(cfg)).with_context(|| format!("writing {}", path.display()))
}

/// The JSON Schema for the config file, generated from the Rust types.
pub fn schema_value() -> Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(Config)).expect("the schema serializes");
    if let Some(object) = schema.as_object_mut() {
        object.insert("$id".into(), Value::String(SCHEMA_URL.into()));
        object.insert(
            "title".into(),
            Value::String("touchpad-snap configuration".into()),
        );
    }
    schema
}

/// Pretty JSON Schema, with a trailing newline.
pub fn schema_json() -> String {
    let mut text = serde_json::to_string_pretty(&schema_value()).expect("the schema serializes");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(value: Value) -> Binding {
        decode_binding(&value).unwrap_or_else(|issue| panic!("{}", issue.message))
    }

    fn json(text: &str) -> Config {
        let path = write_temp(text);
        let cfg = Config::load_report(&path).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        cfg
    }

    #[test]
    fn defaults_match_the_documented_shape() {
        let cfg = Config::default();
        let value: Value = serde_json::from_str(&to_json(&cfg)).unwrap();
        assert_eq!(value["$schema"], SCHEMA_URL);
        assert_eq!(value["version"], VERSION);
        assert_eq!(value["cycle"]["order"], "stable");
        assert_eq!(value["gesture"]["threshold"], 120.0);
        assert_eq!(value["hud"]["duration_ms"], 700);
        assert_eq!(value["bindings"].as_array().unwrap().len(), 6);
        assert!(cfg.validate_report().problems.is_empty());
        assert_eq!(cfg.hotkeys().len(), 2);
        assert_eq!(cfg.claims().len(), 2);
    }

    #[test]
    fn template_round_trips() {
        let text = default_json();
        let path = write_temp(&text);
        let cfg: Config = Config::load_report(&path).unwrap();
        assert_eq!(to_json(&cfg), text);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn write_temp(text: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("touchpad-snap-temp-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let cfg = json(r#"{"cycle":{"order":"mru"},"bindings":[]}"#);
        assert_eq!(cfg.cycle.order, Order::Mru);
        assert!(cfg.bindings.is_empty());
        assert_eq!(cfg.gesture, GestureConfig::default());
    }

    #[test]
    fn missing_bindings_keep_the_defaults() {
        let cfg = json(r#"{"cycle":{"order":"mru"}}"#);
        assert_eq!(cfg.bindings, Config::default().bindings);
    }

    #[test]
    fn unknown_top_level_keys_are_errors() {
        assert!(Config::load_report(&write_temp(r#"{"ordr":"mru"}"#)).is_err());
        assert!(Config::load_report(&write_temp(r#"{"cycle":{"ordr":"mru"}}"#)).is_err());
    }

    #[test]
    fn one_bad_binding_does_not_drop_the_others() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+K"},"actons":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+L"},"actions":[{"type":"window.prev"}]}
                ]
            }"#,
        );
        assert_eq!(cfg.bindings.len(), 2);
        assert_eq!(cfg.hotkeys().len(), 2);
        assert_eq!(cfg.report().problems.len(), 1);
        assert!(cfg.report().has_errors());
        assert_eq!(cfg.report().problems[0].index, 1);
        assert_eq!(cfg.report().problems[0].severity, Severity::Error);
        assert!(cfg.report().problems[0].message.contains("actons"));
    }

    #[test]
    fn unknown_fields_inside_trigger_and_action_are_errors() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J","oops":1},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+K"},"actions":[{"type":"delay","ms":100,"oops":1}]}
                ]
            }"#,
        );
        assert!(cfg.bindings.is_empty());
        assert_eq!(cfg.report().errors().count(), 2);
    }

    #[test]
    fn unsupported_bindings_are_warnings_and_skipped() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"when":{"title":"Firefox"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"tap","fingers":3},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+K"},"actions":[{"type":"window.close"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+L"},"actions":[{"type":"window.next"}]}
                ]
            }"#,
        );
        assert_eq!(cfg.bindings.len(), 1);
        assert_eq!(cfg.report().problems.len(), 3);
        assert!(!cfg.report().has_errors());
        assert!(
            cfg.report()
                .warnings()
                .all(|p| p.severity == Severity::Warning)
        );
        assert!(cfg.report().problems[0].message.contains("when.title"));
    }

    #[test]
    fn keys_actions_are_checked_when_loading() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"swipe","fingers":3,"modifiers":["alt"],"direction":"down"},
                     "actions":[{"type":"keys","keys":"Ctrl+Shift+Tab"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+K"},
                     "actions":[{"type":"keys","keys":"Ctrl+Hyper+W"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+L"},
                     "actions":[{"type":"keys","keys":"Ctrl+Nonsense"}]}
                ]
            }"#,
        );
        assert_eq!(cfg.bindings.len(), 1, "{:?}", cfg.report().problems);
        let errors: Vec<_> = cfg.report().errors().map(|p| p.message.clone()).collect();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("Hyper"), "{errors:?}");
        assert!(errors[1].contains("Nonsense"), "{errors:?}");
        assert!(errors.iter().all(|e| e.starts_with("action `keys`")));
    }

    #[test]
    fn delay_chain_parses_and_decides_repeat() {
        let swipe = binding(serde_json::json!({
            "trigger": {"type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "down"},
            "actions": [{"type": "delay", "ms": 100}]
        }));
        assert!(!swipe.repeats());
        let cycle = binding(serde_json::json!({
            "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+J"},
            "actions": [{"type": "delay", "ms": 100}, {"type": "window.next"}]
        }));
        assert!(cycle.repeats());
        let explicit = binding(serde_json::json!({
            "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+J"},
            "repeat": false,
            "actions": [{"type": "window.next"}]
        }));
        assert!(!explicit.repeats());
    }

    #[test]
    fn app_actions_parse_and_repeat_by_default() {
        let next = binding(serde_json::json!({
            "trigger": {"type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "left"},
            "actions": [{"type": "app.next"}]
        }));
        assert_eq!(next.actions, vec![Action::AppNext]);
        assert!(next.repeats());
        let prev = binding(serde_json::json!({
            "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+P"},
            "actions": [{"type": "app.prev"}]
        }));
        assert_eq!(prev.actions, vec![Action::AppPrev]);
        assert!(prev.repeats());
    }

    #[test]
    fn exec_actions_parse_and_validate() {
        let direct = binding(serde_json::json!({
            "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+E"},
            "actions": [{"type": "exec", "command": ["ghostty", "--working-directory", "~"]}]
        }));
        assert_eq!(
            direct.actions,
            vec![Action::Exec {
                command: ExecCommand::Args(vec![
                    "ghostty".into(),
                    "--working-directory".into(),
                    "~".into()
                ]),
                shell: false,
            }]
        );
        assert!(!direct.repeats());

        let shell = binding(serde_json::json!({
            "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+E"},
            "actions": [{"type": "exec", "command": "notify-send \"$TOUCHPAD_SNAP_WINDOW_TITLE\"", "shell": true}]
        }));
        assert!(matches!(
            &shell.actions[0],
            Action::Exec {
                command: ExecCommand::Shell(_),
                shell: true
            }
        ));
        assert!(!shell.repeats());

        let string_without_shell = decode_binding(&serde_json::json!({
            "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+E"},
            "actions": [{"type": "exec", "command": "notify-send hi"}]
        }))
        .unwrap_err();
        assert_eq!(string_without_shell.severity, Severity::Error);
        assert!(string_without_shell.message.contains("shell"));

        let list_with_shell = decode_binding(&serde_json::json!({
            "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+E"},
            "actions": [{"type": "exec", "command": ["sh", "-c", "true"], "shell": true}]
        }))
        .unwrap_err();
        assert_eq!(list_with_shell.severity, Severity::Error);
        assert!(list_with_shell.message.contains("runs directly"));

        for command in [serde_json::json!([]), serde_json::json!("")] {
            let shell = command.is_string();
            let bad = decode_binding(&serde_json::json!({
                "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+E"},
                "actions": [{"type": "exec", "command": command, "shell": shell}]
            }))
            .unwrap_err();
            assert_eq!(bad.severity, Severity::Error);
        }
    }

    #[test]
    fn exec_chain_fires_once_unless_repeat_is_set() {
        let make = |repeat: Option<bool>| {
            let mut value = serde_json::json!({
                "trigger": {"type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "up"},
                "actions": [{"type": "exec", "command": ["true"]}]
            });
            if let Some(repeat) = repeat {
                value["repeat"] = Value::Bool(repeat);
            }
            binding(value)
        };
        assert!(!make(None).repeats());
        assert!(make(Some(true)).repeats());
        // A window action keeps the old default even next to exec.
        let mixed = binding(serde_json::json!({
            "trigger": {"type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "up"},
            "actions": [{"type": "exec", "command": ["true"]}, {"type": "window.next"}]
        }));
        assert!(mixed.repeats());
    }

    #[test]
    fn trigger_labels_are_stable() {
        assert_eq!(
            Trigger::Hotkey {
                keys: "Super+Alt+N".into()
            }
            .label(),
            "hotkey-Super+Alt+N"
        );
        assert_eq!(
            Trigger::Swipe {
                fingers: 3,
                modifiers: vec![Modifier::Alt],
                direction: SwipeDirection::Up,
            }
            .label(),
            "swipe-3-alt-up"
        );
        assert_eq!(
            Trigger::Swipe {
                fingers: 4,
                modifiers: vec![],
                direction: SwipeDirection::Left,
            }
            .label(),
            "swipe-4-left"
        );
    }

    #[test]
    fn exec_commands_are_checked_for_paths() {
        let current = std::env::current_exe().unwrap();
        assert!(executable_on_path(&current.to_string_lossy()));
        assert!(!executable_on_path(
            "touchpad-snap-not-a-real-command-3f9c1e"
        ));
        assert!(!executable_on_path("/definitely/not/here"));

        let cfg = json(
            r#"{
                "bindings": [
                    {"name":"editor","trigger":{"type":"hotkey","keys":"Ctrl+Alt+E"},"actions":[{"type":"exec","command":["definitely-missing-3f9c1e"]}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+S"},"actions":[{"type":"exec","command":"true","shell":true}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+N"},"actions":[{"type":"window.next"}]}
                ]
            }"#,
        );
        let missing = cfg.missing_exec_commands();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].index, 0);
        assert_eq!(missing[0].name.as_deref(), Some("editor"));
        assert_eq!(missing[0].severity, Severity::Warning);
        assert!(missing[0].message.contains("definitely-missing-3f9c1e"));
        assert!(cfg.has_shell_exec());
    }

    #[test]
    fn validation_is_per_binding() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"hotkey","keys":"Super+Alt+N"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"super+alt+n"},"actions":[{"type":"window.prev"}]},
                    {"trigger":{"type":"swipe","fingers":2,"direction":"left"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"actions":[{"type":"window.prev"}]}
                ]
            }"#,
        );
        // The same hotkey twice is allowed: `when` can make one conditional,
        // and the later one is simply shadowed. The two-finger swipe is not.
        assert_eq!(cfg.bindings.len(), 3);
        assert_eq!(cfg.hotkeys().len(), 2);
        assert_eq!(cfg.report().errors().count(), 1);
        assert!(cfg.report().errors().any(|p| p.message.contains("3 to 5")));
    }

    #[test]
    fn disabled_bindings_are_kept_but_not_registered() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"name":"old","enabled":false,"trigger":{"type":"hotkey","keys":"Super+Alt+N"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"actions":[{"type":"window.prev"}]}
                ]
            }"#,
        );
        assert_eq!(cfg.bindings.len(), 2);
        assert_eq!(cfg.hotkeys().len(), 1);
        assert_eq!(
            cfg.hotkey_binding(0).unwrap().trigger,
            Trigger::Hotkey {
                keys: "Ctrl+Alt+J".into()
            }
        );
    }

    #[test]
    fn claims_and_lookup() {
        let cfg = Config::default();
        let claims = cfg.claims();
        let claim = |mods| Claim::gesture(3, mods);
        assert_eq!(claims, vec![claim(Mods::CTRL), claim(Mods::ALT)]);
        let bound = |mods, direction| {
            cfg.swipe_bindings(claim(mods), direction)
                .into_iter()
                .map(|b| b.actions.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            bound(Mods::CTRL, SwipeDirection::Left),
            vec![vec![Action::Next]]
        );
        assert_eq!(
            bound(Mods::CTRL, SwipeDirection::Right),
            vec![vec![Action::Prev]]
        );
        assert_eq!(
            bound(Mods::ALT, SwipeDirection::Up),
            vec![vec![Action::Next]]
        );
        assert_eq!(
            bound(Mods::ALT, SwipeDirection::Down),
            vec![vec![Action::Prev]]
        );
        // Each modifier has one axis: the other one is claimed but unbound.
        assert!(bound(Mods::CTRL, SwipeDirection::Up).is_empty());
        assert!(bound(Mods::ALT, SwipeDirection::Left).is_empty());
    }

    fn linux_ctx(app_id: &str, app_name: &str, wm_class: &str) -> TriggerContext {
        TriggerContext::new(
            PlatformDescription::new("linux", Some("gnome")),
            Some(crate::when::AppContext {
                app_id: app_id.into(),
                app_name: app_name.into(),
                wm_class: wm_class.into(),
            }),
        )
    }

    fn firefox() -> TriggerContext {
        linux_ctx("firefox.desktop", "Firefox", "firefox")
    }

    fn editor() -> TriggerContext {
        linux_ctx(
            "org.gnome.TextEditor.desktop",
            "Text Editor",
            "org.gnome.TextEditor",
        )
    }

    #[test]
    fn when_conditions_parse_into_the_binding() {
        let conditional = binding(serde_json::json!({
            "trigger": {"type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "down"},
            "when": {
                "app": ["firefox", "org.chromium.Chromium"],
                "not_app": ["*private*"],
                "os": ["linux"],
                "desktop": ["gnome", "kde"]
            },
            "actions": [{"type": "window.next"}]
        }));
        let when = conditional.when.unwrap();
        assert_eq!(when.app, vec!["firefox", "org.chromium.Chromium"]);
        assert_eq!(when.not_app, vec!["*private*"]);
        assert_eq!(when.os, vec!["linux"]);
        assert_eq!(when.desktop, vec!["gnome", "kde"]);
        assert!(when.matches(&firefox()));
        assert!(!when.matches(&editor()));
        // Empty `when` and an absent `when` behave the same.
        let empty = binding(serde_json::json!({
            "trigger": {"type": "hotkey", "keys": "Ctrl+Alt+J"},
            "when": {},
            "actions": [{"type": "window.next"}]
        }));
        assert!(empty.when.unwrap().is_empty());
    }

    #[test]
    fn conditional_swipes_beat_unconditional_ones_and_ties_go_to_file_order() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"swipe","fingers":3,"modifiers":["ctrl"],"direction":"left"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"swipe","fingers":3,"modifiers":["ctrl"],"direction":"left"},"when":{"app":["Firefox"]},"actions":[{"type":"window.prev"}]},
                    {"trigger":{"type":"swipe","fingers":3,"modifiers":["ctrl"],"direction":"left"},"when":{"app":["FIRE*"]},"actions":[{"type":"delay","ms":10}]}
                ]
            }"#,
        );
        let claim = Claim::gesture(3, Mods::CTRL);
        let left = SwipeDirection::Left;
        let picked = |ctx: &TriggerContext| {
            cfg.swipe_binding_for(claim.clone(), left, ctx)
                .map(|(_, binding)| binding.actions.clone())
        };
        // The first matching `when` wins, even though an unconditional one
        // comes earlier and another conditional one comes later.
        assert_eq!(picked(&firefox()), Some(vec![Action::Prev]));
        // No matching condition: the unconditional binding still fires.
        assert_eq!(picked(&editor()), Some(vec![Action::Next]));
        // With no focused window, only unconditional bindings can fire.
        let no_window = TriggerContext::new(PlatformDescription::new("linux", Some("gnome")), None);
        assert_eq!(picked(&no_window), Some(vec![Action::Next]));
    }

    #[test]
    fn conditional_hotkeys_share_one_registered_key() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"ctrl+alt+j"},"when":{"app":["firefox"]},"actions":[{"type":"window.prev"}]}
                ]
            }"#,
        );
        assert_eq!(cfg.hotkeys().len(), 1);
        let actions = |ctx: &TriggerContext| {
            cfg.hotkey_binding_for(0, ctx)
                .map(|binding| binding.actions.clone())
        };
        assert_eq!(actions(&firefox()), Some(vec![Action::Prev]));
        assert_eq!(actions(&editor()), Some(vec![Action::Next]));
    }

    #[test]
    fn platform_conditions_select_and_describe_inactive_bindings() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"when":{"os":["windows"]},"actions":[{"type":"window.prev"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+K"},"when":{"desktop":["kde"]},"actions":[{"type":"window.prev"}]}
                ]
            }"#,
        );
        let linux = TriggerContext::new(PlatformDescription::new("linux", Some("gnome")), None);
        assert_eq!(
            cfg.hotkey_binding_for(0, &linux)
                .map(|binding| binding.actions.clone()),
            Some(vec![Action::Next])
        );
        assert!(cfg.hotkey_binding_for(1, &linux).is_none());
        let windows = TriggerContext::new(PlatformDescription::new("windows", None), None);
        assert_eq!(
            cfg.hotkey_binding_for(0, &windows)
                .map(|binding| binding.actions.clone()),
            Some(vec![Action::Prev])
        );

        let inactive = cfg.inactive_on(&PlatformDescription::new("linux", Some("gnome")));
        assert_eq!(inactive.len(), 2);
        assert_eq!(inactive[0].index, 0);
        assert!(inactive[0].reason.contains("windows"));
        assert_eq!(inactive[1].index, 2);
        assert!(inactive[1].reason.contains("kde"));
    }

    #[test]
    fn claims_carry_app_filters_and_skip_other_platforms() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"swipe","fingers":3,"modifiers":["ctrl"],"direction":"left"},"when":{"app":["Firefox"]},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"swipe","fingers":3,"modifiers":["ctrl"],"direction":"right"},"when":{"app":["Chromium"]},"actions":[{"type":"window.prev"}]},
                    {"trigger":{"type":"swipe","fingers":3,"modifiers":["alt"],"direction":"up"},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"swipe","fingers":4,"direction":"left"},"when":{"os":["windows"]},"actions":[{"type":"window.next"}]}
                ]
            }"#,
        );
        let claims = cfg.claims_for(&PlatformDescription::new("linux", Some("gnome")));
        assert_eq!(
            claims,
            vec![
                Claim {
                    fingers: 3,
                    mods: Mods::CTRL,
                    apps: vec!["Firefox".into(), "Chromium".into()],
                },
                Claim::gesture(3, Mods::ALT),
            ]
        );
        // On Windows the four-finger claim exists and the GNOME ones do too,
        // since they have no OS condition.
        let windows = cfg.claims_for(&PlatformDescription::new("windows", None));
        assert!(windows.contains(&Claim::gesture(4, Mods::default())));
    }

    #[test]
    fn impossible_conditions_are_errors_and_drop_only_that_binding() {
        let cfg = json(
            r#"{
                "bindings": [
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+J"},"when":{"os":["macos"]},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+K"},"when":{"app":["x"],"not_app":["X"]},"actions":[{"type":"window.next"}]},
                    {"trigger":{"type":"hotkey","keys":"Ctrl+Alt+L"},"actions":[{"type":"window.next"}]}
                ]
            }"#,
        );
        assert_eq!(cfg.bindings.len(), 1);
        assert_eq!(cfg.report().errors().count(), 2);
        assert!(cfg.report().errors().any(|p| p.message.contains("macos")));
        assert!(
            cfg.report()
                .errors()
                .any(|p| p.message.contains("can never fire"))
        );
    }

    #[test]
    fn legacy_toml_is_read_in_memory_until_imported() {
        let dir = std::env::temp_dir().join(format!("touchpad-snap-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let legacy = dir.join("config.toml");
        std::fs::write(
            &legacy,
            "order = \"mru\"\n[[hotkey]]\nkeys = \"Ctrl+Alt+J\"\naction = \"prev\"\n",
        )
        .unwrap();

        let cfg = Config::load_report(&path).unwrap();
        assert_eq!(cfg.cycle.order, Order::Mru);
        assert_eq!(cfg.report().source, Source::LegacyToml(legacy.clone()));
        assert_eq!(cfg.hotkeys().len(), 1);
        assert!(!path.exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ensure_writes_the_defaults_only_once() {
        let dir = std::env::temp_dir().join(format!("touchpad-snap-ensure-{}", std::process::id()));
        let path = dir.join("nested").join("config.json");
        assert!(ensure(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), default_json());
        assert!(!ensure(&path).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn committed_schema_is_current() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crates/core is two levels below the workspace root");
        let path = root.join("schema/config.schema.json");
        let committed: Value = serde_json::from_str(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display())),
        )
        .unwrap();
        assert_eq!(
            committed,
            schema_value(),
            "schema/config.schema.json is stale; run `touchpad-snap config schema > schema/config.schema.json`"
        );
    }
}
