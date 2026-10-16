//! Rendering reports: terminal text, GitHub workflow annotations at `file:line`, and
//! the job summary (the file named by `GITHUB_STEP_SUMMARY`).
//!
//! Every `render_*` function is pure and returns text with one `\n` per line. [`Output`]
//! is the thin writer on top of them: it prints, and appends markdown to the summary
//! file. Nothing here truncates the summary, because every step of a job shares it.

use std::{env, ffi::OsString, fs::OpenOptions, io::Write, iter, path::PathBuf};

use crate::{
    cli::SkipArgs,
    error::XtaskError,
    report::{Check, Finding, Location, Report, Severity},
};

const GITHUB_ACTIONS_VAR: &str = "GITHUB_ACTIONS";
const GITHUB_ACTIONS_ENABLED: &str = "true";
const STEP_SUMMARY_VAR: &str = "GITHUB_STEP_SUMMARY";

/// Printed when `--skip` carries no reason. Unreachable through the CLI, where clap's
/// `requires` rejects it, but `SkipArgs` can be built by hand.
const MISSING_SKIP_REASON: &str = "no reason given";

/// Where a command's output goes: always the terminal, plus workflow commands under
/// GitHub Actions and the job summary when `GITHUB_STEP_SUMMARY` names a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    workflow_commands: bool,
    summary: Option<PathBuf>,
}

/// One row of the `ci` step table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    pub name: String,
    pub outcome: StepOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Passed,
    Failed,
}

impl Output {
    pub fn new(workflow_commands: bool, summary: Option<PathBuf>) -> Self {
        Self {
            workflow_commands,
            summary,
        }
    }

    pub fn from_env() -> Self {
        Self::from_vars(
            env::var(GITHUB_ACTIONS_VAR).ok().as_deref(),
            env::var_os(STEP_SUMMARY_VAR),
        )
    }

    /// Prints the findings of one check run and, under GitHub Actions, their
    /// annotations; appends the findings table to the job summary.
    pub fn check_report(&self, title: &str, report: &Report) -> Result<(), XtaskError> {
        print!("{}", render_terminal(title, report));
        if self.workflow_commands {
            print!("{}", render_annotations(report));
        }
        self.append_summary(&render_check_summary(title, report))
    }

    /// Prints the `skipped <check>: <reason>` lines and appends them to the job summary.
    /// Writes nothing when no check is skipped.
    pub fn skips(&self, skip: &SkipArgs) -> Result<(), XtaskError> {
        print!("{}", render_skips_terminal(skip));
        self.append_summary(&render_skips_summary(skip))
    }

    /// Marks where a step that writes its own output (a cargo run) begins.
    pub fn step_started(&self, name: &str) {
        println!("==> {name}");
    }

    /// Prints the final pass/fail table of the `ci` steps and appends it to the summary.
    pub fn steps(&self, title: &str, steps: &[StepResult]) -> Result<(), XtaskError> {
        print!("{}", render_steps_terminal(title, steps));
        self.append_summary(&render_steps_summary(title, steps))
    }

    /// Prints a markdown body as is, and appends it to the summary under a heading.
    pub fn markdown_report(&self, title: &str, body: &str) -> Result<(), XtaskError> {
        println!("{}", body.trim_end());
        self.append_summary(&render_markdown_summary(title, body))
    }

    pub fn lock_ids(&self, ids: &[&str]) {
        print!("{}", render_lock_ids(ids));
    }

    fn from_vars(github_actions: Option<&str>, step_summary: Option<OsString>) -> Self {
        Self::new(
            github_actions == Some(GITHUB_ACTIONS_ENABLED),
            step_summary
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        )
    }

