//! Command-line interface, shared by `window-jump` and `window-jumpw`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::config::{self, Config};
use crate::cycle::{self, Order, Pick, Session, Step, WorkspaceScope};
use crate::{autostart, daemon, platform, session};

#[derive(Parser, Debug)]
#[command(
    name = "window-jump",
    version,
    about = "Switch between windows of the same app with hotkeys and touchpad swipes (GNOME, Sway, Hyprland, niri, Windows)",
    after_help = "Bind `window-jump next` / `window-jump prev` to any shortcut, or run `window-jump daemon` \
                  for the hotkeys and swipes in the config file."
)]
struct Cli {
    /// Config file [default: ~/.config/window-jump/config.toml, %APPDATA%\window-jump\config.toml]
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Focus the next window of the focused app
    Next(JumpArgs),
    /// Focus the previous window of the focused app
    Prev(JumpArgs),
    /// List windows, grouped by app
    List {
        /// Print the raw snapshot as JSON
        #[arg(long)]
        json: bool,
    },
    /// Focus one window by id (as printed by `list`); for scripts and debugging
    Activate {
        /// Window id, decimal or 0x-prefixed hex
        #[arg(value_parser = parse_id)]
        id: u64,
    },
    /// Run in the background, serving the configured hotkeys and swipes
    Daemon {
        /// Log every hotkey, swipe and jump
        #[arg(short, long)]
        verbose: bool,
    },
    /// Manage the config file
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Start the daemon at login
    Autostart {
        #[command(subcommand)]
        command: AutostartCommand,
    },
    /// Check that everything is wired up
    Doctor,
}

#[derive(clap::Args, Debug)]
struct JumpArgs {
    /// Override `order` from the config
    #[arg(long, value_parser = parse_order)]
    order: Option<Order>,
    /// Override `workspace` from the config
    #[arg(long, value_parser = parse_workspace)]
    workspace: Option<WorkspaceScope>,
}

#[derive(Subcommand, Debug)]
enum ConfigCommand {
    /// Print the config file path
    Path,
    /// Write a commented default config file
    Init {
        /// Overwrite an existing file
        #[arg(long)]
        force: bool,
    },
    /// Validate the config file
    Check,
}

#[derive(Subcommand, Debug)]
enum AutostartCommand {
    Enable,
    Disable,
}

fn parse_id(s: &str) -> Result<u64, String> {
    let parsed = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => s.parse(),
    };
    parsed.map_err(|_| format!("`{s}` is not a window id"))
}

fn parse_order(s: &str) -> Result<Order, String> {
    match s {
        "stable" => Ok(Order::Stable),
        "position" => Ok(Order::Position),
        "mru" => Ok(Order::Mru),
        _ => Err("expected stable, position or mru".into()),
    }
}

fn parse_workspace(s: &str) -> Result<WorkspaceScope, String> {
    match s {
        "all" => Ok(WorkspaceScope::All),
        "current" => Ok(WorkspaceScope::Current),
        _ => Err("expected all or current".into()),
    }
}

/// Runs the CLI with the given arguments (including argv[0]).
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    match dispatch(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("window-jump: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> Result<()> {
    let config_path = cli.config.clone().unwrap_or_else(config::default_path);
    match cli.command {
        Command::Next(args) => jump(&config_path, Step::Next, args),
        Command::Prev(args) => jump(&config_path, Step::Prev, args),
        Command::List { json } => list(json),
        Command::Activate { id } => platform::connect()?.activate(id),
        Command::Daemon { verbose } => {
            let cfg = config::load(&config_path)?;
            daemon::run(cfg, config_path, verbose)
        }
        Command::Config { command } => match command {
            ConfigCommand::Path => {
                println!("{}", config_path.display());
                Ok(())
            }
            ConfigCommand::Init { force } => {
                if config_path.exists() && !force {
                    bail!(
                        "{} already exists (use --force to overwrite)",
                        config_path.display()
                    );
                }
                if let Some(dir) = config_path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&config_path, config::TEMPLATE)
                    .with_context(|| format!("writing {}", config_path.display()))?;
                println!("wrote {}", config_path.display());
                Ok(())
            }
            ConfigCommand::Check => {
                let exists = config_path.exists();
                let cfg = config::load(&config_path)?;
                cfg.validate()?;
                if exists {
                    println!("{}: ok", config_path.display());
                } else {
                    println!(
                        "{} does not exist; using the defaults",
                        config_path.display()
                    );
                }
                Ok(())
            }
        },
        Command::Autostart { command } => {
            let msg = match command {
                AutostartCommand::Enable => autostart::enable()?,
                AutostartCommand::Disable => autostart::disable()?,
            };
            println!("{msg}");
            Ok(())
        }
        Command::Doctor => doctor(&config_path),
    }
}

