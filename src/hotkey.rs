//! Portable key-combination syntax (`"Super+Alt+N"`) and its translation into
//! a GNOME accelerator string and a Win32 `RegisterHotKey` pair.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Modifier bit set. The same bits travel over D-Bus to the GNOME extension
/// (swipe claims), so they are part of the extension protocol: do not renumber.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mods(pub u32);

impl Mods {
    pub const SHIFT: Mods = Mods(1);
    pub const CTRL: Mods = Mods(2);
    pub const ALT: Mods = Mods(4);
    pub const SUPER: Mods = Mods(8);

    pub fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        Mods(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Mods {
    fn bitor_assign(&mut self, rhs: Mods) {
        self.0 |= rhs.0;
    }
}

/// A modifier as written in the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Shift,
    #[serde(alias = "control")]
    Ctrl,
    #[serde(alias = "option")]
    Alt,
    #[serde(alias = "win", alias = "meta", alias = "cmd", alias = "logo")]
    Super,
}

impl Modifier {
    pub fn bits(self) -> Mods {
        match self {
            Modifier::Shift => Mods::SHIFT,
            Modifier::Ctrl => Mods::CTRL,
            Modifier::Alt => Mods::ALT,
            Modifier::Super => Mods::SUPER,
        }
    }

    fn parse(s: &str) -> Option<Modifier> {
        match s.to_ascii_lowercase().as_str() {
            "shift" => Some(Modifier::Shift),
            "ctrl" | "control" => Some(Modifier::Ctrl),
            "alt" | "option" => Some(Modifier::Alt),
            "super" | "win" | "meta" | "cmd" | "logo" => Some(Modifier::Super),
            _ => None,
        }
    }
}

pub fn mods_of(list: &[Modifier]) -> Mods {
    list.iter().fold(Mods::default(), |acc, m| acc | m.bits())
}

/// A non-modifier key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Letter(char),
    Digit(char),
    F(u8),
    Named(Named),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Named {
    Left,
    Right,
    Up,
    Down,
    Tab,
    Space,
    Return,
    Escape,
    Home,
    End,
    PageUp,
    PageDown,
    Grave,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Semicolon,
    Apostrophe,
    Comma,
    Period,
    Slash,
    Backslash,
    Backspace,
    Delete,
}

/// (accepted spellings, GNOME keysym name, Windows virtual-key code).
/// Punctuation VKs are the US-layout `VK_OEM_*` codes.
const NAMED: &[(&[&str], Named, &str, u16)] = &[
    (&["left"], Named::Left, "Left", 0x25),
    (&["right"], Named::Right, "Right", 0x27),
    (&["up"], Named::Up, "Up", 0x26),
    (&["down"], Named::Down, "Down", 0x28),
    (&["tab"], Named::Tab, "Tab", 0x09),
    (&["space"], Named::Space, "space", 0x20),
    (&["return", "enter"], Named::Return, "Return", 0x0D),
    (&["escape", "esc"], Named::Escape, "Escape", 0x1B),
    (&["home"], Named::Home, "Home", 0x24),
    (&["end"], Named::End, "End", 0x23),
    (
        &["pageup", "page_up", "pgup"],
        Named::PageUp,
        "Page_Up",
        0x21,
    ),
    (
        &["pagedown", "page_down", "pgdn"],
        Named::PageDown,
        "Page_Down",
        0x22,
    ),
    (&["grave", "backquote", "`"], Named::Grave, "grave", 0xC0),
    (&["minus", "-"], Named::Minus, "minus", 0xBD),
    (&["equal", "="], Named::Equal, "equal", 0xBB),
    (
        &["bracketleft", "["],
        Named::BracketLeft,
        "bracketleft",
        0xDB,
    ),
    (
        &["bracketright", "]"],
        Named::BracketRight,
        "bracketright",
        0xDD,
    ),
    (&["semicolon", ";"], Named::Semicolon, "semicolon", 0xBA),
    (
        &["apostrophe", "quote", "'"],
        Named::Apostrophe,
        "apostrophe",
        0xDE,
    ),
    (&["comma", ","], Named::Comma, "comma", 0xBC),
    (&["period", "."], Named::Period, "period", 0xBE),
    (&["slash", "/"], Named::Slash, "slash", 0xBF),
    (&["backslash", "\\"], Named::Backslash, "backslash", 0xDC),
    (&["backspace"], Named::Backspace, "BackSpace", 0x08),
    (&["delete", "del"], Named::Delete, "Delete", 0x2E),
];

impl Key {
    fn parse(s: &str) -> Option<Key> {
        let lower = s.to_ascii_lowercase();
        let mut chars = lower.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_lowercase() {
                return Some(Key::Letter(c));
            }
            if c.is_ascii_digit() {
                return Some(Key::Digit(c));
            }
        }
        if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
            && (1..=24).contains(&n)
        {
            return Some(Key::F(n));
        }
        NAMED
            .iter()
            .find(|(names, ..)| names.contains(&lower.as_str()))
            .map(|(_, named, ..)| Key::Named(*named))
    }

