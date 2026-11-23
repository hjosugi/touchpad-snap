//! Global hotkeys through the XDG GlobalShortcuts portal.
//!
//! `org.freedesktop.portal.GlobalShortcuts` is implemented by KDE Plasma and
//! by GNOME 48 and newer. It gives us hotkeys on KDE (whose window list comes
//! from the KWin script in [`super::kde`]) and an alternative to the GNOME
//! extension's accelerator grabs: the desktop shows a confirmation dialog and
//! keeps the trigger strings itself.
//!
//! The portal belongs to the session bus:
//!
//!   bus name   org.freedesktop.portal.Desktop
//!   object     /org/freedesktop/portal/desktop
//!   interface  org.freedesktop.portal.GlobalShortcuts
//!
//! Unsandboxed applications associate their connection with an app id through
//! `org.freedesktop.host.portal.Registry.Register` (xdg-desktop-portal 1.20+);
//! the id should match a `.desktop` file. That call is made before any other
//! portal method, and its absence is not fatal on older portals.
//!
//! Triggers use the [shortcuts XDG specification]: `CTRL+ALT+SHIFT+LOGO+a`,
//! `LOGO+Return`, `ALT+Page_Up`. Modifiers are upper-case; the key is an
//! xkbcommon keysym name without the `XKB_KEY_` prefix, which is exactly what
//! [`crate::hotkey::Hotkey::to_gnome`] already produces apart from the
//! modifier spelling.
//!
//! [shortcuts XDG specification]: https://specifications.freedesktop.org/shortcuts-spec/latest/

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type as MessageType;
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Value};
use zbus::{MatchRule, Message};

use super::Event;
use crate::config::{Action, Config};
use crate::hotkey::{Hotkey, Mods};

pub const BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const REGISTRY_INTERFACE: &str = "org.freedesktop.host.portal.Registry";

/// A stable application id. It should match a `.desktop` file's basename for
/// desktops that require one.
pub const APP_ID: &str = "io.github.hjosugi.TouchpadSnap";

const SHORTCUT_PREFIX: &str = "hotkey:";

/// The trigger string sent as `preferred_trigger`, e.g. `ALT+LOGO+n`.
///
/// The mapping is total: every [`Key`](crate::hotkey::Key) variant has an
/// xkbcommon keysym name through [`Hotkey::to_gnome`].
pub fn trigger(hotkey: &Hotkey) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(5);
    if hotkey.mods.contains(Mods::CTRL) {
        parts.push("CTRL");
    }
    if hotkey.mods.contains(Mods::ALT) {
        parts.push("ALT");
    }
    if hotkey.mods.contains(Mods::SHIFT) {
        parts.push("SHIFT");
    }
    if hotkey.mods.contains(Mods::SUPER) {
        parts.push("LOGO");
    }
    let key = hotkey.key.gnome_name();
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(&key);
    s
}

/// The portal id of the binding at `index`. Stable across restarts so desktops
/// can remember the user's trigger.
pub fn shortcut_id(index: usize) -> String {
    format!("{SHORTCUT_PREFIX}{index}")
}

/// The binding index for a portal shortcut id.
pub fn shortcut_index(id: &str) -> Option<usize> {
    id.strip_prefix(SHORTCUT_PREFIX)?.parse().ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIdStatus {
    /// The portal accepted the registration.
    Registered,
    /// The portal is too old for the Registry interface or refused it; the
    /// session may still work through `.desktop`/platform defaults.
    Ignored,
}

/// The result of a request's `Response` signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Response {
    Accepted,
    Declined(u32),
}

fn response_status(response: u32) -> Response {
    if response == 0 {
        Response::Accepted
    } else {
        Response::Declined(response)
    }
}

/// Reads the session handle out of a `CreateSession` response. The value is a
/// string holding an object path (the portal documents this quirk).
fn session_handle_from(results: &HashMap<String, OwnedValue>) -> Result<OwnedObjectPath> {
    let value = results
        .get("session_handle")
        .ok_or_else(|| anyhow!("the portal's CreateSession response had no session_handle"))?;
    let handle: &str = value
        .try_into()
        .context("the portal's session_handle was not a string")?;
    OwnedObjectPath::try_from(handle.to_string())
        .context("the portal's session_handle is not an object path")
}

