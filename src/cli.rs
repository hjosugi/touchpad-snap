//! Command-line interface, shared by `touchpad-snap` and `touchpad-snapw`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use crate::config::{self, Config, Source};
use crate::cycle::{self, Order, Pick, Session, Step, WorkspaceScope};
use crate::{autostart, daemon, legacy, platform, session, systemd};

#[derive(Parser, Debug)]
#[command(
    name = "touchpad-snap",
    version,
    about = "Switch between windows of the same app with hotkeys and touchpad swipes (GNOME, KDE Plasma, Sway, Hyprland, niri, Windows)",
    after_help = "Bind `touchpad-snap next` / `touchpad-snap prev` to any shortcut, or run `touchpad-snap daemon` \
                  for the hotkeys and swipes in the config file."
)]
struct Cli {
    /// Config file [default: ~/.config/touchpad-snap/config.json, %APPDATA%\touchpad-snap\config.json]
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
    /// Write the default config.json
    Init {
        /// Overwrite an existing file
        #[arg(long)]
        force: bool,
    },
    /// Convert the old config.toml next to config.json
    Import {
        /// Overwrite an existing config.json
        #[arg(long)]
        force: bool,
    },
    /// Print the JSON Schema for config.json
    Schema,
    /// Validate the config file and list skipped bindings
    Check,
}

