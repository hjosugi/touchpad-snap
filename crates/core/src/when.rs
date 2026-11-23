//! Conditions attached to a binding (`when`): the focused app, the OS and the
//! desktop a binding applies to.
//!
//! Matching is deliberately small and pure: case-insensitive glob patterns
//! where `*` matches any run of characters. App patterns are matched against
//! the focused window's desktop-file id, app name and WM class, so
//! `firefox`, `Firefox` and `firefox.desktop` all work. Empty lists mean "no
//! condition"; all present conditions must match.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::Window;

/// The `when` object of a binding. An empty `When` matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// Apps this binding applies to (glob, case-insensitive).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app: Vec<String>,
    /// Apps this binding does not apply to (glob, case-insensitive).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_app: Vec<String>,
    /// `linux`, `windows` (or `*`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub os: Vec<String>,
    /// `gnome`, `kde`, `sway`, `hyprland`, `niri` (or `*`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub desktop: Vec<String>,
}

impl When {
    pub fn is_empty(&self) -> bool {
        self.app.is_empty()
            && self.not_app.is_empty()
            && self.os.is_empty()
            && self.desktop.is_empty()
    }

    /// Whether every present condition holds in `ctx`.
    pub fn matches(&self, ctx: &TriggerContext) -> bool {
        if !self.matches_platform(&ctx.platform) {
            return false;
        }
        let Some(app) = &ctx.app else {
            // Without a focused window, app conditions cannot match. A
            // not_app condition is vacuously satisfied.
            return self.app.is_empty();
        };
        if !self.app.is_empty() && !self.app.iter().any(|p| app.matches(p)) {
            return false;
        }
        if self.not_app.iter().any(|p| app.matches(p)) {
            return false;
        }
        true
    }

    /// OS and desktop only; used to decide whether a binding can fire on
    /// this machine at all.
    pub fn matches_platform(&self, platform: &PlatformDescription) -> bool {
        if !self.os.is_empty() && !self.os.iter().any(|p| pattern_matches(p, platform.os)) {
            return false;
        }
        if !self.desktop.is_empty() {
            let desktop = platform.desktop.unwrap_or("");
            if !self.desktop.iter().any(|p| pattern_matches(p, desktop)) {
                return false;
            }
        }
        true
    }

    /// Why this binding is inactive on `platform`, or `None` when it applies.
    pub fn inactive_reason(&self, platform: &PlatformDescription) -> Option<String> {
        if self.matches_platform(platform) {
            return None;
        }
        let mut parts = Vec::new();
        if !self.os.is_empty() && !self.os.iter().any(|p| pattern_matches(p, platform.os)) {
            parts.push(format!("os {:?} (this machine: {})", self.os, platform.os));
        }
        if !self.desktop.is_empty() {
            let desktop = platform.desktop.unwrap_or("unknown");
            if !self.desktop.iter().any(|p| pattern_matches(p, desktop)) {
                parts.push(format!(
                    "desktop {:?} (this machine: {desktop})",
                    self.desktop
                ));
            }
        }
        Some(parts.join(", "))
    }

    /// Conditions that can never fire. App patterns matching no installed
    /// window cannot be known statically and are not reported.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for os in &self.os {
            if !os.eq_ignore_ascii_case("*")
                && !os.eq_ignore_ascii_case("linux")
                && !os.eq_ignore_ascii_case("windows")
            {
                problems.push(format!(
                    "`when.os` has unknown value `{os}` (expected `linux` or `windows`)"
                ));
            }
        }
        for desktop in &self.desktop {
            if !desktop.eq_ignore_ascii_case("*")
                && !matches!(
                    desktop.to_ascii_lowercase().as_str(),
                    "gnome" | "kde" | "sway" | "hyprland" | "niri"
                )
            {
                problems.push(format!(
                    "`when.desktop` has unknown value `{desktop}` \
                     (expected gnome, kde, sway, hyprland, niri or *)"
                ));
            }
        }
        for app in &self.app {
            if self
                .not_app
                .iter()
                .any(|excluded| excluded.eq_ignore_ascii_case(app))
            {
                problems.push(format!(
                    "`when.app` and `when.not_app` both list `{app}`, so the binding can never fire"
                ));
            }
        }
        if self.not_app.iter().any(|p| p == "*") {
            problems.push(
                "`when.not_app` excludes every app (`*`), so the binding can never fire".into(),
            );
        }
        problems
    }
}

