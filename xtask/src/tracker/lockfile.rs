//! The append-only pain-point ID lockfile: one ID per line, each ending in `\n`, with no
//! comments and no blank lines.

use std::{collections::BTreeSet, fs::OpenOptions, io::Write, path::Path};

use super::{
    checks::parse_rows,
    load::{LOCKFILE_PATH, SourceText, load},
    model::PainPointId,
};
use crate::{error::XtaskError, report::Location};

/// One line of the lockfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LockLine {
    Id { id: PainPointId, location: Location },
    Invalid { text: String, location: Location },
}

pub(super) fn parse_lockfile(lockfile: &SourceText) -> Vec<LockLine> {
    lockfile
        .text
        .lines()
        .enumerate()
        .map(|(index, text)| {
            let location = Location::from_index(&lockfile.path, index);
            match PainPointId::parse(text) {
                Some(id) => LockLine::Id { id, location },
                None => LockLine::Invalid {
                    text: text.to_owned(),
                    location,
                },
            }
        })
        .collect()
}

/// Appends tracker IDs that are missing from the lockfile, in tracker order, creating
/// the file if needed. Existing lines are never rewritten or reordered. Returns the
/// appended IDs.
///
/// Unparseable lockfile lines are skipped here; the `TrackerIds` check reports them.
/// A last line without its `\n` is terminated before appending, so IDs never run
/// together.
pub fn lock_ids(repo_root: &Path) -> Result<Vec<PainPointId>, XtaskError> {
    let input = load(repo_root)?;
    let (rows, _schema_findings) = parse_rows(&input.tracker);
    let mut known: BTreeSet<PainPointId> = input
        .lockfile
        .iter()
        .flat_map(parse_lockfile)
        .filter_map(|line| match line {
            LockLine::Id { id, .. } => Some(id),
            LockLine::Invalid { .. } => None,
        })
        .collect();
    // `insert` is false for a locked ID and for a repeat of an ID already appended.
    let missing: Vec<PainPointId> = rows
        .into_iter()
        .map(|row| row.id)
        .filter(|id| known.insert(id.clone()))
        .collect();
    if !missing.is_empty() {
        let unterminated = input
            .lockfile
            .as_ref()
            .is_some_and(|lockfile| !lockfile.text.is_empty() && !lockfile.text.ends_with('\n'));
        append(repo_root, unterminated, &missing)?;
    }
    Ok(missing)
}

fn append(repo_root: &Path, unterminated: bool, ids: &[PainPointId]) -> Result<(), XtaskError> {
    let path = repo_root.join(LOCKFILE_PATH);
    let separator = if unterminated { "\n" } else { "" };
    let lines: String = ids.iter().map(|id| format!("{id}\n")).collect();
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(format!("{separator}{lines}").as_bytes()))
        .map_err(|source| XtaskError::Io { path, source })
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf, process};

    use super::*;
    use crate::tracker::load::TRACKER_PATH;

    const TRACKER: &str = "\
| ID | Item | Reason |
|---|---|---|
| PP-01 | one | r |
| PP-02 | two | r |
| PP-03 | three | r |
| PP-03 | three again | r |
";

    /// A scratch repository root that removes itself when dropped.
    struct TempRepo {
        root: PathBuf,
    }

    impl TempRepo {
        fn new(test_name: &str) -> Self {
            let root = env::temp_dir().join(format!("xtask-{}-{test_name}", process::id()));
            let _ = fs::remove_dir_all(&root);
            let repo = Self { root };
            repo.write(TRACKER_PATH, TRACKER);
            repo
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn lockfile(&self) -> Option<String> {
            fs::read_to_string(self.root.join(LOCKFILE_PATH)).ok()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn ids(appended: &[PainPointId]) -> Vec<&str> {
        appended.iter().map(PainPointId::as_str).collect()
    }

    #[test]
    fn lock_ids_creates_the_lockfile_with_every_tracker_id_once() {
        let repo = TempRepo::new("lock_ids_creates");

        let appended = lock_ids(&repo.root).unwrap();

        assert_eq!(ids(&appended), ["PP-01", "PP-02", "PP-03"]);
        assert_eq!(repo.lockfile().as_deref(), Some("PP-01\nPP-02\nPP-03\n"));
    }

    #[test]
    fn lock_ids_only_appends_and_a_second_run_changes_nothing() {
        let repo = TempRepo::new("lock_ids_appends");
        // Out of order, with a junk line and no final newline: all of it must survive.
        repo.write(LOCKFILE_PATH, "PP-03\njunk\nPP-02");

        let first = lock_ids(&repo.root).unwrap();
        let after_first = repo.lockfile();
        let second = lock_ids(&repo.root).unwrap();

        assert_eq!(ids(&first), ["PP-01"]);
        assert_eq!(after_first.as_deref(), Some("PP-03\njunk\nPP-02\nPP-01\n"));
        assert_eq!(ids(&second), Vec::<&str>::new());
        assert_eq!(repo.lockfile(), after_first);
    }
}
