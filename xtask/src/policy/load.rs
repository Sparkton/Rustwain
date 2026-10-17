//! Gathering policy inputs from `git` and the environment.

use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

use super::{
    BranchContext, BranchSource, CommitMessage, CommitRange, TrackedFile,
    branch::{BranchLookup, GithubEnv},
    git::{self, Revisions},
};
use crate::{
    error::XtaskError,
    tracker::{self, LOCKFILE_PATH, LockfileChange, SourceText},
};

const SHEBANG: &[u8] = b"#!";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyInput {
    pub files: Vec<TrackedFile>,
    pub commits: Vec<CommitMessage>,
    pub branch: Option<BranchContext>,
    /// The tracker lockfile where the commits forked from their base, and now. `None` when
    /// there is no base to compare with, or the fork point has no lockfile.
    pub lockfile: Option<LockfileChange>,
}

/// Lists tracked files, reads the commits in `range` (default `origin/main..HEAD`), works
/// out the branch under check, and reads the lockfile at the range's base.
///
/// A base that does not resolve (for example after a force-push) is treated like a first
/// push, so only the head commit is inspected.
pub fn load(repo_root: &Path, range: Option<&CommitRange>) -> Result<PolicyInput, XtaskError> {
    let revisions = Revisions::select(range, |revision| git::commit_exists(repo_root, revision))?;
    Ok(PolicyInput {
        files: tracked_files(repo_root, git::tracked_paths(repo_root)?)?,
        commits: git::commit_messages(repo_root, revisions)?,
        branch: load_branch(repo_root, &GithubEnv::from_process())?,
        lockfile: load_lockfile_change(repo_root, revisions)?,
    })
}

