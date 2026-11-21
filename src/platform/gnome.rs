//! GNOME Shell (Wayland and X11) through the touchpad-snap shell extension.
//!
//! Wayland gives clients no way to list or focus other clients' windows, grab
//! global keys or see touchpad gestures meant for the shell. The extension
//! (gnome-extension/ in this repository) runs inside the compositor and
//! offers all of that on the session bus:
//!
//!   bus name   io.github.hjosugi.TouchpadSnap
//!   object     /io/github/hjosugi/TouchpadSnap
//!   interface  io.github.hjosugi.TouchpadSnap1
//!
//! See gnome-extension/README.md for the protocol.

use std::sync::mpsc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type as MessageType;
use zbus::{MatchRule, Message};

use super::{Backend, Event};
use crate::config::{Claim, Config};
use crate::gesture::Phase;
use crate::hotkey::{Hotkey, Mods};
use crate::model::Snapshot;

pub const BUS_NAME: &str = "io.github.hjosugi.TouchpadSnap";
pub const OBJECT_PATH: &str = "/io/github/hjosugi/TouchpadSnap";
pub const INTERFACE: &str = "io.github.hjosugi.TouchpadSnap1";
/// The extension protocol this binary speaks. Version 2 added ShowHud,
/// version 3 the AcceleratorReleased signal. Older extensions still work,
/// without the overlay (1) or with MRU sessions going by time (2).
pub const PROTOCOL_VERSION: u32 = 3;
pub const EXTENSION_UUID: &str = "touchpad-snap@hjosugi.github.io";

pub struct Gnome {
    conn: Connection,
    proxy: Proxy<'static>,
}

impl Gnome {
    pub fn connect() -> Result<Self> {
        let conn = Connection::session().context("connecting to the D-Bus session bus")?;
        let proxy = zbus::blocking::proxy::Builder::new(&conn)
            .destination(BUS_NAME)?
            .path(OBJECT_PATH)?
            .interface(INTERFACE)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()?;
        Ok(Self { conn, proxy })
    }

    pub fn protocol_version(&self) -> Result<u32> {
        self.proxy
            .get_property::<u32>("ProtocolVersion")
            .map_err(explain)
    }

    /// Whether the extension currently owns its bus name and answers.
    pub fn extension_available() -> bool {
        Gnome::connect()
            .and_then(|gnome| gnome.protocol_version())
            .is_ok()
    }

    /// Pushes hotkeys and swipe claims to the extension. It keeps them only
    /// while this connection is alive. Returns the extension's protocol
    /// version. With `portal` the hotkeys are left to the GlobalShortcuts
    /// portal; the extension only gets the swipe claims (and any old grabs
    /// are cleared).
    fn register(&self, cfg: &Config, hotkeys: &[Hotkey], portal: bool) -> Result<u32> {
        let version = self.protocol_version()?;
        if version < PROTOCOL_VERSION {
            let missing = if version < 2 {
                "the overlay after a jump, and hotkey sessions that end when the modifiers are released"
            } else {
                "hotkey sessions that end when the modifiers are released (they go by time instead)"
            };
            eprintln!(
                "touchpad-snap: the GNOME extension is older (protocol {version}, this binary {PROTOCOL_VERSION}); \
                 everything works except {missing}. Update the extension and log in again."
            );
        } else if version > PROTOCOL_VERSION {
            eprintln!(
                "touchpad-snap: the GNOME extension is newer (protocol {version}, this binary {PROTOCOL_VERSION}); update touchpad-snap"
            );
        }

        let mut failed: Vec<String> = Vec::new();
        if portal {
            // Move any previous extension grabs out of the portal's way.
            let empty: Vec<(String, String)> = Vec::new();
            let _: Vec<String> = self
                .proxy
                .call("SetAccelerators", &(empty,))
                .map_err(explain)?;
        } else {
            let bindings: Vec<(String, String)> = hotkeys
                .iter()
                .enumerate()
                .map(|(i, h)| (h.to_gnome(), format!("hotkey:{i}")))
                .collect();
            failed = self
                .proxy
                .call("SetAccelerators", &(bindings,))
                .map_err(explain)?;
            for accel in &failed {
                eprintln!(
                    "touchpad-snap: could not grab {accel}: GNOME or another app already uses it (check Settings → Keyboard → Shortcuts)"
                );
            }
        }

        let claims: Vec<(u32, u32)> = cfg.claims().iter().map(|c| (c.fingers, c.mods.0)).collect();
        self.proxy
            .call_method("SetSwipeClaims", &(claims,))
            .map_err(explain)?;

        if portal {
            eprintln!(
                "touchpad-snap: registered {} swipe claim(s) with GNOME Shell (hotkeys through the portal)",
                cfg.claims().len()
            );
        } else {
            eprintln!(
                "touchpad-snap: registered {} hotkey(s) and {} swipe claim(s) with GNOME Shell",
                hotkeys.len() - failed.len(),
                cfg.claims().len()
            );
        }
        Ok(version)
    }
}

