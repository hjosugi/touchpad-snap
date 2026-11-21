use std::process::ExitCode;

fn main() -> ExitCode {
    window_jump::cli::run(std::env::args_os())
}
