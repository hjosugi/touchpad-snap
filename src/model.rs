//! The platform-neutral view of the desktop that every backend produces.

use serde::{Deserialize, Serialize};

/// One top-level window, as reported by a platform backend.
///
/// GNOME fills this from the shell extension's JSON; Windows fills it from
/// `EnumWindows`. Fields a platform does not have are left at their defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    /// Opaque, stable for the lifetime of the window (Meta window id / HWND).
    pub id: u64,
    pub title: String,
    /// The app the window belongs to: a desktop-file id on GNOME
    /// (`org.gnome.Nautilus.desktop`), the executable path on Windows.
    pub app_id: String,
    pub app_name: String,
    /// X11 WM_CLASS / Wayland app_id on GNOME, the window class on Windows.
    pub wm_class: String,
    pub pid: u32,
    /// Workspace index, or -1 for a window shown on every workspace.
    pub workspace: i32,
    pub minimized: bool,
    pub focused: bool,
    /// Position in most-recently-used order, 0 being the most recent.
    pub mru: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Window {
    /// The key that decides which windows count as "the same app".
    ///
    /// GNOME hands windows it cannot match to a desktop file a synthetic,
    /// per-window app id (`window:123`); grouping by that would leave every
    /// such window alone in its group, so fall back to the window class and
    /// then to the process.
    pub fn app_key(&self) -> String {
        if !self.app_id.is_empty() && !self.app_id.starts_with("window:") {
            return self.app_id.clone();
        }
        if !self.wm_class.is_empty() {
            return format!("wm_class:{}", self.wm_class);
        }
        format!("pid:{}", self.pid)
    }

    pub fn is_on_workspace(&self, workspace: i32) -> bool {
        self.workspace < 0 || self.workspace == workspace
    }
}

/// Everything a backend reports in one go.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub active_workspace: i32,
    pub windows: Vec<Window>,
}

impl Snapshot {
    pub fn focused(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.focused)
    }

    pub fn get(&self, id: u64) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_key_prefers_desktop_id() {
        let w = Window {
            app_id: "org.gnome.TextEditor.desktop".into(),
            wm_class: "gnome-text-editor".into(),
            pid: 7,
            ..Default::default()
        };
        assert_eq!(w.app_key(), "org.gnome.TextEditor.desktop");
    }

    #[test]
    fn app_key_skips_synthetic_gnome_ids() {
        let w = Window {
            app_id: "window:42".into(),
            wm_class: "foo".into(),
            pid: 7,
            ..Default::default()
        };
        assert_eq!(w.app_key(), "wm_class:foo");

        let bare = Window {
            pid: 7,
            ..Default::default()
        };
        assert_eq!(bare.app_key(), "pid:7");
    }

    #[test]
    fn parses_extension_json_with_unknown_fields() {
        let json = r#"{"version":1,"active_workspace":2,"windows":[
            {"id":5,"title":"a","app_id":"x.desktop","focused":true,"mru":0,"extra":1}
        ]}"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.active_workspace, 2);
        assert_eq!(snap.focused().unwrap().id, 5);
    }
}
