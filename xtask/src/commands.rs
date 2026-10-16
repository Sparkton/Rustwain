//! Command dispatch: composes loaders, checks and output for each `cargo xtask`
//! command, applies `--skip`, and runs the `ci` aggregate.
//!
//! Exit status: 0 when no finding is an error and every step passed, 1 otherwise. A
//! failure of xtask itself is an `Err`, which `main` maps to status 2.

mod ci;

use std::{path::Path, process::ExitCode};

use crate::{
    cli::{Cli, Command, DocsCommand, PolicyCheckArgs, PolicyCommand, SkipArgs},
    docs,
    error::XtaskError,
    output::Output,
    policy::{self, CommitRange},
    report::{Finding, Report},
    tracker::{self, PainPointId},
};

const DOCS_CHECK: &str = "docs check";
const DOCS_REPORT: &str = "docs report";
const POLICY_CHECK: &str = "policy check";

/// Runs one command. `Ok(ExitCode::FAILURE)` means a check reported errors or a `ci`
/// step failed; `Err` means xtask itself failed.
pub fn run(cli: Cli) -> Result<ExitCode, XtaskError> {
    let root = workspace_root();
    let output = Output::from_env();
    match cli.command {
        Command::Ci(args) => ci::run(root, &output, &args.skip),
        Command::Docs(DocsCommand::Check(skip)) => docs_check(root, &output, &skip),
        Command::Docs(DocsCommand::Report) => docs_report(root, &output),
        Command::Docs(DocsCommand::LockIds) => docs_lock_ids(root, &output),
        Command::Policy(PolicyCommand::Check(args)) => policy_check(root, &output, &args),
    }
}

fn docs_check(root: &Path, output: &Output, skip: &SkipArgs) -> Result<ExitCode, XtaskError> {
    let report = check_docs(root, skip)?;
    finish_check(output, DOCS_CHECK, &report, skip)
}

fn docs_report(root: &Path, output: &Output) -> Result<ExitCode, XtaskError> {
    let doc_set = docs::load(root)?;
    output.markdown_report(DOCS_REPORT, &docs::marker_report(&doc_set))?;
    Ok(ExitCode::SUCCESS)
}

fn docs_lock_ids(root: &Path, output: &Output) -> Result<ExitCode, XtaskError> {
    let appended = tracker::lock_ids(root)?;
    let ids: Vec<&str> = appended.iter().map(PainPointId::as_str).collect();
    output.lock_ids(&ids);
    Ok(ExitCode::SUCCESS)
}

fn policy_check(
    root: &Path,
    output: &Output,
    args: &PolicyCheckArgs,
) -> Result<ExitCode, XtaskError> {
    let report = check_policy(root, args.range.as_ref(), &args.skip)?;
    finish_check(output, POLICY_CHECK, &report, &args.skip)
}

fn finish_check(
    output: &Output,
    title: &str,
    report: &Report,
    skip: &SkipArgs,
) -> Result<ExitCode, XtaskError> {
    output.check_report(title, report)?;
    output.skips(skip)?;
    Ok(report_exit_code(report))
}

/// The docs and tracker checks over the repository, minus the skipped checks.
fn check_docs(root: &Path, skip: &SkipArgs) -> Result<Report, XtaskError> {
    let doc_set = docs::load(root)?;
    let tracker_input = tracker::load(root)?;
    let findings = docs::check(&doc_set)
        .into_iter()
        .chain(tracker::check(&tracker_input))
        .collect();
    Ok(apply_skip(findings, skip))
}

/// The policy checks over the repository, minus the skipped checks. `range` defaults to
/// the policy loader's own default.
fn check_policy(
    root: &Path,
    range: Option<&CommitRange>,
    skip: &SkipArgs,
) -> Result<Report, XtaskError> {
    let input = policy::load(root, range)?;
    Ok(apply_skip(policy::check(&input), skip))
}

/// Drops the findings of the checks named by `--skip`. A skip for a check that did not
/// run, or found nothing, changes nothing.
fn apply_skip(findings: Vec<Finding>, skip: &SkipArgs) -> Report {
    Report::new(
        findings
            .into_iter()
            .filter(|finding| !skip.checks.contains(&finding.check))
            .collect(),
    )
}

fn report_exit_code(report: &Report) -> ExitCode {
    exit_code(!report.has_errors())
}

fn exit_code(success: bool) -> ExitCode {
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask's manifest directory is `<workspace>/xtask`, so it has a parent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Check;

    fn skip(checks: Vec<Check>) -> SkipArgs {
        SkipArgs {
            checks,
            skip_reason: Some("xtask bug".to_owned()),
        }
    }

    #[test]
    fn skip_drops_only_the_named_checks() {
        let findings = vec![
            Finding::error(Check::DocLinks, None, "first"),
            Finding::error(Check::DocAnchors, None, "second"),
            Finding::warning(Check::DocLinks, None, "third"),
            Finding::error(Check::TrackerIds, None, "fourth"),
        ];

        let report = apply_skip(findings, &skip(vec![Check::DocLinks, Check::BranchName]));

        assert_eq!(
            report,
            Report::new(vec![
                Finding::error(Check::DocAnchors, None, "second"),
                Finding::error(Check::TrackerIds, None, "fourth"),
            ])
        );
    }

    #[test]
    fn exit_status_fails_only_on_error_findings() {
        let errors = Report::new(vec![
            Finding::warning(Check::Markers, None, "odd"),
            Finding::error(Check::DocLinks, None, "broken"),
        ]);
        let warnings = Report::new(vec![Finding::warning(Check::Markers, None, "odd")]);

        assert_eq!(report_exit_code(&errors), ExitCode::FAILURE);
        assert_eq!(report_exit_code(&warnings), ExitCode::SUCCESS);
        assert_eq!(report_exit_code(&Report::default()), ExitCode::SUCCESS);
    }
}