/// One case-insensitive glob match, where only `*` is special.
pub fn pattern_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let value: Vec<char> = value.to_lowercase().chars().collect();
    let (mut p, mut v) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_value = 0usize;
    while v < value.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            p += 1;
            star_value = v;
        } else if p < pattern.len() && pattern[p] == value[v] {
            p += 1;
            v += 1;
        } else if let Some(s) = star {
            p = s + 1;
            star_value += 1;
            v = star_value;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// The focused app when a trigger started. Kept for the whole gesture so a
/// window switch halfway through cannot change which binding applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppContext {
    pub app_id: String,
    pub app_name: String,
    pub wm_class: String,
}

impl AppContext {
    pub fn from_window(window: &Window) -> Self {
        Self {
            app_id: window.app_id.clone(),
            app_name: window.app_name.clone(),
            wm_class: window.wm_class.clone(),
        }
    }

    /// Uses the extension's app id (the one the GNOME claim was matched
    /// against) when it sent one.
    pub fn with_app_id(mut self, app_id: Option<&str>) -> Self {
        if let Some(app_id) = app_id.filter(|id| !id.is_empty()) {
            self.app_id = app_id.to_string();
        }
        self
    }

    fn matches(&self, pattern: &str) -> bool {
        [&self.app_id, &self.app_name, &self.wm_class]
            .into_iter()
            .any(|value| !value.is_empty() && pattern_matches(pattern, value))
    }
}

/// The OS and desktop a trigger happened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformDescription {
    pub os: &'static str,
    pub desktop: Option<&'static str>,
}

impl PlatformDescription {
    pub fn new(os: &'static str, desktop: Option<&'static str>) -> Self {
        Self { os, desktop }
    }

    /// The platform as named by the environment, without probing any
    /// backend (the daemon has a richer detector; this is for pure callers
    /// such as [`crate::config::Config::claims`]).
    pub fn current() -> Self {
        let os = if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(windows) {
            "windows"
        } else {
            std::env::consts::OS
        };
        let desktop = if cfg!(target_os = "linux") {
            std::env::var("XDG_CURRENT_DESKTOP")
                .ok()
                .and_then(|value| desktop_name(&value))
        } else {
            None
        };
        Self::new(os, desktop)
    }
}

/// The `when.desktop` name in an `XDG_CURRENT_DESKTOP` value.
fn desktop_name(desktop: &str) -> Option<&'static str> {
    for part in desktop.split([':', ';']) {
        let part = part.trim();
        if part.eq_ignore_ascii_case("gnome") {
            return Some("gnome");
        }
        if part.eq_ignore_ascii_case("kde") || part.eq_ignore_ascii_case("plasma") {
            return Some("kde");
        }
        if part.eq_ignore_ascii_case("sway") {
            return Some("sway");
        }
        if part.eq_ignore_ascii_case("hyprland") {
            return Some("hyprland");
        }
        if part.eq_ignore_ascii_case("niri") {
            return Some("niri");
        }
    }
    None
}

/// Everything a `when` condition can be evaluated against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TriggerContext {
    pub platform: PlatformDescription,
    pub app: Option<AppContext>,
}

impl Default for PlatformDescription {
    fn default() -> Self {
        Self::new(if cfg!(windows) { "windows" } else { "linux" }, None)
    }
}

impl TriggerContext {
    pub fn new(platform: PlatformDescription, app: Option<AppContext>) -> Self {
        Self { platform, app }
    }

    pub fn with_app_id(mut self, app_id: Option<&str>) -> Self {
        self.app = self.app.map(|app| app.with_app_id(app_id));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str, wm_class: &str) -> AppContext {
        AppContext {
            app_id: id.into(),
            app_name: name.into(),
            wm_class: wm_class.into(),
        }
    }

    fn ctx(app: Option<AppContext>) -> TriggerContext {
        TriggerContext::new(PlatformDescription::new("linux", Some("gnome")), app)
    }

