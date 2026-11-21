//! GNOME Shell (Wayland and X11) through the window-jump shell extension.
//!
//! Wayland gives clients no way to list or focus other clients' windows, grab
//! global keys or see touchpad gestures meant for the shell. The extension
//! (gnome-extension/ in this repository) runs inside the compositor and
//! offers all of that on the session bus:
//!
//!   bus name   org.hjosugi.WindowJump
//!   object     /org/hjosugi/WindowJump
//!   interface  org.hjosugi.WindowJump1
//!
//! See gnome-extension/README.md for the protocol.

use std::sync::mpsc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::fdo::RequestNameFlags;
use zbus::message::Type as MessageType;
use zbus::{MatchRule, Message};

use super::{Backend, Event};
use crate::config::{Claim, Config};
use crate::gesture::Phase;
use crate::hotkey::{Hotkey, Mods};
use crate::model::Snapshot;

pub const BUS_NAME: &str = "org.hjosugi.WindowJump";
pub const OBJECT_PATH: &str = "/org/hjosugi/WindowJump";
pub const INTERFACE: &str = "org.hjosugi.WindowJump1";
/// The extension protocol this binary speaks. Version 2 added ShowHud; a
/// version 1 extension still works, without the overlay.
pub const PROTOCOL_VERSION: u32 = 2;
pub const EXTENSION_UUID: &str = "window-jump@hjosugi.github.io";

/// Held by the running daemon so a second one refuses to start.
const DAEMON_NAME: &str = "org.hjosugi.WindowJump.Daemon";

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

    /// Pushes hotkeys and swipe claims to the extension. It keeps them only
    /// while this connection is alive.
    fn register(&self, cfg: &Config, hotkeys: &[Hotkey]) -> Result<()> {
        let version = self.protocol_version()?;
        if version < PROTOCOL_VERSION {
            eprintln!(
                "window-jump: the GNOME extension is older (protocol {version}, this binary {PROTOCOL_VERSION}); \
                 everything works except the overlay after a jump. Update the extension and log in again."
            );
        } else if version > PROTOCOL_VERSION {
            eprintln!(
                "window-jump: the GNOME extension is newer (protocol {version}, this binary {PROTOCOL_VERSION}); update window-jump"
            );
        }

        let bindings: Vec<(String, String)> = hotkeys
            .iter()
            .enumerate()
            .map(|(i, h)| (h.to_gnome(), format!("hotkey:{i}")))
            .collect();
        let failed: Vec<String> = self
            .proxy
            .call("SetAccelerators", &(bindings,))
            .map_err(explain)?;
        for accel in &failed {
            eprintln!(
                "window-jump: could not grab {accel}: GNOME or another app already uses it (check Settings → Keyboard → Shortcuts)"
            );
        }

        let claims: Vec<(u32, u32)> = cfg.claims().iter().map(|c| (c.fingers, c.mods.0)).collect();
        self.proxy
            .call_method("SetSwipeClaims", &(claims,))
            .map_err(explain)?;

        eprintln!(
            "window-jump: registered {} hotkey(s) and {} swipe claim(s) with GNOME Shell",
            hotkeys.len() - failed.len(),
            cfg.claims().len()
        );
        Ok(())
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
            "the window-jump GNOME Shell extension is not running.\n  \
             Enable it with: gnome-extensions enable {EXTENSION_UUID}\n  \
             (After installing it for the first time, log out and back in: GNOME on Wayland only discovers new extensions at login.)"
        )
    } else {
        anyhow::Error::new(e)
    }
}

enum Incoming {
    Signal(Message),
    /// The extension's bus name got a new owner (true) or lost it (false).
    Owner(bool),
}

pub fn run_daemon(cfg: &Config, hotkeys: &[Hotkey], handle: &mut super::Handler) -> Result<()> {
    let mut cfg = cfg.clone();
    let mut hotkeys = hotkeys.to_vec();
    let gnome = Gnome::connect()?;
    match gnome
        .conn
        .request_name_with_flags(DAEMON_NAME, RequestNameFlags::DoNotQueue.into())
    {
        Err(zbus::Error::NameTaken) => bail!("another window-jump daemon is already running"),
        Err(e) => return Err(e).context("claiming the daemon bus name"),
        Ok(_) => {}
    }

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
    std::thread::spawn(move || {
        for msg in owners.flatten() {
            if let Ok((_, _, new_owner)) = msg.body().deserialize::<(String, String, String)>()
                && tx.send(Incoming::Owner(!new_owner.is_empty())).is_err()
            {
                break;
            }
        }
    });

    if let Err(e) = gnome.register(&cfg, &hotkeys) {
        eprintln!("window-jump: waiting for the GNOME Shell extension: {e:#}");
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
                if let Err(e) = gnome.register(&cfg, &hotkeys) {
                    eprintln!("window-jump: registering with the GNOME extension failed: {e:#}");
                }
            }
            Some(Incoming::Owner(false)) => {
                eprintln!("window-jump: the GNOME extension went away; waiting for it to return");
            }
            Some(Incoming::Signal(msg)) => {
                if let Some(event) = parse_signal(&msg) {
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
                    if let Err(e) = gnome.register(&cfg, &hotkeys) {
                        eprintln!(
                            "window-jump: registering with the GNOME extension failed: {e:#}"
                        );
                    }
                }
                Err(e) => eprintln!("window-jump: ignoring an invalid config: {e:#}"),
            }
        }
    }
    Ok(())
}

fn parse_signal(msg: &Message) -> Option<Event> {
    let header = msg.header();
    let member = header.member()?;
    match member.as_str() {
        "AcceleratorActivated" => {
            let (action,): (String,) = msg.body().deserialize().ok()?;
            let index = action.strip_prefix("hotkey:")?.parse().ok()?;
            Some(Event::Hotkey(index))
        }
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