fn tracked_files(repo_root: &Path, paths: Vec<PathBuf>) -> Result<Vec<TrackedFile>, XtaskError> {
    paths
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

fn load_lockfile_change(
    repo_root: &Path,
    revisions: Revisions<'_>,
) -> Result<Option<LockfileChange>, XtaskError> {
    lockfile_change(
        revisions,
        |base, head| git::merge_base(repo_root, base, head),
        |commit| git::file_at(repo_root, commit, LOCKFILE_PATH),
        || tracker::load_lockfile(repo_root),
    )
}

/// The lockfile where the commits branched off the base, against the one on disk.
///
/// The base side is read at the merge-base of base and head, the fork point, not at the
/// base tip: a branch that is behind the base (the base has appended IDs the branch has
/// not merged yet) would otherwise look as if it had dropped them.
///
/// `None` when there is nothing to compare with: the revisions have no base, share no
/// history, or the fork point has no lockfile. The lookups are injected so the decision
/// runs without a repository: `merge_base` takes the base and head revisions,
/// `lockfile_at` a commit, and `working_tree_lockfile` is read only once a base exists.
fn lockfile_change(
    revisions: Revisions<'_>,
    merge_base: impl FnOnce(&str, &str) -> Result<Option<String>, XtaskError>,
    lockfile_at: impl FnOnce(&str) -> Result<Option<String>, XtaskError>,
    working_tree_lockfile: impl FnOnce() -> Result<Option<SourceText>, XtaskError>,
) -> Result<Option<LockfileChange>, XtaskError> {
    let Revisions::Span { base, head } = revisions else {
        return Ok(None);
    };
    let Some(fork_point) = merge_base(base, head)? else {
        return Ok(None);
    };
    let Some(base_text) = lockfile_at(&fork_point)? else {
        return Ok(None);
    };
    Ok(Some(LockfileChange::new(
        SourceText::new(LOCKFILE_PATH, base_text),
        working_tree_lockfile()?,
    )))
}

fn load_branch(repo_root: &Path, env: &GithubEnv) -> Result<Option<BranchContext>, XtaskError> {
    match BranchLookup::from_env(env) {
        BranchLookup::Named(branch) => Ok(Some(branch)),
        BranchLookup::NoBranch => Ok(None),
        BranchLookup::LocalCheckout => Ok(git::current_branch(repo_root)?
            .map(|name| BranchContext::new(name, BranchSource::Local))),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{policy::rust_only_files, tracker::temp_dir::TempDir};

    const SCRIPT: &str = "#!/bin/sh\necho hi\n";
    const SPAN: Revisions<'static> = Revisions::Span {
        base: "main",
        head: "topic",
    };

    fn merge_base_not_asked(_: &str, _: &str) -> Result<Option<String>, XtaskError> {
        unreachable!("the merge-base is not looked up here")
    }

    fn lockfile_not_asked(_: &str) -> Result<Option<String>, XtaskError> {
        unreachable!("the base lockfile is not read here")
    }

    fn working_tree_not_asked() -> Result<Option<SourceText>, XtaskError> {
        unreachable!("the working-tree lockfile is not read here")
    }

    #[test]
    fn shebang_is_found_in_scripts_and_not_in_other_files() {
        let dir = TempDir::new("starts_with_shebang");
        dir.write("script", SCRIPT);
        dir.write("plain", "hello\n");
        fs::create_dir(dir.path().join("subdir")).unwrap();

        let results = ["script", "plain", "missing", "subdir"]
            .map(|name| starts_with_shebang(&dir.path().join(name)).unwrap());

        assert_eq!(results, [true, false, false, false]);
    }

    #[test]
    fn an_extensionless_script_is_flagged_but_a_document_starting_with_shebang_text_is_not() {
        let dir = TempDir::new("tracked_files_shebang");
        dir.write("bin/tool", SCRIPT);
        dir.write("notes.md", SCRIPT);
        dir.write("LICENSE", "MIT\n");
        let paths = ["bin/tool", "notes.md", "LICENSE", "deleted"].map(PathBuf::from);

        let files = tracked_files(dir.path(), paths.to_vec()).unwrap();

        assert_eq!(
            files,
            [
                TrackedFile::new("bin/tool", true),
                TrackedFile::new("notes.md", false),
                TrackedFile::new("LICENSE", false),
                TrackedFile::new("deleted", false),
            ]
        );
        let flagged: Vec<_> = rust_only_files(&files)
            .into_iter()
            .filter_map(|finding| finding.location.map(|location| location.path))
            .collect();
        assert_eq!(flagged, [PathBuf::from("bin/tool")]);
    }

    #[test]
    fn a_lone_commit_has_no_lockfile_to_compare_and_reads_nothing() {
        let change = lockfile_change(
            Revisions::Only("HEAD"),
            merge_base_not_asked,
            lockfile_not_asked,
            working_tree_not_asked,
        );

        assert_eq!(change.unwrap(), None);
    }

    #[test]
    fn histories_without_a_merge_base_have_no_lockfile_to_compare() {
        let change = lockfile_change(
            SPAN,
            |_, _| Ok(None),
            lockfile_not_asked,
            working_tree_not_asked,
        );

        assert_eq!(change.unwrap(), None);
    }

    #[test]
    fn a_fork_point_without_a_lockfile_has_nothing_to_compare() {
        let change = lockfile_change(
            SPAN,
            |_, _| Ok(Some("forkpoint".to_owned())),
            |_| Ok(None),
            working_tree_not_asked,
        );

        assert_eq!(change.unwrap(), None);
    }

    // The fork point is built from base and head, and the base text from the fork point,
    // so the expected text proves each lookup got the previous one's answer.
    #[test]
    fn the_base_lockfile_is_read_at_the_merge_base_not_at_the_base_tip() {
        let working_tree = SourceText::new(LOCKFILE_PATH, "PP-01\nPP-02\n");

        let change = lockfile_change(
            SPAN,
            |base, head| Ok(Some(format!("fork-of-{base}-and-{head}"))),
            |commit| Ok(Some(format!("PP-01 at {commit}"))),
            || Ok(Some(working_tree.clone())),
        );

        assert_eq!(
            change.unwrap(),
            Some(LockfileChange::new(
                SourceText::new(LOCKFILE_PATH, "PP-01 at fork-of-main-and-topic"),
                Some(working_tree),
            ))
        );
    }

    #[test]
    fn a_missing_working_tree_lockfile_is_a_deleted_one() {
        let change = lockfile_change(
            SPAN,
            |_, _| Ok(Some("forkpoint".to_owned())),
            |_| Ok(Some("PP-01\n".to_owned())),
            || Ok(None),
        );

        assert_eq!(
            change.unwrap(),
            Some(LockfileChange::new(
                SourceText::new(LOCKFILE_PATH, "PP-01\n"),
                None
            ))
        );
    }
}