    fn append_summary(&self, markdown: &str) -> Result<(), XtaskError> {
        let Some(path) = &self.summary else {
            return Ok(());
        };
        if markdown.is_empty() {
            return Ok(());
        }
        let io_error = |source| XtaskError::Io {
            path: path.clone(),
            source,
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(io_error)?;
        file.write_all(markdown.as_bytes()).map_err(io_error)
    }
}

impl StepResult {
    pub fn new(name: impl Into<String>, outcome: StepOutcome) -> Self {
        Self {
            name: name.into(),
            outcome,
        }
    }
}

impl StepOutcome {
    pub fn from_success(success: bool) -> Self {
        if success { Self::Passed } else { Self::Failed }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

/// One line per finding, then the counts.
fn render_terminal(title: &str, report: &Report) -> String {
    join_lines(
        report
            .findings
            .iter()
            .map(terminal_line)
            .chain(iter::once(count_line(title, report))),
    )
}

fn terminal_line(finding: &Finding) -> String {
    let severity = severity_label(finding.severity);
    let check = finding.check.name();
    match &finding.location {
        Some(location) => format!(
            "{severity}[{check}]: {}: {}",
            location_text(location),
            finding.message
        ),
        None => format!("{severity}[{check}]: {}", finding.message),
    }
}

fn count_line(title: &str, report: &Report) -> String {
    let errors = count_severity(report, Severity::Error);
    let warnings = count_severity(report, Severity::Warning);
    format!(
        "{title}: {}, {}",
        counted(errors, "error"),
        counted(warnings, "warning")
    )
}

fn count_severity(report: &Report, severity: Severity) -> usize {
    report
        .findings
        .iter()
        .filter(|finding| finding.severity == severity)
        .count()
}

fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

fn location_text(location: &Location) -> String {
    format!("{}:{}", location.path.display(), location.line)
}

/// GitHub workflow commands, which the Actions UI turns into annotations on the file.
fn render_annotations(report: &Report) -> String {
    join_lines(report.findings.iter().map(annotation))
}

fn annotation(finding: &Finding) -> String {
    let level = severity_label(finding.severity);
    let title = escape_property(finding.check.name());
    let properties = match &finding.location {
        Some(location) => format!(
            "file={},line={},title={title}",
            escape_property(&location.path.to_string_lossy()),
            location.line
        ),
        None => format!("title={title}"),
    };
    format!("::{level} {properties}::{}", escape_data(&finding.message))
}

/// Escapes the message of a workflow command the way GitHub's toolkit does.
fn escape_data(text: &str) -> String {
    escape(text, data_replacement)
}

/// Escapes a workflow command property, which additionally cannot contain the `:` and
/// `,` that delimit the property list.
fn escape_property(text: &str) -> String {
    escape(text, |c| match c {
        ':' => Some("%3A"),
        ',' => Some("%2C"),
        other => data_replacement(other),
    })
}

fn data_replacement(c: char) -> Option<&'static str> {
    match c {
        '%' => Some("%25"),
        '\r' => Some("%0D"),
        '\n' => Some("%0A"),
        _ => None,
    }
}

fn escape(text: &str, replacement: fn(char) -> Option<&'static str>) -> String {
    text.chars()
        .fold(String::with_capacity(text.len()), |mut escaped, c| {
            match replacement(c) {
                Some(replacement) => escaped.push_str(replacement),
                None => escaped.push(c),
            }
            escaped
        })
}

fn render_check_summary(title: &str, report: &Report) -> String {
    let body = if report.findings.is_empty() {
        "No findings.\n".to_owned()
    } else {
        findings_table(report)
    };
    format!("## {title}\n\n{body}\n")
}

fn findings_table(report: &Report) -> String {
    markdown_table(
        &["Severity", "Check", "Location", "Message"],
        report.findings.iter().map(|finding| {
            vec![
                severity_label(finding.severity).to_owned(),
                format!("`{}`", finding.check),
                finding.location.as_ref().map_or_else(
                    || "-".to_owned(),
                    |location| format!("`{}`", location_text(location)),
                ),
                finding.message.clone(),
            ]
        }),
    )
}

fn render_skips_terminal(skip: &SkipArgs) -> String {
    join_lines(skipped(skip).map(|(check, reason)| format!("skipped {check}: {reason}")))
}

fn render_skips_summary(skip: &SkipArgs) -> String {
    if skip.checks.is_empty() {
        return String::new();
    }
    let lines =
        join_lines(skipped(skip).map(|(check, reason)| format!("- Skipped `{check}`: {reason}")));
    format!("{lines}\n")
}

fn skipped(skip: &SkipArgs) -> impl Iterator<Item = (Check, &str)> {
    let reason = skip.skip_reason.as_deref().unwrap_or(MISSING_SKIP_REASON);
    skip.checks.iter().map(move |&check| (check, reason))
}

fn render_steps_terminal(title: &str, steps: &[StepResult]) -> String {
    let width = steps
        .iter()
        .map(|step| step.name.chars().count())
        .max()
        .unwrap_or_default();
    let rows = steps
        .iter()
        .map(|step| format!("  {:<width$}  {}", step.name, step.outcome.label()));
    let tally = format!(
        "{title}: {} passed, {} failed",
        count_outcome(steps, StepOutcome::Passed),
        count_outcome(steps, StepOutcome::Failed)
    );
    join_lines(
        iter::once(format!("{title} steps:"))
            .chain(rows)
            .chain(iter::once(tally)),
    )
}

fn count_outcome(steps: &[StepResult], outcome: StepOutcome) -> usize {
    steps.iter().filter(|step| step.outcome == outcome).count()
}

fn render_steps_summary(title: &str, steps: &[StepResult]) -> String {
    let table = markdown_table(
        &["Step", "Result"],
        steps
            .iter()
            .map(|step| vec![format!("`{}`", step.name), step.outcome.label().to_owned()]),
    );
    format!("## {title}\n\n{table}\n")
}

fn render_markdown_summary(title: &str, body: &str) -> String {
    format!("## {title}\n\n{}\n\n", body.trim_end())
}

fn render_lock_ids(ids: &[&str]) -> String {
    if ids.is_empty() {
        return "lockfile up to date\n".to_owned();
    }
    join_lines(ids.iter().map(|id| format!("appended {id}")))
}

fn markdown_table(headers: &[&str], rows: impl IntoIterator<Item = Vec<String>>) -> String {
    let header = table_row(headers.iter().map(|header| (*header).to_owned()));
    let rule = table_row(headers.iter().map(|_| "---".to_owned()));
    join_lines(
        [header, rule]
            .into_iter()
            .chain(rows.into_iter().map(table_row)),
    )
}

fn table_row(cells: impl IntoIterator<Item = String>) -> String {
    let cells: Vec<String> = cells.into_iter().map(|cell| escape_cell(&cell)).collect();
    format!("| {} |", cells.join(" | "))
}

/// Keeps a cell on one table row: `|` would end the cell and a newline would end the row.
fn escape_cell(text: &str) -> String {
    text.replace('\r', "")
        .replace('|', "\\|")
        .replace('\n', "<br>")
}

fn join_lines(lines: impl IntoIterator<Item = String>) -> String {
    lines.into_iter().map(|line| line + "\n").collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, num::NonZeroU32, process};

