//! Persists [`CycleState`] between jumps.
//!
//! The CLI is a new process for every key press, so the state that makes MRU
//! sessions and the focus hint work lives in a small file in the runtime
//! directory. The daemon uses the same file, so mixing the two behaves.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cycle::CycleState;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn default_path() -> PathBuf {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    base.join("window-jump").join("state.json")
}

/// Loads the state. A missing or unreadable file is simply "no state".
pub fn load(path: &Path) -> Option<CycleState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Saves the state. Failures are logged, never fatal: the worst outcome is
/// that the next jump starts a fresh session.
pub fn save(path: &Path, state: &Option<CycleState>) {
    let result = (|| -> std::io::Result<()> {
        match state {
            Some(s) => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, serde_json::to_vec(s)?)?;
                std::fs::rename(tmp, path)
            }
            None => match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        }
    })();
    if let Err(e) = result {
        eprintln!(
            "window-jump: could not save state to {}: {e}",
            path.display()
        );
    }
}
