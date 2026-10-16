//! `BranchName`: pushed branches are `sparkton/<brief>`.

use crate::report::Finding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchContext {
    pub name: String,
    pub source: BranchSource,
}

/// Where the branch name came from, which decides the severity of a bad name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchSource {
    /// `GITHUB_HEAD_REF` of a pull request.
    PullRequest,
    /// `GITHUB_REF_NAME` of a branch push.
    Push,
    /// The local checkout's current branch.
    Local,
}

/// `None` when there is no branch to check (detached HEAD, a tag push).
pub fn branch_name(branch: Option<&BranchContext>) -> Vec<Finding> {
    todo!()
}