impl Backend for Gnome {
    fn snapshot(&self) -> Result<Snapshot> {
        let json: String = self.proxy.call("List", &()).map_err(explain)?;
        serde_json::from_str(&json).context("parsing the window list from the GNOME extension")
    }

    fn activate(&self, id: u64) -> Result<()> {
        let ok: bool = self.proxy.call("Activate", &(id,)).map_err(explain)?;
        if !ok {
            bail!("window {id} is gone");
        }
        Ok(())
    }

    fn show_hud(&self, id: u64, index: usize, total: usize, duration_ms: u64) -> Result<()> {
        let args = (
            id,
            index as u32,
            total as u32,
            duration_ms.min(u32::MAX as u64) as u32,
        );
        match self.proxy.call::<_, _, bool>("ShowHud", &args) {
            Ok(_) => Ok(()),
            // A protocol 1 extension: no overlay, and nothing to complain about.
            Err(e) if e.to_string().contains("UnknownMethod") => Ok(()),
            Err(e) => Err(explain(e)),
        }
    }

    fn describe(&self) -> String {
        match self.protocol_version() {
            Ok(v) => format!("GNOME Shell extension {EXTENSION_UUID} (protocol {v})"),
            Err(e) => format!("GNOME Shell extension not reachable: {e:#}"),
        }
    }
}

/// Turns "nobody owns the name" into advice.
fn explain(e: zbus::Error) -> anyhow::Error {
    let text = e.to_string();
    if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        anyhow!(
            "the touchpad-snap GNOME Shell extension is not running.\n  \
             Enable it with: gnome-extensions enable {EXTENSION_UUID}\n  \
             (After installing it for the first time, log out and back in: GNOME on Wayland only discovers new extensions at login.)"
        )
    } else {
        anyhow::Error::new(e)
    }
}

enum Incoming {
    Signal(Message),
    /// An `Activated`/`Deactivated` signal from the GlobalShortcuts portal.
    Portal(Message),
    /// The extension's bus name got a new owner (true) or lost it (false).
    Owner(bool),
}

pub fn run_daemon(
    cfg: &Config,
    hotkeys: &[Hotkey],
    verbose: bool,
    handle: &mut super::Handler,
) -> Result<()> {
    run_daemon_with(cfg, hotkeys, verbose, false, handle)
}

/// Like [`run_daemon`], but the hotkeys go through the XDG GlobalShortcuts
/// portal; the extension still provides swipes and the HUD.
pub fn run_daemon_with_portal(
    cfg: &Config,
    hotkeys: &[Hotkey],
    verbose: bool,
    handle: &mut super::Handler,
) -> Result<()> {
    run_daemon_with(cfg, hotkeys, verbose, true, handle)
}

