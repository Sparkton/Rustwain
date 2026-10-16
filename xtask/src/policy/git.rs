//! The only place policy runs `git`, as a subprocess (design 0001, section 5.2). Every
//! call goes through `git -C <repo root>`.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use super::{CommitMessage, CommitRange};
use crate::error::XtaskError;

const GIT: &str = "git";
/// Commit hash, NUL, raw message, then the record separator `\x1e`.
const LOG_FORMAT: &str = "--format=%H%x00%B%x1e";
const RECORD_SEPARATOR: char = '\x1e';
const FIELD_SEPARATOR: char = '\0';
/// Stops a revision that starts with `-` from being read as an option.
const END_OF_OPTIONS: &str = "--end-of-options";

pub(super) const DEFAULT_BASE: &str = "origin/main";
const DEFAULT_HEAD: &str = "HEAD";

/// The commits whose messages are checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Revisions<'a> {
    /// Commits reachable from `head` but not from `base`.
    Span { base: &'a str, head: &'a str },
    /// A single commit, for when there is no base to compare with.
    Only(&'a str),
}

impl<'a> Revisions<'a> {
    /// Without a range: `origin/main..HEAD` if `origin/main` exists, else `HEAD` alone. A
    /// first push has no base, so only its head commit is inspected.
    pub(super) fn select(
        range: Option<&'a CommitRange>,
        origin_main_exists: impl FnOnce() -> Result<bool, XtaskError>,
    ) -> Result<Self, XtaskError> {
        Ok(match range {
            Some(range) if range.is_first_push() => Self::Only(&range.head),
            Some(range) => Self::Span {
                base: &range.base,
                head: &range.head,
            },
            None if origin_main_exists()? => Self::Span {
                base: DEFAULT_BASE,
                head: DEFAULT_HEAD,
            },
            None => Self::Only(DEFAULT_HEAD),
        })
    }
}

pub(super) fn tracked_paths(repo_root: &Path) -> Result<Vec<PathBuf>, XtaskError> {
    let output = run(repo_root, &["ls-files", "-z"])?;
    Ok(output
        .split(FIELD_SEPARATOR)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect())
}

pub(super) fn commit_messages(
    repo_root: &Path,
    revisions: Revisions<'_>,
) -> Result<Vec<CommitMessage>, XtaskError> {
    let output = match revisions {
        Revisions::Span { base, head } => {
            let span = format!("{base}..{head}");
            run(repo_root, &["log", LOG_FORMAT, END_OF_OPTIONS, &span])?
        }
        Revisions::Only(head) => run(repo_root, &["log", "-1", LOG_FORMAT, END_OF_OPTIONS, head])?,
    };
    Ok(parse_log(&output))
}

/// Whether `git` can resolve `name` to an object.
pub(super) fn ref_exists(repo_root: &Path, name: &str) -> Result<bool, XtaskError> {
    let status = Command::new(GIT)
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "--verify", "--quiet", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(spawn_error)?;
    Ok(status.success())
}

/// `None` on a detached HEAD.
pub(super) fn current_branch(repo_root: &Path) -> Result<Option<String>, XtaskError> {
    let output = run(repo_root, &["branch", "--show-current"])?;
    let name = output.trim();
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

fn parse_log(output: &str) -> Vec<CommitMessage> {
    output
        .split(RECORD_SEPARATOR)
        .filter_map(|record| {
            // `git log` ends each entry with a newline, which lands before the next hash.
            let (sha, message) = record
                .trim_start_matches('\n')
                .split_once(FIELD_SEPARATOR)?;
            Some(CommitMessage::new(sha, message))
        })
        .collect()
}

fn run(repo_root: &Path, args: &[&str]) -> Result<String, XtaskError> {
    let command = format!("{GIT} {}", args.join(" "));
    let output = Command::new(GIT)
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(spawn_error)?;
    if !output.status.success() {
        return Err(XtaskError::CommandFailed {
            command,
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    String::from_utf8(output.stdout).map_err(|_| XtaskError::NonUtf8Output { command })
}

fn spawn_error(source: std::io::Error) -> XtaskError {
    XtaskError::Spawn {
        program: GIT.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_output_splits_into_commits_with_multiline_messages() {
        let output = "aaa\0Subject\n\nBody line\n\x1e\nbbb\0Other\n\x1e\n";

        let commits = parse_log(output);

        assert_eq!(
            commits,
            [
                CommitMessage::new("aaa", "Subject\n\nBody line\n"),
                CommitMessage::new("bbb", "Other\n"),
            ]
        );
    }

    #[test]
    fn revisions_follow_the_range_or_fall_back_to_what_exists() {
        let zeros = "0".repeat(40);
        let range = CommitRange::new("main", "topic");
        let first_push = CommitRange::new(zeros, "topic");
        let exists = || Ok(true);
        let absent = || Ok(false);

        let selected = [
            Revisions::select(Some(&range), exists),
            Revisions::select(Some(&first_push), exists),
            Revisions::select(None, exists),
            Revisions::select(None, absent),
        ]
        .map(Result::unwrap);

        assert_eq!(
            selected,
            [
                Revisions::Span {
                    base: "main",
                    head: "topic"
                },
                Revisions::Only("topic"),
                Revisions::Span {
                    base: "origin/main",
                    head: "HEAD"
                },
                Revisions::Only("HEAD"),
            ]
        );
    }
}