    fn row(named: Named) -> &'static (&'static [&'static str], Named, &'static str, u16) {
        NAMED
            .iter()
            .find(|(_, n, ..)| *n == named)
            .expect("every Named has a row")
    }

    /// The X keysym name GNOME expects in an accelerator.
    pub fn gnome_name(self) -> String {
        match self {
            Key::Letter(c) | Key::Digit(c) => c.to_string(),
            Key::F(n) => format!("F{n}"),
            Key::Named(n) => Key::row(n).2.to_string(),
        }
    }

    /// The X keysym (GNOME's `notify_keyval` takes these): the lowercase
    /// ASCII code for letters, digits and most punctuation, `0xffxx` for the
    /// rest.
    pub fn keysym(self) -> u32 {
        match self {
            Key::Letter(c) | Key::Digit(c) => c as u32,
            Key::F(n) => 0xffbe + (n as u32 - 1),
            Key::Named(n) => match n {
                Named::Left => 0xff51,
                Named::Up => 0xff52,
                Named::Right => 0xff53,
                Named::Down => 0xff54,
                Named::Tab => 0xff09,
                Named::Space => 0x20,
                Named::Return => 0xff0d,
                Named::Escape => 0xff1b,
                Named::Home => 0xff50,
                Named::End => 0xff57,
                Named::PageUp => 0xff55,
                Named::PageDown => 0xff56,
                Named::Grave => 0x60,
                Named::Minus => 0x2d,
                Named::Equal => 0x3d,
                Named::BracketLeft => 0x5b,
                Named::BracketRight => 0x5d,
                Named::Semicolon => 0x3b,
                Named::Apostrophe => 0x27,
                Named::Comma => 0x2c,
                Named::Period => 0x2e,
                Named::Slash => 0x2f,
                Named::Backslash => 0x5c,
                Named::Backspace => 0xff08,
                Named::Delete => 0xffff,
            },
        }
    }

    /// The Win32 virtual-key code.
    pub fn windows_vk(self) -> u16 {
        match self {
            Key::Letter(c) => c.to_ascii_uppercase() as u16,
            Key::Digit(c) => c as u16,
            Key::F(n) => 0x70 + (n as u16 - 1),
            Key::Named(n) => Key::row(n).3,
        }
    }
}

/// A parsed key combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub mods: Mods,
    pub key: Key,
}

impl Hotkey {
    /// Parses `"Super+Alt+N"`, `"ctrl+shift+F5"`, `"Alt+grave"` and the like.
    /// Case does not matter; the key comes last.
    pub fn parse(spec: &str) -> Result<Hotkey, String> {
        let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
        // "Ctrl++" means Ctrl and the plus key, which we do not support; an
        // empty part is always a typo.
        if parts.iter().any(|p| p.is_empty()) {
            return Err(format!("`{spec}`: empty key name"));
        }
        let (key, mods) = parts
            .split_last()
            .ok_or_else(|| format!("`{spec}`: empty"))?;
        let mut bits = Mods::default();
        for m in mods {
            let m = Modifier::parse(m).ok_or_else(|| {
                format!("`{spec}`: `{m}` is not a modifier (shift, ctrl, alt, super)")
            })?;
            bits |= m.bits();
        }
        let key = Key::parse(key).ok_or_else(|| format!("`{spec}`: unknown key `{key}`"))?;
        Ok(Hotkey { mods: bits, key })
    }

    /// GNOME accelerator syntax, e.g. `<Super><Alt>n`.
    pub fn to_gnome(self) -> String {
        let mut s = String::new();
        if self.mods.contains(Mods::SUPER) {
            s.push_str("<Super>");
        }
        if self.mods.contains(Mods::CTRL) {
            s.push_str("<Control>");
        }
        if self.mods.contains(Mods::ALT) {
            s.push_str("<Alt>");
        }
        if self.mods.contains(Mods::SHIFT) {
            s.push_str("<Shift>");
        }
        s.push_str(&self.key.gnome_name());
        s
    }

