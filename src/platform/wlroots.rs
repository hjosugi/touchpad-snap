//! Sway, Hyprland and niri through their native IPC.
//!
//! These compositors expose the window list and focus over their own UNIX
//! socket, so no shell extension is needed. Global hotkeys have no portable
//! API on wlroots, so they are bound in the compositor configuration and the
//! daemon is optional.

use anyhow::{Result, bail};

use super::Backend;
#[cfg(feature = "libinput")]
use super::Event as DaemonEvent;
use crate::config::Config;
use crate::model::Snapshot;

mod hyprland;
mod niri;
mod sway;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compositor {
    Sway,
    Hyprland,
    Niri,
}

impl Compositor {
    fn name(self) -> &'static str {
        match self {
            Compositor::Sway => "Sway",
            Compositor::Hyprland => "Hyprland",
            Compositor::Niri => "niri",
        }
    }
}

/// Which compositor is running, from its socket variable, falling back to
/// `XDG_CURRENT_DESKTOP` to explain a missing socket.
///
/// `get` looks up an environment variable; passing it in keeps detection
/// testable without touching the process environment.
pub fn detect<F>(get: F) -> Result<Compositor>
where
    F: Fn(&str) -> Option<String>,
{
    let present = |key: &str| get(key).filter(|value| !value.trim().is_empty());
    if present("SWAYSOCK").is_some() {
        return Ok(Compositor::Sway);
    }
    if present("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Ok(Compositor::Hyprland);
    }
    if present("NIRI_SOCKET").is_some() {
        return Ok(Compositor::Niri);
    }

    match desktop_hint(&get) {
        Some(compositor) => {
            let variable = match compositor {
                Compositor::Sway => "SWAYSOCK",
                Compositor::Hyprland => "HYPRLAND_INSTANCE_SIGNATURE",
                Compositor::Niri => "NIRI_SOCKET",
            };
            bail!(
                "{} detected from XDG_CURRENT_DESKTOP, but {variable} is not set; \
                 start window-jump inside the compositor session",
                compositor.name()
            )
        }
        None => bail!(
            "no supported wlroots compositor found (Sway, Hyprland or niri); \
             expected SWAYSOCK, HYPRLAND_INSTANCE_SIGNATURE or NIRI_SOCKET"
        ),
    }
}

fn desktop_hint<F>(get: &F) -> Option<Compositor>
where
    F: Fn(&str) -> Option<String>,
{
    let desktop = get("XDG_CURRENT_DESKTOP")?;
    for part in desktop.split([':', ';']) {
        let part = part.trim();
        if part.eq_ignore_ascii_case("sway") {
            return Some(Compositor::Sway);
        }
        if part.eq_ignore_ascii_case("hyprland") {
            return Some(Compositor::Hyprland);
        }
        if part.eq_ignore_ascii_case("niri") {
            return Some(Compositor::Niri);
        }
    }
    None
}

pub fn detect_from_env() -> Result<Compositor> {
    detect(|key| std::env::var(key).ok())
}

pub fn desktop_hint_from_env() -> Option<Compositor> {
    let get = |key: &str| std::env::var(key).ok();
    desktop_hint(&get)
}

enum Inner {
    Sway(sway::Sway),
    Hyprland(hyprland::Hyprland),
    Niri(niri::Niri),
}

pub struct Wlroots {
    inner: Inner,
}

impl Wlroots {
    pub fn connect() -> Result<Self> {
        let inner = match detect_from_env()? {
            Compositor::Sway => Inner::Sway(sway::Sway::connect()?),
            Compositor::Hyprland => Inner::Hyprland(hyprland::Hyprland::connect()?),
            Compositor::Niri => Inner::Niri(niri::Niri::connect()?),
        };
        Ok(Self { inner })
    }
}

impl Backend for Wlroots {
    fn snapshot(&self) -> Result<Snapshot> {
        match &self.inner {
            Inner::Sway(backend) => backend.snapshot(),
            Inner::Hyprland(backend) => backend.snapshot(),
            Inner::Niri(backend) => backend.snapshot(),
        }
    }

    fn activate(&self, id: u64) -> Result<()> {
        match &self.inner {
            Inner::Sway(backend) => backend.activate(id),
            Inner::Hyprland(backend) => backend.activate(id),
            Inner::Niri(backend) => backend.activate(id),
        }
    }

    fn describe(&self) -> String {
        match &self.inner {
            Inner::Sway(backend) => backend.describe(),
            Inner::Hyprland(backend) => backend.describe(),
            Inner::Niri(backend) => backend.describe(),
        }
    }
}

