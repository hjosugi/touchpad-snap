//! A systemd user service, an alternative to the XDG autostart entry.
//!
//! The unit lives in `~/.config/systemd/user/touchpad-snap.service`, starts
//! with `graphical-session.target` and is restarted by systemd if the daemon
//! crashes; its output goes to the journal.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::legacy;

/// The unit name `systemctl --user` takes, and the file it is stored in.
pub const UNIT: &str = "touchpad-snap.service";

/// Renders the unit file. `command` is the executable, already quoted for an
/// `ExecStart=` line; `daemon` is appended here.
pub fn unit_file(command: &str) -> String {
    format!(
        "[Unit]\n\
         Description=touchpad-snap daemon\n\
         PartOf=graphical-session.target\n\
         After=graphical-session.target\n\
         \n\
         [Service]\n\
         ExecStart={command} daemon\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n"
    )
}

/// Quotes one executable path for an `ExecStart=` line. systemd expands `%`
/// specifiers and `$` variables even inside quotes, so both are doubled;
/// backslashes and quotes are escaped.
pub fn quote_path(path: &str) -> String {
    let escaped = path
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

/// Runs `systemctl`. Tests substitute a recorder so they never touch the
/// real user manager.
pub trait Systemctl {
    fn run(&self, args: &[&str]) -> Result<()>;
}

/// The real thing: `systemctl --user <args>`.
pub struct SystemSystemctl;

impl Systemctl for SystemSystemctl {
    fn run(&self, args: &[&str]) -> Result<()> {
        let output = std::process::Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .with_context(|| format!("running `systemctl --user {}`", args.join(" ")))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            anyhow::bail!(
                "`systemctl --user {}` failed ({})",
                args.join(" "),
                output.status
            )
        } else {
            anyhow::bail!("`systemctl --user {}` failed: {stderr}", args.join(" "))
        }
    }
}

/// Writes and enables the unit in `dir`, where `dir` is the systemd user unit
/// directory. A failing `systemctl` leaves the unit in place for mending.
pub fn enable_at(dir: &Path, command: &str, systemctl: &dyn Systemctl) -> Result<String> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(UNIT);
    std::fs::write(&path, unit_file(command))
        .with_context(|| format!("writing {}", path.display()))?;
    remove(&legacy_unit(dir))?;

    let reload = ["daemon-reload"];
    systemctl
        .run(&reload)
        .map_err(|e| failed(&path, &reload, e))?;
    let enable = ["enable", "--now", UNIT];
    systemctl
        .run(&enable)
        .map_err(|e| failed(&path, &enable, e))?;

    Ok(format!(
        "touchpad-snap will start with the graphical session ({}); see `systemctl --user status {UNIT}`",
        path.display()
    ))
}

/// Removes the unit from `dir`. A unit that was never enabled (or a machine
/// without systemd) makes `systemctl` fail; the file still has to go, so those
/// failures are ignored.
pub fn disable_at(dir: &Path, systemctl: &dyn Systemctl) -> Result<String> {
    let _ = systemctl.run(&["disable", "--now", UNIT]);
    let path = dir.join(UNIT);
    remove(&path)?;
    remove(&legacy_unit(dir))?;
    let _ = systemctl.run(&["daemon-reload"]);
    Ok(format!("removed {}", path.display()))
}

fn failed(path: &Path, args: &[&str], error: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!(
        "wrote {} but `systemctl --user {}` failed: {error:#}\n\
         the unit was left in place; fix it and run the command again",
        path.display(),
        args.join(" ")
    )
}

fn legacy_unit(dir: &Path) -> PathBuf {
    dir.join(format!("{}.service", legacy::NAME))
}

/// Removes a file; one that is not there is fine.
fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(not(windows))]
fn command() -> Result<String> {
    let exe = crate::autostart::executable_path()?;
    if exe == "touchpad-snap" {
        Ok(exe)
    } else {
        Ok(quote_path(&exe))
    }
}

#[cfg(target_os = "linux")]
fn unit_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("systemd")
        .join("user")
}

#[cfg(target_os = "linux")]
pub fn enable() -> Result<String> {
    enable_at(&unit_dir(), &command()?, &SystemSystemctl)
}

#[cfg(not(target_os = "linux"))]
pub fn enable() -> Result<String> {
    anyhow::bail!("systemd user services are only available on Linux")
}

