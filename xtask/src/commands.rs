//! Command dispatch: composes loaders, checks and output for each `cargo xtask`
//! command, applies `--skip`, and runs the `ci` aggregate.

use std::process::ExitCode;

use crate::{cli::Cli, error::XtaskError};

/// Runs one command. `Ok(ExitCode::FAILURE)` means a check reported errors or a `ci`
/// step failed; `Err` means xtask itself failed.
pub fn run(cli: Cli) -> Result<ExitCode, XtaskError> {
    todo!()
}