    /// The modifiers of this combination, as the list of single keys to press.
    pub fn modifier_list(self) -> Vec<Modifier> {
        [
            Modifier::Super,
            Modifier::Ctrl,
            Modifier::Alt,
            Modifier::Shift,
        ]
        .into_iter()
        .filter(|m| self.mods.contains(m.bits()))
        .collect()
    }

    /// `(fsModifiers, vk)` for `RegisterHotKey`, with `MOD_NOREPEAT` set so
    /// holding the keys down does not machine-gun through windows.
    pub fn to_windows(self) -> (u32, u32) {
        const MOD_ALT: u32 = 0x1;
        const MOD_CONTROL: u32 = 0x2;
        const MOD_SHIFT: u32 = 0x4;
        const MOD_WIN: u32 = 0x8;
        const MOD_NOREPEAT: u32 = 0x4000;
        let mut m = MOD_NOREPEAT;
        if self.mods.contains(Mods::ALT) {
            m |= MOD_ALT;
        }
        if self.mods.contains(Mods::CTRL) {
            m |= MOD_CONTROL;
        }
        if self.mods.contains(Mods::SHIFT) {
            m |= MOD_SHIFT;
        }
        if self.mods.contains(Mods::SUPER) {
            m |= MOD_WIN;
        }
        (m, self.key.windows_vk() as u32)
    }
}

/// One step of typing a shortcut while the trigger's own modifiers are down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Typed {
    /// A modifier the trigger is held with: released first, pressed again at
    /// the end, so the user's real release stays the only one applications
    /// see afterwards.
    Held(Modifier, bool),
    /// A modifier of the shortcut itself.
    Target(Modifier, bool),
    /// The shortcut's key.
    Key(bool),
    /// A tap of a key that means nothing. Windows opens the menu bar after
    /// a lone Alt and Start after a lone Win; a keystroke in between turns
    /// that off.
    Neutral,
}

