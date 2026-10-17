//! Findings produced by checks, and the report they form (design 0001, section 5.1).

use std::{fmt, num::NonZeroU32, path::PathBuf};

/// Every check xtask can run. `--skip` parses these by their kebab-case names, which
/// [`Check::name`] returns. `BudgetSync` is implemented in T2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, clap::ValueEnum)]
pub enum Check {
    DocLinks,
    DocAnchors,
    DocIndex,
    Markers,
    TrackerIds,
    TrackerSchema,
    BudgetSync,
    RustOnlyFiles,
    CommitTrailers,
    BranchName,
}

impl Check {
    /// Stable name used on the command line, in annotations and in the job summary.
    pub fn name(self) -> &'static str {
        match self {
            Self::DocLinks => "doc-links",
            Self::DocAnchors => "doc-anchors",
            Self::DocIndex => "doc-index",
            Self::Markers => "markers",
            Self::TrackerIds => "tracker-ids",
            Self::TrackerSchema => "tracker-schema",
            Self::BudgetSync => "budget-sync",
            Self::RustOnlyFiles => "rust-only-files",
            Self::CommitTrailers => "commit-trailers",
            Self::BranchName => "branch-name",
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// A repository-relative path (forward slashes) and a 1-based line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location {
    pub path: PathBuf,
    pub line: NonZeroU32,
}

impl Location {
    pub fn new(path: impl Into<PathBuf>, line: NonZeroU32) -> Self {
        Self {
            path: path.into(),
            line,
        }
    }

    /// From a 0-based line index, as produced by `str::lines().enumerate()`. Saturates
    /// at `u32::MAX` for files longer than that.
    pub fn from_index(path: impl Into<PathBuf>, index: usize) -> Self {
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        Self::new(path, NonZeroU32::MIN.saturating_add(index))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: Check,
    pub severity: Severity,
    /// `None` for findings without a file, such as a commit message.
    pub location: Option<Location>,
    pub message: String,
}

impl Finding {
    pub fn error(check: Check, location: Option<Location>, message: impl Into<String>) -> Self {
        Self::new(check, Severity::Error, location, message)
    }

    pub fn warning(check: Check, location: Option<Location>, message: impl Into<String>) -> Self {
        Self::new(check, Severity::Warning, location, message)
    }

    fn new(
        check: Check,
        severity: Severity,
        location: Option<Location>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            check,
            severity,
            location,
            message: message.into(),
        }
    }
}

/// The exit status of a checking command is non-zero iff any finding is an `Error`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn new(findings: Vec<Finding>) -> Self {
        Self { findings }
    }

    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    }
}