fn next_token(kind: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    // A D-Bus object path element: only [A-Za-z0-9_].
    format!("touchpad_snap_{kind}_{}_{nanos}_{n}", std::process::id())
}

fn dbus_error_name(e: &zbus::Error) -> Option<String> {
    match e {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str().to_string()),
        _ => None,
    }
}

fn is_missing(error: &zbus::Error, suffix: &str) -> bool {
    dbus_error_name(error).is_some_and(|n| n.ends_with(suffix))
}

/// Associates this D-Bus connection with [`APP_ID`] before any portal method
/// call. Newer portals require it for unsandboxed applications; older ones
/// answer `UnknownMethod`, which is fine.
pub fn register_app_id(conn: &Connection, verbose: bool) -> Result<AppIdStatus> {
    let options: HashMap<String, Value<'static>> = HashMap::new();
    match conn.call_method(
        Some(BUS_NAME),
        OBJECT_PATH,
        Some(REGISTRY_INTERFACE),
        "Register",
        &(APP_ID, options),
    ) {
        Ok(_) => {
            if verbose {
                eprintln!("touchpad-snap: registered app id {APP_ID} with the desktop portal");
            }
            Ok(AppIdStatus::Registered)
        }
        Err(e) if is_missing(&e, "UnknownMethod") || is_missing(&e, "UnknownInterface") => {
            if verbose {
                eprintln!(
                    "touchpad-snap: the desktop portal has no app id registration ({}); continuing",
                    dbus_error_name(&e).unwrap_or_default()
                );
            }
            Ok(AppIdStatus::Ignored)
        }
        Err(e) if is_missing(&e, "AccessDenied") => {
            if verbose {
                eprintln!(
                    "touchpad-snap: the desktop portal refused app id registration; continuing"
                );
            }
            Ok(AppIdStatus::Ignored)
        }
        Err(e) if is_missing(&e, "ServiceUnknown") || is_missing(&e, "NameHasNoOwner") => Err(
            anyhow!("the XDG desktop portal is not running, so GlobalShortcuts is unavailable"),
        ),
        Err(e) => {
            if verbose {
                eprintln!("touchpad-snap: app id registration failed: {e}");
            }
            Ok(AppIdStatus::Ignored)
        }
    }
}

/// The GlobalShortcuts interface version, used by `doctor` as a liveness test.
pub fn shortcuts_version(conn: &Connection) -> Result<u32> {
    let proxy: zbus::blocking::Proxy<'_> = zbus::blocking::proxy::Builder::new(conn)
        .destination(BUS_NAME)?
        .path(OBJECT_PATH)?
        .interface(INTERFACE)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()?;
    proxy
        .get_property::<u32>("version")
        .map_err(|e| anyhow!("the GlobalShortcuts interface is unavailable: {e}"))
}

/// One line for `touchpad-snap doctor`; never fails.
pub fn doctor() -> String {
    let conn = match Connection::session() {
        Ok(conn) => conn,
        Err(e) => return format!("unavailable ({e})"),
    };
    let app_id = match register_app_id(&conn, false) {
        Ok(status) => status,
        Err(e) => return format!("unavailable ({e:#})"),
    };
    match shortcuts_version(&conn) {
        Ok(version) => {
            let app_id = match app_id {
                AppIdStatus::Registered => "app id registered",
                AppIdStatus::Ignored => "app id not registered (portal may not require it)",
            };
            format!("available (GlobalShortcuts v{version}; {app_id})")
        }
        Err(e) => format!("unavailable ({e:#})"),
    }
}

/// Predicted `org.freedesktop.portal.Request` path:
/// `/org/freedesktop/portal/desktop/request/SENDER/TOKEN`, with the unique
/// name's leading `:` removed and `.` replaced by `_`.
fn predicted_request_path(conn: &Connection, token: &str) -> Result<OwnedObjectPath> {
    let unique = conn
        .unique_name()
        .ok_or_else(|| anyhow!("the session bus connection has no unique name"))?;
    let sender = unique.as_str().trim_start_matches(':').replace('.', "_");
    let path = format!("{OBJECT_PATH}/request/{sender}/{token}");
    OwnedObjectPath::try_from(path).context("building the portal request path")
}

