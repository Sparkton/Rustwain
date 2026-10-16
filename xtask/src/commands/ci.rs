//! The `ci` aggregate: the local equivalent of the blocking CI jobs.
//!
//! The steps run in order and all run to completion, so one run lists every failing
//! step. A step that fails is a result; only xtask itself failing (a loader error, a
//! cargo binary that cannot be started) aborts the run with an `Err`.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::Path,
    process::{Command, ExitCode},
};

use super::{DOCS_CHECK, POLICY_CHECK, check_docs, check_policy, exit_code};
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
    let results = run_steps(&STEPS, |step| ci.run_step(step))?;
    output.steps(CI_TITLE, &results)?;
    output.skips(skip)?;
    Ok(steps_exit_code(&results))
}

/// Runs every step, whatever the earlier ones returned, and pairs each with its name.
fn run_steps(
    steps: &[Step],
    mut run_step: impl FnMut(Step) -> Result<StepOutcome, XtaskError>,
) -> Result<Vec<StepResult>, XtaskError> {
    steps
        .iter()
        .map(|&step| Ok(StepResult::new(step.name(), run_step(step)?)))
        .collect()
}

fn steps_exit_code(results: &[StepResult]) -> ExitCode {
    exit_code(
        results
            .iter()
            .all(|result| result.outcome == StepOutcome::Passed),
    )
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

    #[test]
    fn every_step_runs_after_a_failure_and_the_run_fails() {
        let mut started = Vec::new();

        let results = run_steps(&STEPS, |step| {
            started.push(step);
            Ok(StepOutcome::from_success(step != Step::PolicyCheck))
        })
        .unwrap();

        assert_eq!(started, STEPS);
        assert_eq!(
            results
                .iter()
                .map(|result| result.outcome)
                .collect::<Vec<_>>(),
            [
                StepOutcome::Passed,
                StepOutcome::Failed,
                StepOutcome::Passed,
                StepOutcome::Passed,
                StepOutcome::Passed,
            ]
        );
        assert_eq!(steps_exit_code(&results), ExitCode::FAILURE);
    }

    #[test]
    fn run_succeeds_when_every_step_passed() {
        let results = run_steps(&STEPS, |_| Ok(StepOutcome::Passed)).unwrap();

        assert_eq!(steps_exit_code(&results), ExitCode::SUCCESS);
    }

    #[test]
    fn tool_failure_aborts_the_run() {
        let mut started = 0;

        let result = run_steps(&STEPS, |_| {
            started += 1;
            Err(XtaskError::InvalidRange {
                input: "x".to_owned(),
            })
        });

        assert!(result.is_err());
        assert_eq!(started, 1);
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
