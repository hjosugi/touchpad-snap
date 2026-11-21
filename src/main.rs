use std::process::ExitCode;

fn main() -> ExitCode {
    touchpad_snap::cli::run(std::env::args_os())
}