#[derive(Subcommand, Debug)]
enum AutostartCommand {
    Enable {
        /// Use a systemd user service instead of the XDG autostart entry (Linux)
        #[arg(long)]
        systemd: bool,
    },
    Disable {
        /// Remove the systemd user service instead of the XDG autostart entry (Linux)
        #[arg(long)]
        systemd: bool,
    },
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
            eprintln!("touchpad-snap: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> Result<()> {
    let config_path = cli.config.clone().unwrap_or_else(config::default_path);
    if cli.config.is_none() && !config_path.exists() {
        // First run after the rename: carry window-jump's settings over as the
        // old TOML next to config.json, so the usual legacy migration applies.
        let old = legacy::config_path();
        let toml_path = config::legacy_path(&config_path);
        match legacy::import_config(&old, &toml_path) {
            Ok(true) => eprintln!(
                "touchpad-snap: copied the settings of {} from {} to {}",
                legacy::NAME,
                old.display(),
                toml_path.display()
            ),
            Ok(false) => {}
            Err(e) => eprintln!(
                "touchpad-snap: could not carry over the settings of {}: {e:#}",
                legacy::NAME
            ),
        }
    }
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
                config::write(&config_path, &Config::default())?;
                println!("wrote {}", config_path.display());
                Ok(())
            }
            ConfigCommand::Import { force } => {
                let from = config::legacy_path(&config_path);
                if !from.exists() {
                    bail!("{} does not exist", from.display());
                }
                if config_path.exists() && !force {
                    bail!(
                        "{} already exists (use --force to overwrite)",
                        config_path.display()
                    );
                }
                let cfg = config::load_legacy(&from)?;
                if cfg.report().has_errors() {
                    for problem in cfg.report().errors() {
                        eprintln!("{}", problem.describe());
                    }
                    bail!(
                        "{} has bindings that cannot be imported; fix them first",
                        from.display()
                    );
                }
                for problem in cfg.report().warnings() {
                    eprintln!("{}", problem.describe());
                }
                config::write(&config_path, &cfg)?;
                println!(
                    "wrote {} from {} ({} binding(s))",
                    config_path.display(),
                    from.display(),
                    cfg.bindings.len()
                );
                Ok(())
            }
            ConfigCommand::Schema => {
                print!("{}", config::schema_json());
                Ok(())
            }
            ConfigCommand::Check => config_check(&config_path),
        },
        Command::Autostart { command } => {
            let msg = match command {
                AutostartCommand::Enable { systemd: true } => systemd::enable()?,
                AutostartCommand::Enable { systemd: false } => autostart::enable()?,
                AutostartCommand::Disable { systemd: true } => systemd::disable()?,
                AutostartCommand::Disable { systemd: false } => autostart::disable()?,
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
        cfg.cycle.order = order;
    }
    if let Some(workspace) = args.workspace {
        cfg.cycle.workspace = workspace;
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

fn config_check(config_path: &std::path::Path) -> Result<()> {
    let cfg = Config::load_report(config_path)?;
    let report = cfg.report();
    match &report.source {
        Source::Json(path) => println!(
            "{}: {} ({} binding(s), {} hotkey(s), {} swipe claim(s))",
            path.display(),
            if report.has_errors() {
                "problems"
            } else {
                "ok"
            },
            cfg.bindings.len(),
            cfg.hotkeys().len(),
            cfg.claims().len()
        ),
        Source::Defaults => println!(
            "{} does not exist; using the defaults",
            config_path.display()
        ),
        Source::LegacyToml(path) => {
            println!(
                "{} is loaded in memory from the legacy {}",
                config_path.display(),
                path.display()
            );
            println!(
                "note: run `touchpad-snap config import` to write {}",
                config_path.display()
            );
        }
    }
    for problem in &report.problems {
        println!("{}", problem.describe());
    }
    for problem in cfg.missing_exec_commands() {
        println!("{}", problem.describe());
    }
    if cfg.has_shell_exec() {
        println!("note: commands run with \"shell\": true are not checked against PATH");
    }
    if report.has_errors() {
        bail!("{} binding(s) could not be loaded", report.errors().count());
    }
    Ok(())
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
    let cfg = match Config::load_report(config_path) {
        Ok(c) => {
            let label = match &c.report().source {
                Source::Json(_) => "ok",
                Source::Defaults => "missing, using defaults",
                Source::LegacyToml(_) => "legacy config.toml, loaded in memory",
            };
            println!(
                "{} ({} binding(s), {} hotkey(s), {} swipe claim(s))",
                label,
                c.bindings.len(),
                c.hotkeys().len(),
                c.claims().len()
            );
            c
        }
        Err(e) => {
            problems += 1;
            println!("ERROR {e:#}");
            Config::default()
        }
    };
    if let Source::LegacyToml(path) = &cfg.report().source {
        println!(
            "migration run `touchpad-snap config import` to convert {} to config.json",
            path.display()
        );
    }
    for problem in &cfg.report().problems {
        println!("config   {}", problem.describe());
    }
    problems += cfg.report().errors().count();

    match platform::connect() {
        Err(e) => {
            problems += 1;
            println!("backend  ERROR {e:#}");
        }
        Ok(backend) => {
            println!("backend  {}", backend.describe());
            for leftover in backend.leftovers() {
                problems += 1;
                println!("backend  ERROR {leftover}");
            }
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
    println!("kde      {}", platform::kde::doctor());
    #[cfg(target_os = "linux")]
    println!("portal   {}", platform::portal::doctor());
    #[cfg(target_os = "linux")]
    println!("gestures {}", platform::libinput::doctor());

    if let Some(leftover) = autostart::leftover() {
        problems += 1;
        println!("startup  ERROR {leftover}");
    }
    #[cfg(target_os = "linux")]
    if let Some(leftover) = systemd::leftover() {
        problems += 1;
        println!("startup  ERROR {leftover}");
    }

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
        let cli = Cli::try_parse_from(["touchpad-snap", "next", "--order", "mru"]).unwrap();
        match cli.command {
            Command::Next(a) => assert_eq!(a.order, Some(Order::Mru)),
            other => panic!("{other:?}"),
        }
        assert!(Cli::try_parse_from(["touchpad-snap", "next", "--order", "nope"]).is_err());
    }

    #[test]
    fn parses_systemd_autostart_flag() {
        let cli =
            Cli::try_parse_from(["touchpad-snap", "autostart", "enable", "--systemd"]).unwrap();
        match cli.command {
            Command::Autostart {
                command: AutostartCommand::Enable { systemd: true },
            } => {}
            other => panic!("{other:?}"),
        }

        let cli = Cli::try_parse_from(["touchpad-snap", "autostart", "disable"]).unwrap();
        match cli.command {
            Command::Autostart {
                command: AutostartCommand::Disable { systemd: false },
            } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn window_ids_in_decimal_or_hex() {
        assert_eq!(parse_id("42"), Ok(42));
        assert_eq!(parse_id("0x2a"), Ok(42));
        assert!(parse_id("zz").is_err());
    }
}
