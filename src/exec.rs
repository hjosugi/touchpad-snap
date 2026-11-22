//! `exec` actions: run a program without blocking the daemon or tying the
//! child to its lifetime.

use std::process::{Command, ExitStatus, Stdio};

use crate::config::{ExecCommand, Trigger};
use crate::model::Snapshot;

/// What the child learns about the window the trigger started on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionContext {
    pub window_id: Option<u64>,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub trigger: String,
}

impl ActionContext {
    /// The focused window at trigger start (not at action time) and the
    /// trigger label.
    pub fn at(trigger: &Trigger, snapshot: Option<&Snapshot>) -> Self {
        let focused = snapshot.and_then(Snapshot::focused);
        let text = |value: Option<&String>| value.filter(|s| !s.is_empty()).cloned();
        Self {
            window_id: focused.map(|window| window.id),
            app_id: text(focused.map(|window| &window.app_id)),
            app_name: text(focused.map(|window| &window.app_name)),
            title: text(focused.map(|window| &window.title)),
            trigger: trigger.label(),
        }
    }

    fn apply(&self, command: &mut Command) {
        command.env("TOUCHPAD_SNAP_TRIGGER", &self.trigger);
        if let Some(id) = self.window_id {
            command.env("TOUCHPAD_SNAP_WINDOW_ID", id.to_string());
        }
        if let Some(value) = &self.app_id {
            command.env("TOUCHPAD_SNAP_APP_ID", value);
        }
        if let Some(value) = &self.app_name {
            command.env("TOUCHPAD_SNAP_APP_NAME", value);
        }
        if let Some(value) = &self.title {
            command.env("TOUCHPAD_SNAP_WINDOW_TITLE", value);
        }
    }
}

/// Starts the command and returns at once. A thread reaps it (and, in
/// verbose mode, reports the exit status and stderr) so the daemon never
/// waits for it.
pub fn spawn(command: &ExecCommand, shell: bool, context: &ActionContext, verbose: bool) {
    let label = command.display();
    let mut process = match build(command, shell) {
        Ok(process) => process,
        Err(message) => {
            eprintln!("touchpad-snap: exec `{label}`: {message}");
            return;
        }
    };
    context.apply(&mut process);
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if verbose {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    detach(&mut process);

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("touchpad-snap: exec `{label}`: {error}");
            return;
        }
    };
    if verbose {
        std::thread::spawn(move || match child.wait_with_output() {
            Ok(output) => {
                eprintln!(
                    "touchpad-snap: exec `{label}`: {}",
                    describe_status(output.status)
                );
                let stderr = String::from_utf8_lossy(&output.stderr);
                for line in stderr.lines() {
                    eprintln!("touchpad-snap: exec `{label}`: {line}");
                }
            }
            Err(error) => eprintln!("touchpad-snap: exec `{label}`: {error}"),
        });
    } else {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn build(command: &ExecCommand, shell: bool) -> Result<Command, String> {
    match command {
        ExecCommand::Shell(script) => {
            if !shell {
                return Err(
                    "a command string needs \"shell\": true; use an argument list to run a \
                     program directly"
                        .into(),
                );
            }
            #[cfg(windows)]
            {
                let mut process = Command::new("cmd");
                process.args(["/C", script.as_str()]);
                Ok(process)
            }
            #[cfg(not(windows))]
            {
                let mut process = Command::new("sh");
                process.args(["-c", script.as_str()]);
                Ok(process)
            }
        }
        ExecCommand::Args(args) => {
            if shell {
                return Err("a command list runs directly; remove \"shell\": true".into());
            }
            let (program, rest) = args
                .split_first()
                .ok_or_else(|| "\"command\" must not be an empty list".to_string())?;
            let mut process = Command::new(program);
            process.args(rest);
            Ok(process)
        }
    }
}

fn describe_status(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exited with status {code}"),
        None => status.to_string(),
    }
}

/// Gives the child its own process group on Linux, or no console window on
/// Windows.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, DETACHED_PROCESS};
    command.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config::ExecCommand;
    use crate::model::Window;

    fn wait_for_file(path: &std::path::Path) -> String {
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(path) {
                return text;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("{} was not written", path.display());
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("touchpad-snap-exec-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("marker")
    }

    #[test]
    fn runs_an_argument_list_detached() {
        let path = temp_path("direct");
        let command = ExecCommand::Args(vec![
            "sh".into(),
            "-c".into(),
            format!("printf direct > {}", path.display()),
        ]);
        let context = ActionContext::default();
        spawn(&command, false, &context, false);
        assert_eq!(wait_for_file(&path), "direct");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn shell_command_receives_the_trigger_window_and_trigger() {
        let path = temp_path("shell");
        let script = format!(
            "printf '%s|%s|%s|%s|%s' \"$TOUCHPAD_SNAP_WINDOW_ID\" \
             \"$TOUCHPAD_SNAP_APP_ID\" \"$TOUCHPAD_SNAP_APP_NAME\" \
             \"$TOUCHPAD_SNAP_WINDOW_TITLE\" \"$TOUCHPAD_SNAP_TRIGGER\" > {}",
            path.display()
        );
        let snapshot = Snapshot {
            active_workspace: 0,
            windows: vec![Window {
                id: 42,
                app_id: "org.example.App.desktop".into(),
                app_name: "Example".into(),
                title: "A title".into(),
                focused: true,
                ..Default::default()
            }],
        };
        let trigger = Trigger::Swipe {
            fingers: 3,
            modifiers: vec![crate::hotkey::Modifier::Alt],
            direction: crate::gesture::SwipeDirection::Up,
        };
        let context = ActionContext::at(&trigger, Some(&snapshot));
        spawn(&ExecCommand::Shell(script), true, &context, false);
        assert_eq!(
            wait_for_file(&path),
            "42|org.example.App.desktop|Example|A title|swipe-3-alt-up"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_unknown_window_sets_only_the_trigger() {
        let context = ActionContext::at(
            &Trigger::Hotkey {
                keys: "Ctrl+Alt+E".into(),
            },
            None,
        );
        assert_eq!(context.window_id, None);
        assert_eq!(context.app_id, None);
        assert_eq!(context.app_name, None);
        assert_eq!(context.title, None);
        assert_eq!(context.trigger, "hotkey-Ctrl+Alt+E");
    }

    #[test]
    fn mismatched_shell_flags_are_reported_before_spawning() {
        let context = ActionContext::default();
        // These print a message and return; the daemon stays up either way.
        let path = temp_path("mismatch");
        spawn(
            &ExecCommand::Args(vec![
                "sh".into(),
                "-c".into(),
                format!("printf x > {}", path.display()),
            ]),
            true,
            &context,
            false,
        );
        spawn(
            &ExecCommand::Shell(format!("printf x > {}", path.display())),
            false,
            &context,
            false,
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
