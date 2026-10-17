//! All policy checks.

use super::{branch_name, commit_trailers, load::PolicyInput, rust_only_files};
use crate::{report::Finding, tracker};

/// Runs `RustOnlyFiles`, `CommitTrailers` and `BranchName`, and holds the tracker
/// lockfile to being append-only since the range's base (a `TrackerIds` check).
pub fn check(input: &PolicyInput) -> Vec<Finding> {
    let mut findings = rust_only_files(&input.files);
    findings.extend(commit_trailers(&input.commits));
    findings.extend(branch_name(input.branch.as_ref()));
    findings.extend(input.lockfile.iter().flat_map(tracker::append_only));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        policy::{BranchContext, BranchSource, CommitMessage, TrackedFile},
        report::Check,
        tracker::{LOCKFILE_PATH, LockfileChange, SourceText},
    };

    #[test]
    fn all_the_policy_checks_contribute_to_the_findings() {
        let input = PolicyInput {
            files: vec![TrackedFile::new("run.sh", false)],
            commits: vec![CommitMessage::new(
                "abc",
                "Add it\n\nCo-authored-by: Claude <c@a.b>",
            )],
            branch: Some(BranchContext::new("wip", BranchSource::Push)),
            lockfile: Some(LockfileChange::new(
                SourceText::new(LOCKFILE_PATH, "PP-01\n"),
                Some(SourceText::new(LOCKFILE_PATH, "PP-02\n")),
            )),
        };

        let checks: Vec<_> = check(&input).iter().map(|finding| finding.check).collect();

        assert_eq!(
            checks,
            [
                Check::RustOnlyFiles,
                Check::CommitTrailers,
                Check::BranchName,
                Check::TrackerIds
            ]
        );
    }
}