fn jump(config_path: &std::path::Path, step: Step, args: JumpArgs) -> Result<()> {
    let mut cfg = config::load(config_path)?;
    if let Some(order) = args.order {
        cfg.order = order;
    }
    if let Some(workspace) = args.workspace {
        cfg.workspace = workspace;
    }
    let backend = platform::connect()?;
    // A shortcut tool runs us once per press; whether its keys are still held
    // is unknown, so MRU sessions go by time.
    match daemon::jump(
        backend.as_ref(),
        &cfg,
        step,
        Session::ByTime,
        &session::default_path(),
    )? {
        Pick::Jump(_) | Pick::Alone => Ok(()),
        Pick::NoFocus => bail!("no window is focused"),
    }
}

fn list(json: bool) -> Result<()> {
    let backend = platform::connect()?;
    let snap = backend.snapshot()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&snap)?);
        return Ok(());
    }
    let mut groups: Vec<(String, Vec<&crate::model::Window>)> = Vec::new();
    let mut by_mru: Vec<_> = snap.windows.iter().collect();
    by_mru.sort_by_key(|w| w.mru);
    for w in by_mru {
        let key = w.app_key();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(w),
            None => groups.push((key, vec![w])),
        }
    }
    for (key, windows) in groups {
        let name = &windows[0].app_name;
        println!("{name}  [{key}]");
        for w in windows {
            let mut flags = String::new();
            if w.focused {
                flags.push_str(" focused");
            }
            if w.minimized {
                flags.push_str(" minimized");
            }
            let ws = if w.workspace < 0 {
                "all".to_string()
            } else {
                w.workspace.to_string()
            };
            println!(
                "  {:>12}  ws {ws:<3}{flags:<19} {}",
                format!("{:#x}", w.id),
                w.title
            );
        }
    }
    Ok(())
}

fn doctor(config_path: &std::path::Path) -> Result<()> {
    let mut problems = 0;

    print!("config   {}: ", config_path.display());
    let cfg = match config::load(config_path).and_then(|c| c.validate().map(|_| c)) {
        Ok(c) => {
            println!(
                "{} ({} hotkey(s), {} swipe(s))",
                if config_path.exists() {
                    "ok"
                } else {
                    "missing, using defaults"
                },
                c.hotkeys.len(),
                c.swipes.len()
            );
            c
        }
        Err(e) => {
            problems += 1;
            println!("ERROR {e:#}");
            Config::default()
        }
    };

    match platform::connect() {
        Err(e) => {
            problems += 1;
            println!("backend  ERROR {e:#}");
        }
        Ok(backend) => {
            println!("backend  {}", backend.describe());
            match backend.snapshot() {
                Err(e) => {
                    problems += 1;
                    println!("windows  ERROR {e:#}");
                }
                Ok(snap) => {
                    println!("windows  {} listed", snap.windows.len());
                    match snap.focused() {
                        Some(f) => {
                            let n = cycle::candidates(&snap, f, &cfg.cycle_options()).len();
                            println!(
                                "focused  \"{}\" ({}): {} window(s) of this app to cycle through",
                                f.title,
                                f.app_key(),
                                n
                            );
                        }
                        None => {
                            println!("focused  nothing (normal when run from outside a window)")
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    println!("portal   {}", platform::portal::doctor());
    #[cfg(target_os = "linux")]
    println!("gestures {}", platform::libinput::doctor());

    if problems > 0 {
        bail!("{problems} problem(s) found");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_jump_overrides() {
        let cli = Cli::try_parse_from(["window-jump", "next", "--order", "mru"]).unwrap();
        match cli.command {
            Command::Next(a) => assert_eq!(a.order, Some(Order::Mru)),
            other => panic!("{other:?}"),
        }
        assert!(Cli::try_parse_from(["window-jump", "next", "--order", "nope"]).is_err());
    }

    #[test]
    fn window_ids_in_decimal_or_hex() {
        assert_eq!(parse_id("42"), Ok(42));
        assert_eq!(parse_id("0x2a"), Ok(42));
        assert!(parse_id("zz").is_err());
    }
}