    use super::*;

    fn at(path: &str, line: u32) -> Option<Location> {
        Some(Location::new(path, NonZeroU32::new(line).unwrap()))
    }

    fn skip(checks: Vec<Check>, reason: &str) -> SkipArgs {
        SkipArgs {
            checks,
            skip_reason: Some(reason.to_owned()),
        }
    }

    #[test]
    fn terminal_lines_show_the_location_only_when_there_is_one() {
        let report = Report::new(vec![
            Finding::error(Check::DocLinks, at("docs/x.md", 12), "broken link"),
            Finding::error(Check::CommitTrailers, None, "AI trailer in abc123"),
            Finding::warning(Check::Markers, at("docs/y.md", 3), "odd marker"),
        ]);

        let text = render_terminal("docs check", &report);

        assert_eq!(
            text,
            "error[doc-links]: docs/x.md:12: broken link\n\
             error[commit-trailers]: AI trailer in abc123\n\
             warning[markers]: docs/y.md:3: odd marker\n\
             docs check: 2 errors, 1 warning\n"
        );
    }

    #[test]
    fn clean_report_prints_only_the_counts() {
        let text = render_terminal("policy check", &Report::default());

        assert_eq!(text, "policy check: 0 errors, 0 warnings\n");
    }

    #[test]
    fn annotations_set_file_line_and_title() {
        let report = Report::new(vec![
            Finding::error(Check::DocLinks, at("docs/x.md", 12), "broken link"),
            Finding::warning(Check::BranchName, None, "branch is not sparkton/*"),
        ]);

        let text = render_annotations(&report);

        assert_eq!(
            text,
            "::error file=docs/x.md,line=12,title=doc-links::broken link\n\
             ::warning title=branch-name::branch is not sparkton/*\n"
        );
    }

    #[test]
    fn annotations_escape_data_and_properties_as_github_does() {
        let report = Report::new(vec![Finding::error(
            Check::DocLinks,
            at("docs/a:b,c%.md", 1),
            "100%\r\nsecond: line, still data",
        )]);

        let text = render_annotations(&report);

        assert_eq!(
            text,
            "::error file=docs/a%3Ab%2Cc%25.md,line=1,title=doc-links::100%25%0D%0Asecond: line, still data\n"
        );
    }

    #[test]
    fn workflow_commands_are_enabled_only_when_github_actions_is_true() {
        let enabled = Output::from_vars(Some("true"), None);
        let other_value = Output::from_vars(Some("1"), None);
        let unset = Output::from_vars(None, None);

        assert_eq!(enabled, Output::new(true, None));
        assert_eq!(other_value, Output::new(false, None));
        assert_eq!(unset, Output::new(false, None));
    }

