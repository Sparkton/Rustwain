use std::process::ExitCode;

use clap::Parser;
use xtask::cli::Cli;

fn main() -> ExitCode {
    match xtask::commands::run(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("xtask: {err}");
            ExitCode::from(TOOL_FAILURE)
        }
    }
}

/// Exit status when xtask itself fails (I/O, a subprocess), as opposed to a check that
/// reported errors, which exits with 1.
const TOOL_FAILURE: u8 = 2;
