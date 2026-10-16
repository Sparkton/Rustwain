//! `RustOnlyFiles`: no tooling in another language.

use std::path::PathBuf;

use crate::report::Finding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedFile {
    /// Repository-relative, as printed by `git ls-files`.
    pub path: PathBuf,
    /// The file starts with `#!`.
    pub has_shebang: bool,
}

pub fn rust_only_files(files: &[TrackedFile]) -> Vec<Finding> {
    todo!()
}