fn response_rule(path: Option<&str>) -> Result<zbus::OwnedMatchRule> {
    let mut builder = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender(BUS_NAME)?
        .interface(REQUEST_INTERFACE)?
        .member("Response")?;
    if let Some(path) = path {
        builder = builder.path(path)?;
    }
    Ok(builder.build().into())
}

/// Calls `method`, then waits for the request's `Response` signal.
///
/// The match rules are installed before the call so no signal can be missed.
/// The documented predictable request path is preferred; a broad rule filtered
/// by the returned handle covers older portals and any path surprise.
fn call_with_response<B>(
    conn: &Connection,
    method: &str,
    body: &B,
    token: &str,
) -> Result<(u32, HashMap<String, OwnedValue>)>
where
    B: serde::Serialize + DynamicType,
{
    let predicted = predicted_request_path(conn, token)?;
    let mut primary =
        MessageIterator::for_match_rule(response_rule(Some(predicted.as_str()))?, conn, Some(16))?;
    let mut fallback = MessageIterator::for_match_rule(response_rule(None)?, conn, Some(16))?;

    let reply = conn
        .call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), method, body)
        .with_context(|| format!("calling {method} on the GlobalShortcuts portal"))?;
    let handle: OwnedObjectPath = reply
        .body()
        .deserialize()
        .with_context(|| format!("reading {method}'s request handle"))?;

    let use_primary = handle.as_str() == predicted.as_str();
    let iter: &mut MessageIterator = if use_primary {
        &mut primary
    } else {
        &mut fallback
    };

    loop {
        let msg = match iter.next() {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => return Err(e).context("waiting for the portal's response"),
            None => bail!("the portal closed the connection while waiting for a response"),
        };
        if msg.header().path().map(|p| p.as_str()) != Some(handle.as_str()) {
            continue;
        }
        let (response, results): (u32, HashMap<String, OwnedValue>) = msg
            .body()
            .deserialize()
            .context("reading the portal's response")?;
        return Ok((response, results));
    }
}

/// A live GlobalShortcuts session and its bound shortcuts. Dropping it closes
/// the connection, which ends the session and releases the desktop grabs.
pub struct Shortcuts {
    conn: Connection,
    session: OwnedObjectPath,
}

impl Shortcuts {
    /// Registers [`APP_ID`], creates a session and asks the user to confirm
    /// the bindings. `Ok(None)` means the user declined; the caller can keep
    /// running without hotkeys.
    pub fn register(cfg: &Config, hotkeys: &[Hotkey], verbose: bool) -> Result<Option<Self>> {
        let conn = Connection::session().context("connecting to the D-Bus session bus")?;
        Self::register_on(conn, cfg, hotkeys, verbose)
    }

    pub(crate) fn register_on(
        conn: Connection,
        cfg: &Config,
        hotkeys: &[Hotkey],
        verbose: bool,
    ) -> Result<Option<Self>> {
        register_app_id(&conn, verbose)?;
        let session = create_session(&conn, verbose)?;
        if bind_shortcuts(&conn, &session, cfg, hotkeys, verbose)? {
            Ok(Some(Self { conn, session }))
        } else {
            Ok(None)
        }
    }

    pub fn session_handle(&self) -> &OwnedObjectPath {
        &self.session
    }

    /// Signal stream for this connection's portal object.
    pub fn signals(&self) -> Result<MessageIterator> {
        let rule = MatchRule::builder()
            .msg_type(MessageType::Signal)
            .sender(BUS_NAME)?
            .path(OBJECT_PATH)?
            .interface(INTERFACE)?
            .build();
        Ok(MessageIterator::for_match_rule(rule, &self.conn, Some(64))?)
    }
}

