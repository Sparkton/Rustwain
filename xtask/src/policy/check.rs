//! All policy checks.

use super::load::PolicyInput;
use crate::report::Finding;

/// Runs `RustOnlyFiles`, `CommitTrailers` and `BranchName`.
pub fn check(input: &PolicyInput) -> Vec<Finding> {
    todo!()
}
