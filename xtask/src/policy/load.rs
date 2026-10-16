//! Gathering policy inputs from `git` and the environment.

use std::path::Path;

use super::{BranchContext, CommitMessage, CommitRange, TrackedFile};
use crate::error::XtaskError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyInput {
    pub files: Vec<TrackedFile>,
    pub commits: Vec<CommitMessage>,
    pub branch: Option<BranchContext>,
}

/// Lists tracked files, reads the commits in `range` (default `origin/main..HEAD`) and
/// works out the branch under check.
pub fn load(repo_root: &Path, range: Option<&CommitRange>) -> Result<PolicyInput, XtaskError> {
    todo!()
}