/// There is no portable global-key or gesture API on wlroots. Hotkeys belong
/// in the compositor config; say so once. Without the `libinput` feature the
/// daemon then returns instead of blocking. With it, the daemon stays for
/// touchpad swipes read straight from libinput (and still reloads the config).
pub fn run_daemon(cfg: &Config, verbose: bool, handle: &mut super::Handler) -> Result<()> {
    eprintln!(
        "window-jump: this compositor has no portable global-hotkey API; \
         bind `window-jump next` / `window-jump prev` in its config instead \
         (for example `bindsym $mod+n exec window-jump next`). \
         The daemon is not needed for hotkeys."
    );
    #[cfg(feature = "libinput")]
    {
        eprintln!("window-jump: serving configured touchpad swipes through libinput");
        run_gestures(cfg, verbose, handle)
    }
    #[cfg(not(feature = "libinput"))]
    {
        let _ = (cfg, verbose, handle);
        Ok(())
    }
}

/// The daemon loop used when libinput gestures are compiled in: no hotkeys to
/// register, just gesture events and the config-reload tick.
#[cfg(feature = "libinput")]
fn run_gestures(cfg: &Config, verbose: bool, handle: &mut super::Handler) -> Result<()> {
    use std::sync::mpsc;
    use std::time::Instant;

    let backend = Wlroots::connect()?;
    let (tx, rx) = mpsc::channel();
    // Keep the channel connected even if the libinput thread stops (no seat,
    // no permission); the daemon then keeps checking the config.
    let keepalive = tx.clone();
    let gestures = super::libinput::GestureSource::spawn(cfg.claims(), tx, verbose);
    let _keepalive = keepalive;

    let mut last_tick = Instant::now();
    loop {
        let wait = super::TICK.saturating_sub(last_tick.elapsed());
        let incoming = match rx.recv_timeout(wait) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut changed = None;
        if last_tick.elapsed() >= super::TICK {
            last_tick = Instant::now();
            changed = handle(&backend, DaemonEvent::Tick);
        }
        if let Some(event) = incoming {
            changed = changed.or(handle(&backend, event));
        }
        if let Some(new) = changed {
            gestures.set_claims(new.claims());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn socket_variables_win_in_order() {
        let all = lookup(&[
            ("SWAYSOCK", "/run/user/1000/sway-ipc.1000.1.sock"),
            ("HYPRLAND_INSTANCE_SIGNATURE", "sig"),
            ("NIRI_SOCKET", "/run/user/1000/niri.wayland-1.sock"),
        ]);
        assert_eq!(detect(all).unwrap(), Compositor::Sway);

        let hypr = lookup(&[
            ("HYPRLAND_INSTANCE_SIGNATURE", "sig"),
            ("NIRI_SOCKET", "/run/user/1000/niri.wayland-1.sock"),
        ]);
        assert_eq!(detect(hypr).unwrap(), Compositor::Hyprland);

        let niri = lookup(&[("NIRI_SOCKET", "/run/user/1000/niri.wayland-1.sock")]);
        assert_eq!(detect(niri).unwrap(), Compositor::Niri);
    }

    #[test]
    fn empty_socket_variable_does_not_count() {
        let env = lookup(&[
            ("SWAYSOCK", ""),
            ("HYPRLAND_INSTANCE_SIGNATURE", "  "),
            ("NIRI_SOCKET", "/run/user/1000/niri.sock"),
        ]);
        assert_eq!(detect(env).unwrap(), Compositor::Niri);
    }

    #[test]
    fn desktop_names_name_the_missing_socket() {
        for (desktop, variable) in [
            ("sway", "SWAYSOCK"),
            ("Hyprland", "HYPRLAND_INSTANCE_SIGNATURE"),
            ("niri", "NIRI_SOCKET"),
        ] {
            let err = detect(lookup(&[("XDG_CURRENT_DESKTOP", desktop)])).unwrap_err();
            let message = err.to_string();
            assert!(message.contains(variable), "{message}");
        }
        let err = detect(lookup(&[("XDG_CURRENT_DESKTOP", "sway:GNOME")])).unwrap_err();
        assert!(err.to_string().contains("SWAYSOCK"));
    }

    #[test]
    fn no_compositor_is_a_clear_error() {
        let err = detect(lookup(&[])).unwrap_err();
        assert!(err.to_string().contains("Sway, Hyprland or niri"));
        assert!(err.to_string().contains("SWAYSOCK"));

        let gnome = detect(lookup(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")])).unwrap_err();
        assert!(
            gnome
                .to_string()
                .contains("no supported wlroots compositor")
        );
    }
}