#[cfg(target_os = "linux")]
pub fn disable() -> Result<String> {
    disable_at(&unit_dir(), &SystemSystemctl)
}

#[cfg(not(target_os = "linux"))]
pub fn disable() -> Result<String> {
    anyhow::bail!("systemd user services are only available on Linux")
}

/// A unit left by the old name, which would start a second daemon. This only
/// looks at the file, so it does not need systemd to be running.
#[cfg(target_os = "linux")]
pub fn leftover() -> Option<String> {
    let path = legacy_unit(&unit_dir());
    path.exists().then(|| {
        format!(
            "{}'s systemd user unit still exists ({}); remove it with: touchpad-snap autostart disable --systemd",
            legacy::NAME,
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "touchpad-snap-systemd-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<Vec<String>>>,
        fail: RefCell<Option<&'static str>>,
    }

    impl Systemctl for Fake {
        fn run(&self, args: &[&str]) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|arg| arg.to_string()).collect());
            if self.fail.borrow().is_some_and(|cmd| cmd == args.join(" ")) {
                anyhow::bail!("fake failure");
            }
            Ok(())
        }
    }

    #[test]
    fn unit_file_has_the_expected_sections() {
        assert_eq!(
            unit_file("touchpad-snap"),
            "[Unit]\n\
             Description=touchpad-snap daemon\n\
             PartOf=graphical-session.target\n\
             After=graphical-session.target\n\
             \n\
             [Service]\n\
             ExecStart=touchpad-snap daemon\n\
             Restart=on-failure\n\
             RestartSec=2\n\
             \n\
             [Install]\n\
             WantedBy=graphical-session.target\n"
        );
    }

    #[test]
    fn shipped_template_matches_the_generated_unit() {
        let template =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("contrib/systemd/touchpad-snap.service");
        // Git on Windows checks the template out with CRLF; the generated
        // unit always uses LF.
        let template = std::fs::read_to_string(template)
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(template, unit_file("touchpad-snap"));
    }

    #[test]
    fn paths_with_spaces_and_specifiers_are_escaped() {
        let path = quote_path("/home/a b/100%/touchpad-$snap");
        assert_eq!(path, "\"/home/a b/100%%/touchpad-$$snap\"");
        assert!(unit_file(&path).contains(&format!("ExecStart={path} daemon\n")));
        assert_eq!(
            quote_path(r#"/a "quoted" \path"#),
            "\"/a \\\"quoted\\\" \\\\path\""
        );
    }

    #[test]
    fn enable_writes_the_unit_enables_it_and_clears_the_old_name() {
        let dir = temp_dir("enable");
        std::fs::write(dir.join("window-jump.service"), "old").unwrap();
        let fake = Fake::default();

        let message = enable_at(&dir, "touchpad-snap", &fake).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join(UNIT)).unwrap(),
            unit_file("touchpad-snap")
        );
        assert!(!dir.join("window-jump.service").exists());
        assert!(message.contains(&dir.join(UNIT).display().to_string()));
        let calls = fake.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ["daemon-reload"]);
        assert_eq!(calls[1], ["enable", "--now", UNIT]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn enable_names_the_unit_and_keeps_it_when_systemctl_fails() {
        let dir = temp_dir("enable-fail");
        let fake = Fake::default();
        *fake.fail.borrow_mut() = Some("enable --now touchpad-snap.service");

        let error = enable_at(&dir, "touchpad-snap", &fake)
            .unwrap_err()
            .to_string();

        assert!(error.contains(&dir.join(UNIT).display().to_string()));
        assert!(error.contains("systemctl --user enable --now touchpad-snap.service"));
        assert!(dir.join(UNIT).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn disable_removes_the_unit_and_tolerates_systemctl_failures() {
        let dir = temp_dir("disable");
        std::fs::write(dir.join(UNIT), "unit").unwrap();
        std::fs::write(dir.join("window-jump.service"), "old").unwrap();
        let fake = Fake::default();

        disable_at(&dir, &fake).unwrap();

        assert!(!dir.join(UNIT).exists());
        assert!(!dir.join("window-jump.service").exists());
        let calls = fake.calls.borrow();
        assert_eq!(calls[0], ["disable", "--now", UNIT]);
        assert_eq!(calls[1], ["daemon-reload"]);

        let failing = Fake::default();
        *failing.fail.borrow_mut() = Some("disable --now touchpad-snap.service");
        disable_at(&dir, &failing).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
