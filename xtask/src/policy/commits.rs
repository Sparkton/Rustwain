//! `CommitTrailers`: no AI co-author or attribution lines.

use crate::report::Finding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMessage {
    pub sha: String,
    pub message: String,
}

pub fn commit_trailers(commits: &[CommitMessage]) -> Vec<Finding> {
    todo!()
}
