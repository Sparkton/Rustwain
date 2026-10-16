//! The `ci` aggregate: the local equivalent of the blocking CI jobs.
//!
//! The steps run in order and all run to completion, so one run lists every failing
//! step. A step can fail two ways. A check with errors or a cargo run that exits
//! non-zero is a failed result and exits with status 1. Xtask itself failing (a loader
//! error, a cargo binary that cannot be started) is printed, counts as a failed step,
//! and does not stop the run; the first such error is then returned, so the exit status
//! is 2.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::Path,
    process::{Command, ExitCode},
};

use super::dispatch::{DOCS_CHECK, POLICY_CHECK, check_docs, check_policy, exit_code};
use crate::{
    cli::SkipArgs,
    error::XtaskError,
    output::{Output, StepOutcome, StepResult},
    report::Report,
};

const CI_TITLE: &str = "ci";
const CARGO_VAR: &str = "CARGO";
const DEFAULT_CARGO: &str = "cargo";

const STEPS: [Step; 5] = [
    Step::DocsCheck,
    Step::PolicyCheck,
    Step::Cargo(CargoStep::Fmt),
    Step::Cargo(CargoStep::Clippy),
    Step::Cargo(CargoStep::Test),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    DocsCheck,
    /// Over the policy loader's default commit range.
    PolicyCheck,
    Cargo(CargoStep),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CargoStep {
    Fmt,
    Clippy,
    Test,
}

/// Where each step runs and how it reports.
struct Ci<'a> {
    root: &'a Path,
    output: &'a Output,
    skip: &'a SkipArgs,
    cargo: OsString,
}

pub(super) fn run(root: &Path, output: &Output, skip: &SkipArgs) -> Result<ExitCode, XtaskError> {
    let ci = Ci {
        root,
        output,
        skip,
        cargo: cargo_program(env::var_os(CARGO_VAR)),
    };
    let steps = run_steps(&STEPS, |step| {
        ci.run_step(step)
            .inspect_err(|error| output.tool_failure(error))
    });
    output.steps(CI_TITLE, &steps.results)?;
    output.skips(skip)?;
    steps.exit_status()
}

/// What a `ci` run produced.
struct Steps {
    /// One per step, in order.
    results: Vec<StepResult>,
    /// The first failure of xtask itself; later ones were reported but are not kept.
    first_error: Option<XtaskError>,
}

/// Runs every step, whatever the earlier ones returned, and pairs each with its name. A
/// step that returns an error counts as failed.
fn run_steps(
    steps: &[Step],
    mut run_step: impl FnMut(Step) -> Result<StepOutcome, XtaskError>,
) -> Steps {
    let mut run = Steps {
        results: Vec::with_capacity(steps.len()),
        first_error: None,
    };
    for &step in steps {
        let outcome = match run_step(step) {
            Ok(outcome) => outcome,
            Err(error) => {
                run.first_error.get_or_insert(error);
                StepOutcome::Failed
            }
        };
        run.results.push(StepResult::new(step.name(), outcome));
    }
    run
}

impl Steps {
    /// Status 1 when any step failed. A tool error is returned as is, so `main` exits
    /// with 2 even though every step ran.
    fn exit_status(self) -> Result<ExitCode, XtaskError> {
        match self.first_error {
            Some(error) => Err(error),
            None => Ok(exit_code(
                self.results
                    .iter()
                    .all(|result| result.outcome == StepOutcome::Passed),
            )),
        }
    }
}

impl Ci<'_> {
    fn run_step(&self, step: Step) -> Result<StepOutcome, XtaskError> {
        match step {
            Step::DocsCheck => self.report(DOCS_CHECK, &check_docs(self.root, self.skip)?),
            Step::PolicyCheck => {
                self.report(POLICY_CHECK, &check_policy(self.root, None, self.skip)?)
            }
            Step::Cargo(cargo_step) => {
                self.output.step_started(&step.name());
                run_program(&self.cargo, cargo_step.args(), self.root)
            }
        }
    }

    fn report(&self, title: &str, report: &Report) -> Result<StepOutcome, XtaskError> {
        self.output.check_report(title, report)?;
        Ok(StepOutcome::from_success(!report.has_errors()))
    }
}

impl Step {
    fn name(self) -> String {
        match self {
            Self::DocsCheck => DOCS_CHECK.to_owned(),
            Self::PolicyCheck => POLICY_CHECK.to_owned(),
            Self::Cargo(cargo_step) => format!("cargo {}", cargo_step.args().join(" ")),
        }
    }
}