fn run_daemon_with(
    cfg: &Config,
    hotkeys: &[Hotkey],
    verbose: bool,
    use_portal: bool,
    handle: &mut super::Handler,
) -> Result<()> {
    let mut cfg = cfg.clone();
    let mut hotkeys = hotkeys.to_vec();
    let gnome = Gnome::connect()?;
    super::claim_daemon_name(&gnome.conn)?;

    // Subscribe before the first registration so no owner change slips by.
    let signals = MessageIterator::for_match_rule(
        MatchRule::builder()
            .msg_type(MessageType::Signal)
            .path(OBJECT_PATH)?
            .interface(INTERFACE)?
            .build(),
        &gnome.conn,
        Some(256),
    )?;
    let owners = MessageIterator::for_match_rule(
        MatchRule::builder()
            .msg_type(MessageType::Signal)
            .sender("org.freedesktop.DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .add_arg(BUS_NAME)?
            .build(),
        &gnome.conn,
        Some(16),
    )?;

    let (tx, rx) = mpsc::channel();
    let signal_tx = tx.clone();
    std::thread::spawn(move || {
        for msg in signals.flatten() {
            if signal_tx.send(Incoming::Signal(msg)).is_err() {
                break;
            }
        }
    });
    let owner_tx = tx.clone();
    std::thread::spawn(move || {
        for msg in owners.flatten() {
            if let Ok((_, _, new_owner)) = msg.body().deserialize::<(String, String, String)>()
                && owner_tx
                    .send(Incoming::Owner(!new_owner.is_empty()))
                    .is_err()
            {
                break;
            }
        }
    });

    // The extension's protocol version, once registered: from 3 on it reports
    // when a hotkey's modifiers are released.
    let mut version = match gnome.register(&cfg, &hotkeys, use_portal) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("touchpad-snap: waiting for the GNOME Shell extension: {e:#}");
            0
        }
    };

    // Portal hotkeys, when requested. Registering after the swipes means they
    // keep working while the desktop's confirmation dialog is open. A user who
    // declines keeps a usable daemon (swipes, HUD, config reload) without
    // global hotkeys.
    let mut portal_session = String::new();
    if use_portal {
        let shortcuts =
            super::portal::Shortcuts::register_on(gnome.conn.clone(), &cfg, &hotkeys, verbose)?;
        if let Some(shortcuts) = shortcuts {
            portal_session = shortcuts.session_handle().as_str().to_string();
            let portal_signals = shortcuts.signals()?;
            let portal_tx = tx.clone();
            std::thread::spawn(move || {
                for msg in portal_signals.flatten() {
                    if portal_tx.send(Incoming::Portal(msg)).is_err() {
                        break;
                    }
                }
            });
        } else {
            eprintln!("touchpad-snap: running without global hotkeys");
        }
    }

    let mut last_tick = Instant::now();
    loop {
        let wait = super::TICK.saturating_sub(last_tick.elapsed());
        let incoming = match rx.recv_timeout(wait) {
            Ok(incoming) => Some(incoming),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        // Ticks are due by the clock, not by silence, so a stream of swipe
        // events cannot starve them.
        let mut changed = None;
        if last_tick.elapsed() >= super::TICK {
            last_tick = Instant::now();
            changed = handle(&gnome, Event::Tick);
        }
        match incoming {
            Some(Incoming::Owner(true)) => {
                // The shell restarted or the extension was re-enabled; it has
                // forgotten our hotkeys and claims.
                match gnome.register(&cfg, &hotkeys, use_portal) {
                    Ok(v) => version = v,
                    Err(e) => {
                        eprintln!("touchpad-snap: registering with the GNOME extension failed: {e:#}")
                    }
                }
                // A release still pending from the old shell will never come.
                changed = changed.or(handle(&gnome, Event::Released));
            }
            Some(Incoming::Owner(false)) => {
                eprintln!("touchpad-snap: the GNOME extension went away; waiting for it to return");
            }
            Some(Incoming::Signal(msg)) => {
                if let Some(event) = parse_signal(&msg, version, &hotkeys) {
                    changed = changed.or(handle(&gnome, event));
                }
            }
            Some(Incoming::Portal(msg)) => {
                if let Some(event) = super::portal::parse_signal(&msg, &portal_session, &hotkeys) {
                    changed = changed.or(handle(&gnome, event));
                }
            }
            None => {}
        }
        if let Some(new) = changed {
            match new.validate() {
                Ok(parsed) => {
                    cfg = new;
                    hotkeys = parsed;
                    match gnome.register(&cfg, &hotkeys, use_portal) {
                        Ok(v) => version = v,
                        Err(e) => eprintln!(
                            "touchpad-snap: registering with the GNOME extension failed: {e:#}"
                        ),
                    }
                    if use_portal {
                        eprintln!(
                            "touchpad-snap: config reloaded; GlobalShortcuts bindings change after a restart"
                        );
                    }
                }
                Err(e) => eprintln!("touchpad-snap: ignoring an invalid config: {e:#}"),
            }
        }
    }
    Ok(())
}

/// `version` is the extension's protocol version, `hotkeys` what was
/// registered with it.
fn parse_signal(msg: &Message, version: u32, hotkeys: &[Hotkey]) -> Option<Event> {
    let header = msg.header();
    let member = header.member()?;
    match member.as_str() {
        "AcceleratorActivated" => {
            let (action,): (String,) = msg.body().deserialize().ok()?;
            let index: usize = action.strip_prefix("hotkey:")?.parse().ok()?;
            // Without modifiers there is nothing to hold.
            let tracked = version >= 3 && hotkeys.get(index).is_some_and(|h| !h.mods.is_empty());
            Some(Event::Hotkey { index, tracked })
        }
        "AcceleratorReleased" => Some(Event::Released),
        "Swipe" => {
            let (phase, fingers, mods, dx, dy): (u32, u32, u32, f64, f64) =
                msg.body().deserialize().ok()?;
            Some(Event::Swipe {
                phase: Phase::from_u32(phase)?,
                claim: Claim {
                    fingers,
                    mods: Mods(mods),
                },
                dx,
                dy,
            })
        }
        _ => None,
    }
}
