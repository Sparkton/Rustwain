//! Repository policy: no non-Rust tooling files, no AI attribution in commit messages,
//! and `sparkton/<brief>` branch names.
//!
//! [`load`] runs `git` and reads the environment; the checks are pure.

mod branch;
mod check;
mod commits;
mod files;
mod git;
mod load;
mod range;

pub use branch::{BranchContext, BranchSource, branch_name};
pub use check::check;
pub use commits::{CommitMessage, commit_trailers};
pub use files::{TrackedFile, rust_only_files};
pub use load::{PolicyInput, load};
pub use range::CommitRange;
