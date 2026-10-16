//! Reading the tracker and its lockfile.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use crate::error::XtaskError;

pub const TRACKER_PATH: &str = "docs/12-critique/06-pain-point-tracker.md";
pub const LOCKFILE_PATH: &str = "docs/12-critique/pain-point-ids.lock";

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

impl SourceText {
    pub fn new(path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            text: text.into(),
        }
    }
}

pub fn load(repo_root: &Path) -> Result<TrackerInput, XtaskError> {
    Ok(TrackerInput {
        tracker: read_source(repo_root, TRACKER_PATH)?,
        lockfile: read_optional_source(repo_root, LOCKFILE_PATH)?,
    })
}

fn read_source(repo_root: &Path, relative: &str) -> Result<SourceText, XtaskError> {
    let path = repo_root.join(relative);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(SourceText::new(relative, text)),
        Err(source) => Err(XtaskError::Io { path, source }),
    }
}

fn read_optional_source(
    repo_root: &Path,
    relative: &str,
) -> Result<Option<SourceText>, XtaskError> {
    match read_source(repo_root, relative) {
        Ok(source) => Ok(Some(source)),
        Err(XtaskError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}
