//! Reading the tracker and its lockfile.

use std::path::{Path, PathBuf};

use crate::error::XtaskError;

/// The raw inputs of the tracker checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerInput {
    pub tracker: SourceText,
    /// `None` when the lockfile does not exist yet.
    pub lockfile: Option<SourceText>,
}

/// A file's text and its repository-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceText {
    pub path: PathBuf,
    pub text: String,
}

pub fn load(repo_root: &Path) -> Result<TrackerInput, XtaskError> {
    todo!()
}