    #[test]
    fn empty_summary_path_means_no_summary() {
        let output = Output::from_vars(None, Some(OsString::new()));

        assert_eq!(output, Output::new(false, None));
    }

    #[test]
    fn summary_table_escapes_pipes_and_marks_missing_locations() {
        let report = Report::new(vec![
            Finding::error(Check::DocLinks, at("docs/x.md", 12), "bad `a|b` link"),
            Finding::warning(Check::CommitTrailers, None, "first\nsecond"),
        ]);

        let markdown = render_check_summary("docs check", &report);

        assert_eq!(
            markdown,
            "## docs check\n\
             \n\
             | Severity | Check | Location | Message |\n\
             | --- | --- | --- | --- |\n\
             | error | `doc-links` | `docs/x.md:12` | bad `a\\|b` link |\n\
             | warning | `commit-trailers` | - | first<br>second |\n\
             \n"
        );
    }

    #[test]
    fn summary_without_findings_says_so() {
        let markdown = render_check_summary("policy check", &Report::default());

        assert_eq!(markdown, "## policy check\n\nNo findings.\n\n");
    }

    #[test]
    fn skips_name_each_check_with_the_reason() {
        let skip = skip(vec![Check::DocLinks, Check::Markers], "xtask bug 12");

        assert_eq!(
            render_skips_terminal(&skip),
            "skipped doc-links: xtask bug 12\nskipped markers: xtask bug 12\n"
        );
        assert_eq!(
            render_skips_summary(&skip),
            "- Skipped `doc-links`: xtask bug 12\n- Skipped `markers`: xtask bug 12\n\n"
        );
    }

    #[test]
    fn no_skips_render_nothing() {
        let skip = SkipArgs::default();

        assert_eq!(render_skips_terminal(&skip), "");
        assert_eq!(render_skips_summary(&skip), "");
    }

    #[test]
    fn step_table_lists_every_step_then_tallies() {
        let steps = [
            StepResult::new("docs check", StepOutcome::Passed),
            StepResult::new("cargo fmt --all -- --check", StepOutcome::Failed),
            StepResult::new("cargo test --workspace -q", StepOutcome::Passed),
        ];

        assert_eq!(
            render_steps_terminal("ci", &steps),
            "ci steps:\n\
             \x20 docs check                  passed\n\
             \x20 cargo fmt --all -- --check  failed\n\
             \x20 cargo test --workspace -q   passed\n\
             ci: 2 passed, 1 failed\n"
        );
        assert_eq!(
            render_steps_summary("ci", &steps),
            "## ci\n\
             \n\
             | Step | Result |\n\
             | --- | --- |\n\
             | `docs check` | passed |\n\
             | `cargo fmt --all -- --check` | failed |\n\
             | `cargo test --workspace -q` | passed |\n\
             \n"
        );
    }

    #[test]
    fn locked_ids_are_listed_or_reported_up_to_date() {
        assert_eq!(
            render_lock_ids(&["PP-1A", "PP-R3"]),
            "appended PP-1A\nappended PP-R3\n"
        );
        assert_eq!(render_lock_ids(&[]), "lockfile up to date\n");
    }

    #[test]
    fn summary_is_appended_to_existing_content() {
        let path = env::temp_dir().join(format!("xtask-summary-append-{}", process::id()));
        fs::write(&path, "earlier step\n").unwrap();
        let output = Output::new(false, Some(path.clone()));
        let report = Report::new(vec![Finding::error(
            Check::DocIndex,
            at("docs/z.md", 1),
            "unreachable",
        )]);

        output.check_report("docs check", &report).unwrap();
        output
            .skips(&skip(vec![Check::DocLinks], "xtask bug 12"))
            .unwrap();

        let written = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(
            written,
            "earlier step\n\
             ## docs check\n\
             \n\
             | Severity | Check | Location | Message |\n\
             | --- | --- | --- | --- |\n\
             | error | `doc-index` | `docs/z.md:1` | unreachable |\n\
             \n\
             - Skipped `doc-links`: xtask bug 12\n\
             \n"
        );
    }

    #[test]
    fn unwritable_summary_is_an_io_error() {
        let path = env::temp_dir().join(format!("xtask-no-such-dir-{}/summary", process::id()));
        let output = Output::new(false, Some(path.clone()));

        let result = output.check_report("docs check", &Report::default());

        assert!(matches!(result, Err(XtaskError::Io { path: failed, .. }) if failed == path));
    }
}
