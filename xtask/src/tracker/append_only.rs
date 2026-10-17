//! The lockfile is append-only across commits: a PR may add IDs but never drop or reorder
//! the ones its base already locked. The comparison is pure; the policy loader reads the
//! base lockfile from `git`.

use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    path::Path,
};

use super::{
    load::{LOCKFILE_PATH, SourceText},
    lockfile::{LockLine, parse_lockfile},
    model::PainPointId,
};
use crate::report::{Check, Finding, Location};

/// The lockfile at the base commit and now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileChange {
    pub base: SourceText,
    /// `None` when the lockfile no longer exists.
    pub current: Option<SourceText>,
}

impl LockfileChange {
    pub fn new(base: SourceText, current: Option<SourceText>) -> Self {
        Self { base, current }
    }
}

/// `TrackerIds` errors for every base ID missing from the current lockfile and for
/// every base ID listed before one that came earlier in the base.
///
/// New IDs may appear anywhere, including between old ones, so a merge that interleaves
/// two PRs' appends passes. A removed ID has no line left to point at, so it is reported
/// at line 1.
pub fn append_only(change: &LockfileChange) -> Vec<Finding> {
    let base_ids = distinct_ids(&change.base);
    let Some(current) = &change.current else {
        return deleted_lockfile(&change.base.path, base_ids.len());
    };
    let current_lines: BTreeMap<PainPointId, Location> =
        distinct_ids(current).into_iter().collect();
    let removed = base_ids
        .iter()
        .filter(|(id, _)| !current_lines.contains_key(id))
        .map(|(id, _)| {
            error(
                Location::new(&current.path, NonZeroU32::MIN),
                format!(
                    "{id} was removed from the lockfile; it is append-only, so restore the line"
                ),
            )
        });
    // Base IDs that survive, in base order, with their current lines.
    let survivors: Vec<(&PainPointId, &Location)> = base_ids
        .iter()
        .filter_map(|(id, _)| current_lines.get_key_value(id))
        .collect();
    let reordered = survivors
        .iter()
        .zip(survivors.iter().skip(1))
        .filter(|((_, earlier_at), (_, later_at))| later_at.line < earlier_at.line)
        .map(|((earlier, _), (later, later_at))| {
            error(
                (*later_at).clone(),
                format!(
                    "{later} is listed before {earlier}, which came first in the base lockfile; \
                     keep the existing order and append new IDs at the end"
                ),
            )
        });
    removed.chain(reordered).collect()
}

/// Each valid ID once, at its first line, in file order.
fn distinct_ids(lockfile: &SourceText) -> Vec<(PainPointId, Location)> {
    let mut seen = BTreeSet::new();
    parse_lockfile(lockfile)
        .into_iter()
        .filter_map(|line| match line {
            LockLine::Id { id, location } => Some((id, location)),
            LockLine::Invalid { .. } => None,
        })
        .filter(|(id, _)| seen.insert(id.clone()))
        .collect()
}

fn deleted_lockfile(path: &Path, base_count: usize) -> Vec<Finding> {
    if base_count == 0 {
        return Vec::new();
    }
    vec![error(
        Location::new(path, NonZeroU32::MIN),
        format!(
            "the lockfile `{LOCKFILE_PATH}` was deleted; it is append-only and the base locked \
             {base_count} IDs"
        ),
    )]
}

fn error(location: Location, message: String) -> Finding {
    Finding::error(Check::TrackerIds, Some(location), message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lockfile(ids: &[&str]) -> SourceText {
        SourceText::new(
            LOCKFILE_PATH,
            ids.iter().map(|id| format!("{id}\n")).collect::<String>(),
        )
    }

    fn change(base: &[&str], current: &[&str]) -> LockfileChange {
        LockfileChange::new(lockfile(base), Some(lockfile(current)))
    }

    fn at_lockfile(line: u32, message: &str) -> Finding {
        let location = Location::new(LOCKFILE_PATH, NonZeroU32::new(line).unwrap());
        Finding::error(Check::TrackerIds, Some(location), message)
    }

    #[test]
    fn appended_ids_at_the_end_or_between_old_ones_are_allowed() {
        let appended = change(&["PP-01", "PP-02"], &["PP-01", "PP-02", "PP-03"]);
        let interleaved = change(&["PP-01", "PP-02"], &["PP-01", "PP-0A", "PP-02", "PP-03"]);

        assert_eq!(append_only(&appended), []);
        assert_eq!(append_only(&interleaved), []);
    }

    #[test]
    fn removed_ids_are_reported_at_line_one_and_reordered_ones_at_their_new_line() {
        let change = change(
            &["PP-01", "PP-02", "PP-03", "PP-04"],
            &["PP-01", "PP-04", "PP-03", "PP-05"],
        );

        let findings = append_only(&change);

        assert_eq!(
            findings,
            [
                at_lockfile(
                    1,
                    "PP-02 was removed from the lockfile; it is append-only, so restore the line"
                ),
                at_lockfile(
                    2,
                    "PP-04 is listed before PP-03, which came first in the base lockfile; keep the existing order and append new IDs at the end"
                ),
            ]
        );
    }

    #[test]
    fn a_deleted_lockfile_is_one_error() {
        let change = LockfileChange::new(lockfile(&["PP-01", "PP-02"]), None);

        let findings = append_only(&change);

        assert_eq!(
            findings,
            [at_lockfile(
                1,
                "the lockfile `docs/12-critique/pain-point-ids.lock` was deleted; it is append-only and the base locked 2 IDs"
            )]
        );
    }
}