fn create_session(conn: &Connection, verbose: bool) -> Result<OwnedObjectPath> {
    let token = next_token("create");
    let session_token = next_token("session");
    let mut options: HashMap<String, Value<'static>> = HashMap::new();
    options.insert("handle_token".into(), Value::from(token.clone()));
    options.insert(
        "session_handle_token".into(),
        Value::from(session_token.clone()),
    );

    let (response, results) = call_with_response(conn, "CreateSession", &(options,), &token)?;
    match response_status(response) {
        Response::Accepted => {}
        Response::Declined(code) => {
            bail!("the desktop refused to create a GlobalShortcuts session (response {code})")
        }
    }
    let session = session_handle_from(&results)?;
    if verbose {
        eprintln!("touchpad-snap: created GlobalShortcuts session {session}");
    }
    Ok(session)
}

fn bind_shortcuts(
    conn: &Connection,
    session: &OwnedObjectPath,
    cfg: &Config,
    hotkeys: &[Hotkey],
    verbose: bool,
) -> Result<bool> {
    let token = next_token("bind");
    let mut options: HashMap<String, Value<'static>> = HashMap::new();
    options.insert("handle_token".into(), Value::from(token.clone()));

    let shortcuts: Vec<(String, HashMap<String, Value<'static>>)> = hotkeys
        .iter()
        .enumerate()
        .map(|(index, hotkey)| {
            let mut props: HashMap<String, Value<'static>> = HashMap::new();
            props.insert(
                "description".into(),
                Value::from(description(cfg, index).to_string()),
            );
            props.insert("preferred_trigger".into(), Value::from(trigger(hotkey)));
            (shortcut_id(index), props)
        })
        .collect();

    let body = (session.clone(), shortcuts, "", options);
    let (response, results) = call_with_response(conn, "BindShortcuts", &body, &token)?;
    match response_status(response) {
        Response::Accepted => {
            let bound = results
                .get("shortcuts")
                .and_then(|v| v.try_clone().ok())
                .and_then(|v| Vec::<(String, HashMap<String, OwnedValue>)>::try_from(v).ok())
                .map(|v| v.len())
                .unwrap_or(hotkeys.len());
            eprintln!("touchpad-snap: the desktop bound {bound} global shortcut(s)");
            Ok(true)
        }
        Response::Declined(code) => {
            eprintln!(
                "touchpad-snap: the desktop did not accept the global shortcuts (response {code}); \
                 running without hotkeys. Approve them in the desktop's shortcut settings and restart touchpad-snap."
            );
            if verbose {
                eprintln!("touchpad-snap: GlobalShortcuts BindShortcuts returned response {code}");
            }
            Ok(false)
        }
    }
}

fn description(cfg: &Config, index: usize) -> &'static str {
    match cfg.hotkey_binding(index).and_then(|b| b.actions.first()) {
        Some(Action::Next) => "touchpad-snap: focus the next window of this app",
        Some(Action::Prev) => "touchpad-snap: focus the previous window of this app",
        Some(Action::AppNext) => "touchpad-snap: focus the next app",
        Some(Action::AppPrev) => "touchpad-snap: focus the previous app",
        _ => "touchpad-snap: switch windows of this app",
    }
}

/// Turns an `Activated`/`Deactivated` signal into a daemon event.
pub fn parse_signal(msg: &Message, session: &str, hotkeys: &[Hotkey]) -> Option<Event> {
    let header = msg.header();
    let member = header.member()?.as_str();
    if !matches!(member, "Activated" | "Deactivated") {
        return None;
    }
    let (session_handle, id, _timestamp, _options): (
        OwnedObjectPath,
        String,
        u64,
        HashMap<String, OwnedValue>,
    ) = msg.body().deserialize().ok()?;
    if session_handle.as_str() != session {
        return None;
    }
    let index = shortcut_index(&id)?;
    if member == "Activated" {
        let tracked = hotkeys.get(index).is_some_and(|h| !h.mods.is_empty());
        Some(Event::Hotkey { index, tracked })
    } else {
        Some(Event::Released)
    }
}

