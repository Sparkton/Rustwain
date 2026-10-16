//! All policy checks.

use super::{branch_name, commit_trailers, load::PolicyInput, rust_only_files};
use crate::report::Finding;

/// Runs `RustOnlyFiles`, `CommitTrailers` and `BranchName`.
pub fn check(input: &PolicyInput) -> Vec<Finding> {
    let mut findings = rust_only_files(&input.files);
    findings.extend(commit_trailers(&input.commits));
    findings.extend(branch_name(input.branch.as_ref()));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        policy::{BranchContext, BranchSource, CommitMessage, TrackedFile},
        report::Check,
    };

    #[test]
    fn all_three_checks_contribute_to_the_policy_findings() {
        let input = PolicyInput {
            files: vec![TrackedFile::new("run.sh", false)],
            commits: vec![CommitMessage::new("abc", "Co-authored-by: Claude <c@a.b>")],
            branch: Some(BranchContext::new("wip", BranchSource::Push)),
        };

        let checks: Vec<_> = check(&input).iter().map(|finding| finding.check).collect();

        assert_eq!(
            checks,
            [
                Check::RustOnlyFiles,
                Check::CommitTrailers,
                Check::BranchName
            ]
        );
    }
}