    #[test]
    fn patterns_are_case_insensitive_globs() {
        assert!(pattern_matches("firefox", "Firefox"));
        assert!(pattern_matches("FIRE*", "firefox.desktop"));
        assert!(pattern_matches("*.desktop", "org.gnome.TextEditor.desktop"));
        assert!(pattern_matches("org.*TextEditor*", "org.gnome.TextEditor"));
        assert!(pattern_matches("*", "anything"));
        assert!(pattern_matches("", ""));
        assert!(!pattern_matches("", "x"));
        assert!(!pattern_matches("firefox", "firefoxes"));
        assert!(!pattern_matches("firefox*", "icefirefox"));
        assert!(pattern_matches("a*b*c", "aXXbYYc"));
        assert!(!pattern_matches("a*b*c", "aXXbYY"));
    }

    #[test]
    fn app_conditions_match_id_name_and_wm_class() {
        let when = When {
            app: vec!["firefox".into()],
            ..Default::default()
        };
        assert!(when.matches(&ctx(Some(app("firefox.desktop", "Firefox", "firefox")))));
        assert!(when.matches(&ctx(Some(app(
            "org.mozilla.firefox",
            "Firefox",
            "Navigator"
        )))));
        assert!(!when.matches(&ctx(Some(app(
            "org.gnome.TextEditor.desktop",
            "Text Editor",
            "org.gnome.TextEditor"
        )))));
        assert!(!when.matches(&ctx(None)));
    }

    #[test]
    fn not_app_excludes_and_empty_when_matches_everything() {
        let when = When {
            not_app: vec!["*Editor*".into()],
            ..Default::default()
        };
        assert!(!when.matches(&ctx(Some(app(
            "org.gnome.TextEditor.desktop",
            "Text Editor",
            "org.gnome.TextEditor"
        )))));
        assert!(when.matches(&ctx(Some(app("firefox.desktop", "Firefox", "firefox")))));
        assert!(When::default().matches(&ctx(None)));
    }

    #[test]
    fn os_and_desktop_conditions() {
        let when = When {
            os: vec!["linux".into()],
            desktop: vec!["gnome".into()],
            ..Default::default()
        };
        assert!(when.matches(&ctx(None)));
        assert!(!when.matches(&TriggerContext::new(
            PlatformDescription::new("windows", None),
            None
        )));
        assert!(!when.matches(&TriggerContext::new(
            PlatformDescription::new("linux", Some("kde")),
            None
        )));
        let any = When {
            os: vec!["*".into()],
            desktop: vec!["*".into()],
            ..Default::default()
        };
        assert!(any.matches(&TriggerContext::new(
            PlatformDescription::new("windows", None),
            None
        )));
    }

    #[test]
    fn impossible_conditions_are_reported() {
        assert_eq!(When::default().problems(), Vec::<String>::new());
        let problems = When {
            app: vec!["Firefox".into()],
            not_app: vec!["firefox".into()],
            os: vec!["macos".into()],
            desktop: vec!["plasmax".into()],
        }
        .problems();
        assert!(problems.iter().any(|p| p.contains("can never fire")));
        assert!(problems.iter().any(|p| p.contains("macos")));
        assert!(problems.iter().any(|p| p.contains("plasmax")));

        let excluded_everything = When {
            not_app: vec!["*".into()],
            ..Default::default()
        }
        .problems();
        assert_eq!(excluded_everything.len(), 1);
        assert!(excluded_everything[0].contains("every app"));
    }

    #[test]
    fn inactive_reason_names_the_mismatch() {
        let when = When {
            os: vec!["windows".into()],
            desktop: vec!["kde".into()],
            ..Default::default()
        };
        let reason = when
            .inactive_reason(&PlatformDescription::new("linux", Some("gnome")))
            .unwrap();
        assert!(reason.contains("os"));
        assert!(reason.contains("windows"));
        assert!(reason.contains("desktop"));
        assert!(reason.contains("kde"));
        assert!(
            when.inactive_reason(&PlatformDescription::new("windows", Some("kde")))
                .is_none()
        );
    }
}