/// What the portal loop receives: a hotkey signal, or (with the `libinput`
/// feature) a gesture from the touchpad reader thread.
enum Incoming {
    Signal(Message),
    #[cfg(feature = "libinput")]
    Gesture(Event),
}

/// Portal-only daemon loop: no extension hotkeys, but the desktop's backend
/// still lists, activates and draws the HUD.
pub fn run_hotkeys(cfg: &Config, hotkeys: &[Hotkey], handle: &mut super::Handler) -> Result<()> {
    run_hotkeys_with(cfg, hotkeys, false, handle)
}

pub fn run_hotkeys_with(
    cfg: &Config,
    hotkeys: &[Hotkey],
    verbose: bool,
    handle: &mut super::Handler,
) -> Result<()> {
    let backend = super::connect()?;
    let conn = Connection::session().context("connecting to the D-Bus session bus")?;
    super::claim_daemon_name(&conn)?;

    let mut hotkeys = hotkeys.to_vec();

    let shortcuts = Shortcuts::register_on(conn, cfg, &hotkeys, verbose)?;
    let mut session = String::new();
    let (tx, rx) = std::sync::mpsc::channel::<Incoming>();
    if let Some(shortcuts) = &shortcuts {
        session = shortcuts.session_handle().as_str().to_string();
        let signals = shortcuts.signals()?;
        let signal_tx = tx.clone();
        std::thread::spawn(move || {
            for msg in signals.flatten() {
                if signal_tx.send(Incoming::Signal(msg)).is_err() {
                    break;
                }
            }
        });
    } else {
        eprintln!("touchpad-snap: running without global hotkeys");
    }

    // Touchpad gestures from libinput, bridged into the same loop. The
    // portal's hotkey behaviour is unchanged.
    #[cfg(feature = "libinput")]
    let gestures = {
        let (gesture_tx, gesture_rx) = std::sync::mpsc::channel::<Event>();
        let source = super::libinput::GestureSource::spawn(
            cfg.claims_for(&super::current_platform()),
            gesture_tx,
            verbose,
        );
        eprintln!("touchpad-snap: serving configured touchpad swipes through libinput");
        let bridge_tx = tx.clone();
        std::thread::spawn(move || {
            for event in gesture_rx {
                if bridge_tx.send(Incoming::Gesture(event)).is_err() {
                    break;
                }
            }
        });
        source
    };

    // Keep the sender alive even when there is no portal session, so the loop
    // keeps ticking instead of seeing a disconnected channel.
    let _keepalive = tx;

    let mut last_tick = Instant::now();
    loop {
        let wait = super::TICK.saturating_sub(last_tick.elapsed());
        let incoming = match rx.recv_timeout(wait) {
            Ok(msg) => Some(msg),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut changed = None;
        if last_tick.elapsed() >= super::TICK {
            last_tick = Instant::now();
            changed = handle(backend.as_ref(), Event::Tick);
        }
        match incoming {
            Some(Incoming::Signal(msg)) => {
                if let Some(event) = parse_signal(&msg, &session, &hotkeys) {
                    changed = changed.or(handle(backend.as_ref(), event));
                }
            }
            #[cfg(feature = "libinput")]
            Some(Incoming::Gesture(event)) => {
                changed = changed.or(handle(backend.as_ref(), event));
            }
            None => {}
        }
        if let Some(new) = changed {
            let claims = new.claims_for(&super::current_platform());
            hotkeys = new.hotkeys();
            #[cfg(feature = "libinput")]
            gestures.set_claims(claims);
            #[cfg(not(feature = "libinput"))]
            let _ = claims;
            eprintln!(
                "touchpad-snap: config reloaded; GlobalShortcuts bindings change after a restart"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(spec: &str) -> Hotkey {
        Hotkey::parse(spec).unwrap()
    }

    #[test]
    fn trigger_uses_the_shortcuts_xdg_spec() {
        assert_eq!(trigger(&parse("Super+Alt+N")), "ALT+LOGO+n");
        assert_eq!(trigger(&parse("Ctrl+Shift+F5")), "CTRL+SHIFT+F5");
        assert_eq!(trigger(&parse("Alt+PageUp")), "ALT+Page_Up");
        assert_eq!(trigger(&parse("Super+grave")), "LOGO+grave");
        assert_eq!(trigger(&parse("Ctrl+1")), "CTRL+1");
        assert_eq!(trigger(&parse("Tab")), "Tab");
    }

    #[test]
    fn trigger_covers_letters_digits_function_and_named_keys() {
        for (spec, expected) in [
            ("a", "a"),
            ("z", "z"),
            ("0", "0"),
            ("9", "9"),
            ("F1", "F1"),
            ("F24", "F24"),
            ("Left", "Left"),
            ("Right", "Right"),
            ("Up", "Up"),
            ("Down", "Down"),
            ("Tab", "Tab"),
            ("Space", "space"),
            ("Return", "Return"),
            ("Escape", "Escape"),
            ("Home", "Home"),
            ("End", "End"),
            ("PageDown", "Page_Down"),
            ("minus", "minus"),
            ("equal", "equal"),
            ("bracketleft", "bracketleft"),
            ("bracketright", "bracketright"),
            ("semicolon", "semicolon"),
            ("apostrophe", "apostrophe"),
            ("comma", "comma"),
            ("period", "period"),
            ("slash", "slash"),
            ("backslash", "backslash"),
        ] {
            assert_eq!(trigger(&parse(spec)), expected, "{spec}");
        }
    }

    #[test]
    fn shortcut_ids_map_back_to_indices() {
        for index in [0, 1, 9, 42] {
            assert_eq!(shortcut_index(&shortcut_id(index)), Some(index));
        }
        assert_eq!(shortcut_index("hotkey:"), None);
        assert_eq!(shortcut_index("hotkey:x"), None);
        assert_eq!(shortcut_index("other:1"), None);
    }

    #[test]
    fn responses_zero_means_accepted() {
        assert_eq!(response_status(0), Response::Accepted);
        assert_eq!(response_status(1), Response::Declined(1));
        assert_eq!(response_status(2), Response::Declined(2));
    }

    #[test]
    fn missing_session_handle_is_an_error() {
        let results: HashMap<String, OwnedValue> = HashMap::new();
        assert!(session_handle_from(&results).is_err());
    }

    #[test]
    fn activated_and_deactivated_signals_map_to_events() {
        let session = OwnedObjectPath::try_from(
            "/org/freedesktop/portal/desktop/session/1_2/touchpad_snap".to_string(),
        )
        .unwrap();
        let hotkeys = [parse("Super+Alt+N"), parse("F5")];

        let activated = Message::signal(OBJECT_PATH, INTERFACE, "Activated")
            .unwrap()
            .build(&(
                session.clone(),
                "hotkey:0".to_string(),
                12u64,
                HashMap::<String, OwnedValue>::new(),
            ))
            .unwrap();
        assert_eq!(
            parse_signal(&activated, session.as_str(), &hotkeys),
            Some(Event::Hotkey {
                index: 0,
                tracked: true
            })
        );

        let untracked = Message::signal(OBJECT_PATH, INTERFACE, "Activated")
            .unwrap()
            .build(&(
                session.clone(),
                "hotkey:1".to_string(),
                12u64,
                HashMap::<String, OwnedValue>::new(),
            ))
            .unwrap();
        assert_eq!(
            parse_signal(&untracked, session.as_str(), &hotkeys),
            Some(Event::Hotkey {
                index: 1,
                tracked: false
            })
        );

        let deactivated = Message::signal(OBJECT_PATH, INTERFACE, "Deactivated")
            .unwrap()
            .build(&(
                session.clone(),
                "hotkey:0".to_string(),
                13u64,
                HashMap::<String, OwnedValue>::new(),
            ))
            .unwrap();
        assert_eq!(
            parse_signal(&deactivated, session.as_str(), &hotkeys),
            Some(Event::Released)
        );

        // Another session's signal is ignored.
        let other = OwnedObjectPath::try_from(
            "/org/freedesktop/portal/desktop/session/9_9/other".to_string(),
        )
        .unwrap();
        assert_eq!(parse_signal(&deactivated, other.as_str(), &hotkeys), None);
    }
}
