//! Test helper: types a key combination into a GNOME session through
//! Mutter's RemoteDesktop D-Bus API, the way a remote-desktop client would.
//!
//!     cargo run --example inject_keys -- Super_L Alt_L n
//!
//! The keys are pressed in order and released in reverse. Mutter ends a
//! remote-desktop session when its caller disconnects, so this has to be one
//! process holding the connection (gdbus cannot do it call by call). Used by
//! test/e2e/gnome.sh against the throw-away headless shell.

#[cfg(target_os = "linux")]
fn main() -> zbus::Result<()> {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedObjectPath;

    let keysyms: Vec<u32> = std::env::args()
        .skip(1)
        .map(|name| keysym(&name).unwrap_or_else(|| panic!("unknown key {name}")))
        .collect();

    let conn = Connection::session()?;
    let rd = Proxy::new(
        &conn,
        "org.gnome.Mutter.RemoteDesktop",
        "/org/gnome/Mutter/RemoteDesktop",
        "org.gnome.Mutter.RemoteDesktop",
    )?;
    let path: OwnedObjectPath = rd.call("CreateSession", &())?;
    let session = Proxy::new(
        &conn,
        "org.gnome.Mutter.RemoteDesktop",
        path,
        "org.gnome.Mutter.RemoteDesktop.Session",
    )?;
    session.call_method("Start", &())?;
    // Give the virtual keyboard a moment to appear in the seat.
    std::thread::sleep(std::time::Duration::from_millis(300));
    for &k in &keysyms {
        session.call_method("NotifyKeyboardKeysym", &(k, true))?;
    }
    for &k in keysyms.iter().rev() {
        session.call_method("NotifyKeyboardKeysym", &(k, false))?;
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    session.call_method("Stop", &())?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn keysym(name: &str) -> Option<u32> {
    Some(match name {
        "Super_L" => 0xffeb,
        "Alt_L" => 0xffe9,
        "Control_L" => 0xffe3,
        "Shift_L" => 0xffe1,
        "Page_Down" => 0xff56,
        "Page_Up" => 0xff55,
        "End" => 0xff57,
        "F9" => 0xffc6,
        s if s.len() == 1 && s.is_ascii() => s.as_bytes()[0] as u32,
        _ => return None,
    })
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("inject_keys needs GNOME on Linux");
}
