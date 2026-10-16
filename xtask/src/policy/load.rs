//! Gathering policy inputs from `git` and the environment.

use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::Path,
};

use super::{
    BranchContext, BranchSource, CommitMessage, CommitRange, TrackedFile,
    branch::{BranchLookup, GithubEnv},
    git::{self, Revisions},
};
use crate::error::XtaskError;

const SHEBANG: &[u8] = b"#!";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyInput {
    pub files: Vec<TrackedFile>,
    pub commits: Vec<CommitMessage>,
    pub branch: Option<BranchContext>,
}

/// Lists tracked files, reads the commits in `range` (default `origin/main..HEAD`) and
/// works out the branch under check.
pub fn load(repo_root: &Path, range: Option<&CommitRange>) -> Result<PolicyInput, XtaskError> {
    Ok(PolicyInput {
        files: load_files(repo_root)?,
        commits: load_commits(repo_root, range)?,
        branch: load_branch(repo_root, &GithubEnv::from_process())?,
    })
}

fn load_files(repo_root: &Path) -> Result<Vec<TrackedFile>, XtaskError> {
    git::tracked_paths(repo_root)?
        .into_iter()
        .map(|path| {
            let has_shebang =
                path.extension().is_none() && starts_with_shebang(&repo_root.join(&path))?;
            Ok(TrackedFile::new(path, has_shebang))
        })
        .collect()
}

/// A tracked file that is missing from the working tree or is a directory (a submodule)
/// has no shebang.
fn starts_with_shebang(path: &Path) -> Result<bool, XtaskError> {
    let mut prefix = Vec::with_capacity(SHEBANG.len());
    let read = File::open(path).and_then(|file| file.take(2).read_to_end(&mut prefix));
    match read {
        Ok(_) => Ok(prefix == SHEBANG),
        Err(source) if matches!(source.kind(), ErrorKind::NotFound | ErrorKind::IsADirectory) => {
            Ok(false)
        }
        Err(source) => Err(XtaskError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn load_commits(
    repo_root: &Path,
    range: Option<&CommitRange>,
) -> Result<Vec<CommitMessage>, XtaskError> {
    let revisions = Revisions::select(range, || git::ref_exists(repo_root, git::DEFAULT_BASE))?;
    git::commit_messages(repo_root, revisions)
}

fn load_branch(repo_root: &Path, env: &GithubEnv) -> Result<Option<BranchContext>, XtaskError> {
    match BranchLookup::from_env(env) {
        BranchLookup::Named(branch) => Ok(Some(branch)),
        BranchLookup::NoBranch => Ok(None),
        BranchLookup::LocalCheckout => Ok(git::current_branch(repo_root)?
            .map(|name| BranchContext::new(name, BranchSource::Local))),
    }
}