impl CargoStep {
    /// The same commands as the `rust` CI job, so a local run matches CI.
    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Fmt => &["fmt", "--all", "--", "--check"],
            Self::Clippy => &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
            Self::Test => &["test", "--workspace", "-q"],
        }
    }
}

/// The cargo that started xtask (`CARGO` is set by `cargo run`), so the steps use the
/// same toolchain; plain `cargo` when xtask is run directly.
fn cargo_program(var: Option<OsString>) -> OsString {
    var.filter(|program| !program.is_empty())
        .unwrap_or_else(|| OsString::from(DEFAULT_CARGO))
}

/// Runs `program` in `root` with inherited stdio. A non-zero exit is a failed step; only
/// failing to start the program is an error.
fn run_program(program: &OsStr, args: &[&str], root: &Path) -> Result<StepOutcome, XtaskError> {
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|source| XtaskError::Spawn {
            program: program.to_string_lossy().into_owned(),
            source,
        })?;
    Ok(StepOutcome::from_success(status.success()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_the_blocking_job_commands_in_order() {
        let names: Vec<String> = STEPS.iter().map(|step| step.name()).collect();

        assert_eq!(
            names,
            [
                "docs check",
                "policy check",
                "cargo fmt --all -- --check",
                "cargo clippy --workspace --all-targets -- -D warnings",
                "cargo test --workspace -q",
            ]
        );
    }

    fn outcomes(steps: &Steps) -> Vec<StepOutcome> {
        steps.results.iter().map(|result| result.outcome).collect()
    }

    fn tool_error(input: &str) -> XtaskError {
        XtaskError::InvalidRange {
            input: input.to_owned(),
        }
    }

    #[test]
    fn every_step_runs_after_a_failure_and_the_run_fails() {
        let mut started = Vec::new();

        let steps = run_steps(&STEPS, |step| {
            started.push(step);
            Ok(StepOutcome::from_success(step != Step::PolicyCheck))
        });

        assert_eq!(started, STEPS);
        assert_eq!(
            outcomes(&steps),
            [
                StepOutcome::Passed,
                StepOutcome::Failed,
                StepOutcome::Passed,
                StepOutcome::Passed,
                StepOutcome::Passed,
            ]
        );
        assert_eq!(steps.exit_status().unwrap(), ExitCode::FAILURE);
    }

    #[test]
    fn run_succeeds_when_every_step_passed() {
        let steps = run_steps(&STEPS, |_| Ok(StepOutcome::Passed));

        assert_eq!(steps.exit_status().unwrap(), ExitCode::SUCCESS);
    }

    #[test]
    fn tool_error_fails_its_step_and_the_first_error_is_returned_after_every_step_ran() {
        let mut started = Vec::new();

        let steps = run_steps(&STEPS, |step| {
            started.push(step);
            match step {
                Step::PolicyCheck => Err(tool_error("first")),
                Step::Cargo(CargoStep::Clippy) => Err(tool_error("second")),
                _ => Ok(StepOutcome::Passed),
            }
        });

        assert_eq!(started, STEPS);
        assert_eq!(
            outcomes(&steps),
            [
                StepOutcome::Passed,
                StepOutcome::Failed,
                StepOutcome::Passed,
                StepOutcome::Failed,
                StepOutcome::Passed,
            ]
        );
        assert!(matches!(
            steps.exit_status(),
            Err(XtaskError::InvalidRange { input }) if input == "first"
        ));
    }

    #[test]
    fn nonzero_exit_is_a_failed_step_not_an_error() {
        let root = Path::new(".");

        let succeeded = run_program(OsStr::new("true"), &[], root).unwrap();
        let failed = run_program(OsStr::new("false"), &[], root).unwrap();

        assert_eq!(succeeded, StepOutcome::Passed);
        assert_eq!(failed, StepOutcome::Failed);
    }

    #[test]
    fn missing_program_is_a_spawn_error() {
        let result = run_program(OsStr::new("xtask-no-such-program"), &[], Path::new("."));

        assert!(matches!(
            result,
            Err(XtaskError::Spawn { program, .. }) if program == "xtask-no-such-program"
        ));
    }

    #[test]
    fn cargo_comes_from_the_environment_with_a_fallback() {
        assert_eq!(
            cargo_program(Some(OsString::from("/opt/cargo"))),
            OsString::from("/opt/cargo")
        );
        assert_eq!(
            cargo_program(Some(OsString::new())),
            OsString::from("cargo")
        );
        assert_eq!(cargo_program(None), OsString::from("cargo"));
    }
}
