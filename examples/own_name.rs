//! Test helper: owns a name on the session bus until it is killed.
//!
//!     cargo run --example own_name -- org.hjosugi.WindowJump
//!
//! test/e2e/gnome.sh uses it to stand in for the GNOME Shell extension of
//! window-jump, this program's old name, which `doctor` has to notice.

#[cfg(target_os = "linux")]
fn main() -> zbus::Result<()> {
    let name = std::env::args().nth(1).expect("usage: own_name <bus name>");
    let conn = zbus::blocking::Connection::session()?;
    conn.request_name(name.as_str())?;
    println!("owning {name}");
    loop {
        std::thread::park();
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {}