/// The steps that type `keys` while `held` modifiers are physically down:
/// `Alt` + swipe sending `Ctrl+Tab` has to arrive as `Ctrl+Tab`, not
/// `Alt+Ctrl+Tab`. Only for backends that can release a key the user holds
/// (Windows); GNOME cannot and waits for the release instead.
pub fn typing_order(keys: Hotkey, held: Mods) -> Vec<Typed> {
    let list = |mods: Mods| {
        Hotkey {
            mods,
            key: keys.key,
        }
        .modifier_list()
    };
    let lone = held.contains(Mods::ALT) || held.contains(Mods::SUPER);
    let mut steps = Vec::new();
    if lone {
        steps.push(Typed::Neutral);
    }
    steps.extend(list(held).into_iter().map(|m| Typed::Held(m, false)));
    steps.extend(list(keys.mods).into_iter().map(|m| Typed::Target(m, true)));
    steps.extend([Typed::Key(true), Typed::Key(false)]);
    steps.extend(
        list(keys.mods)
            .into_iter()
            .rev()
            .map(|m| Typed::Target(m, false)),
    );
    steps.extend(list(held).into_iter().map(|m| Typed::Held(m, true)));
    if lone {
        steps.push(Typed::Neutral);
    }
    steps
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (bit, name) in [
            (Mods::SUPER, "Super"),
            (Mods::CTRL, "Ctrl"),
            (Mods::ALT, "Alt"),
            (Mods::SHIFT, "Shift"),
        ] {
            if self.mods.contains(bit) {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Letter(c) => write!(f, "{}", c.to_ascii_uppercase()),
            Key::Digit(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Named(n) => write!(f, "{}", Key::row(n).0[0]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_order_releases_the_held_modifiers_and_restores_them() {
        let keys = Hotkey::parse("Ctrl+Tab").unwrap();
        // Nothing held: just the shortcut.
        assert_eq!(
            typing_order(keys, Mods::default()),
            vec![
                Typed::Target(Modifier::Ctrl, true),
                Typed::Key(true),
                Typed::Key(false),
                Typed::Target(Modifier::Ctrl, false),
            ]
        );
        // Shift held by a trigger: no lone-Alt/Win problem, no neutral taps.
        assert_eq!(
            typing_order(keys, Mods::SHIFT),
            vec![
                Typed::Held(Modifier::Shift, false),
                Typed::Target(Modifier::Ctrl, true),
                Typed::Key(true),
                Typed::Key(false),
                Typed::Target(Modifier::Ctrl, false),
                Typed::Held(Modifier::Shift, true),
            ]
        );
        // Alt held: Alt must not go up or down on its own.
        let steps = typing_order(keys, Mods::ALT);
        assert_eq!(steps.first(), Some(&Typed::Neutral));
        assert_eq!(steps.last(), Some(&Typed::Neutral));
        assert_eq!(steps[1], Typed::Held(Modifier::Alt, false));
        assert_eq!(steps[steps.len() - 2], Typed::Held(Modifier::Alt, true));
    }

    #[test]
    fn typing_order_leaves_nothing_pressed() {
        for (keys, held) in [
            ("Ctrl+Shift+Tab", Mods::ALT),
            ("Super+L", Mods::CTRL | Mods::ALT),
            ("F5", Mods::SHIFT | Mods::SUPER),
            ("Ctrl+W", Mods::CTRL),
        ] {
            let mut down: Vec<String> = Vec::new();
            for step in typing_order(Hotkey::parse(keys).unwrap(), held) {
                let (name, pressed) = match step {
                    Typed::Held(m, p) => (format!("held {m:?}"), p),
                    Typed::Target(m, p) => (format!("target {m:?}"), p),
                    Typed::Key(p) => ("key".to_string(), p),
                    Typed::Neutral => continue,
                };
                if pressed {
                    down.push(name);
                } else {
                    down.retain(|d| *d != name);
                }
            }
            // Only the held modifiers are down at the end.
            down.sort();
            let mut expected: Vec<String> = Hotkey {
                mods: held,
                key: Key::Letter('a'),
            }
            .modifier_list()
            .into_iter()
            .map(|m| format!("held {m:?}"))
            .collect();
            expected.sort();
            assert_eq!(down, expected, "{keys} with {held:?}");
        }
    }

    #[test]
    fn keysyms_match_the_x11_table() {
        let sym = |spec: &str| Hotkey::parse(spec).unwrap().key.keysym();
        assert_eq!(sym("n"), 0x6e);
        assert_eq!(sym("N"), 0x6e, "letters are lowercase keysyms");
        assert_eq!(sym("0"), 0x30);
        assert_eq!(sym("F1"), 0xffbe);
        assert_eq!(sym("F12"), 0xffc9);
        assert_eq!(sym("Tab"), 0xff09);
        assert_eq!(sym("Page_Up"), 0xff55);
        assert_eq!(sym("Backspace"), 0xff08);
        assert_eq!(sym("Delete"), 0xffff);
    }

    #[test]
    fn parses_and_translates() {
        let h = Hotkey::parse("Super+Alt+N").unwrap();
        assert_eq!(h.mods, Mods::SUPER | Mods::ALT);
        assert_eq!(h.to_gnome(), "<Super><Alt>n");
        assert_eq!(h.to_windows(), (0x4000 | 0x1 | 0x8, 0x4E));
        assert_eq!(h.to_string(), "Super+Alt+N");
    }

    #[test]
    fn accepts_aliases_and_any_case() {
        let h = Hotkey::parse("win + CONTROL + pageup").unwrap();
        assert_eq!(h.mods, Mods::SUPER | Mods::CTRL);
        assert_eq!(h.to_gnome(), "<Super><Control>Page_Up");
        assert_eq!(h.to_windows().1, 0x21);
    }

    #[test]
    fn function_keys_digits_and_punctuation() {
        assert_eq!(Hotkey::parse("F12").unwrap().to_windows().1, 0x7B);
        assert_eq!(Hotkey::parse("Ctrl+F1").unwrap().to_gnome(), "<Control>F1");
        assert_eq!(Hotkey::parse("Alt+1").unwrap().to_windows().1, 0x31);
        assert_eq!(Hotkey::parse("Alt+`").unwrap().to_gnome(), "<Alt>grave");
        assert_eq!(Hotkey::parse("Super+]").unwrap().to_windows().1, 0xDD);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Hotkey::parse("").is_err());
        assert!(Hotkey::parse("Ctrl+").is_err());
        assert!(Hotkey::parse("Hyper+N").is_err());
        assert!(Hotkey::parse("Ctrl+NotAKey").is_err());
        assert!(Hotkey::parse("F25").is_err());
    }

    #[test]
    fn every_named_key_round_trips() {
        for (names, named, ..) in NAMED {
            let h = Hotkey::parse(&format!("Alt+{}", names[0])).unwrap();
            assert_eq!(h.key, Key::Named(*named));
            assert_eq!(Hotkey::parse(&h.to_string()).unwrap(), h);
        }
    }
}
