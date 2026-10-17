//! The tracker checks: pure functions over a [`TrackerInput`].

use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fmt,
    num::NonZeroU32,
    path::Path,
};

use super::{
    load::{LOCKFILE_PATH, SourceText, TrackerInput},
    lockfile::{LockLine, parse_lockfile},
    model::{PainPointId, PainPointSeverity, PainPointStatus, TrackerRow},
    table::{IdTable, TableRow, TableSchema, id_tables},
};
use crate::report::{Check, Finding, Location};

/// Runs `TrackerSchema` and `TrackerIds`.
///
/// A tracker with no recognised ID table gets only the one `TrackerSchema` finding that
/// says so. Without it, every locked ID would also be reported as removed.
pub fn check(input: &TrackerInput) -> Vec<Finding> {
    let tables = id_tables(&input.tracker.text);
    let (rows, mut findings) = parse_tables(&input.tracker, &tables);
    if !tables.is_empty() {
        findings.extend(tracker_ids(&rows, input));
    }
    findings
}

/// Parses every row of every ID table; malformed rows become `TrackerSchema` findings.
///
/// A row whose ID parses is returned even when another cell is malformed, so that one
/// bad cell is not also reported as a removed ID.
pub fn parse_rows(tracker: &SourceText) -> (Vec<TrackerRow>, Vec<Finding>) {
    parse_tables(tracker, &id_tables(&tracker.text))
}

fn parse_tables(tracker: &SourceText, tables: &[IdTable]) -> (Vec<TrackerRow>, Vec<Finding>) {
    if tables.is_empty() {
        return (Vec::new(), vec![no_id_tables(tracker)]);
    }
    let mut rows = Vec::new();
    let mut findings = Vec::new();
    for table in tables {
        for raw in &table.rows {
            let location = Location::from_index(&tracker.path, raw.index);
            let (row, problems) = parse_row(&table.schema, raw, &location);
            rows.extend(row);
            findings.extend(problems.into_iter().map(|problem| {
                Finding::error(
                    Check::TrackerSchema,
                    Some(location.clone()),
                    problem.to_string(),
                )
            }));
        }
    }
    (rows, findings)
}

/// Rows whose cells do not match their table's schema, or the one finding that no ID
/// table was found.
pub fn tracker_schema(tracker: &SourceText) -> Vec<Finding> {
    parse_rows(tracker).1
}

fn no_id_tables(tracker: &SourceText) -> Finding {
    Finding::error(
        Check::TrackerSchema,
        Some(Location::new(&tracker.path, NonZeroU32::MIN)),
        "no `ID` tables found in the tracker; likely causes: a header whose first cell is not \
         exactly `ID` (such as `Id`), a header without a `|---|` separator row under it, or an \
         unclosed code fence hiding the tables",
    )
}

/// Duplicate IDs, IDs missing from the lockfile, and locked IDs removed from the tracker.
pub fn tracker_ids(rows: &[TrackerRow], input: &TrackerInput) -> Vec<Finding> {
    let (mut findings, first_lines) = duplicate_ids(rows, &input.tracker.path);
    match &input.lockfile {
        None => findings.push(Finding::error(
            Check::TrackerIds,
            None,
            format!(
                "lockfile `{LOCKFILE_PATH}` is missing; run `cargo xtask docs lock-ids` and \
                 commit it"
            ),
        )),
        Some(lockfile) => {
            let (lockfile_findings, locked) = lockfile_lines(lockfile, &first_lines);
            findings.extend(lockfile_findings);
            findings.extend(unlocked_ids(
                rows,
                &first_lines,
                &locked,
                &input.tracker.path,
            ));
        }
    }
    findings
}

/// Why a tracker row does not match its table's header.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SchemaProblem {
    CellCount { found: usize, expected: usize },
    InvalidId(String),
    InvalidSeverity(String),
    InvalidStatus(String),
}

impl fmt::Display for SchemaProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CellCount { found, expected } => {
                write!(
                    f,
                    "row has {found} cells but the table header has {expected}"
                )
            }
            Self::InvalidId(cell) => write!(
                f,
                "`{cell}` is not a pain-point ID; expected `PP-` and two characters, or \
                 `PP-R<n>` or `PP-W<n>`"
            ),
            Self::InvalidSeverity(cell) => {
                let allowed = PainPointSeverity::ALL
                    .map(PainPointSeverity::name)
                    .join(", ");
                write!(f, "severity `{cell}` must be one of {allowed}")
            }
            Self::InvalidStatus(cell) => {
                let allowed = PainPointStatus::ALL.map(PainPointStatus::name).join(", ");
                write!(
                    f,
                    "status `{cell}` must be one of {allowed}; the cell holds only the word, so \
                     put a reason in the Trade-off or Reason cell"
                )
            }
        }
    }
}

fn parse_row(
    schema: &TableSchema,
    raw: &TableRow,
    location: &Location,
) -> (Option<TrackerRow>, Vec<SchemaProblem>) {
    let mut problems = Vec::new();
    let well_shaped = raw.cells.len() == schema.columns;
    if !well_shaped {
        problems.push(SchemaProblem::CellCount {
            found: raw.cells.len(),
            expected: schema.columns,
        });
    }
    let id = raw.cells.first().and_then(|cell| {
        let id = PainPointId::parse(cell);
        if id.is_none() {
            problems.push(SchemaProblem::InvalidId(cell.clone()));
        }
        id
    });
    // Column positions mean nothing in a row of the wrong width.
    let cell_at = |column: Option<usize>| {
        column
            .filter(|_| well_shaped)
            .and_then(|column| raw.cells.get(column))
    };
    if let Some(cell) = cell_at(schema.severity_column)
        && PainPointSeverity::parse(cell).is_none()
    {
        problems.push(SchemaProblem::InvalidSeverity(cell.clone()));
    }
    let status = cell_at(schema.status_column).and_then(|cell| {
        let status = PainPointStatus::parse(cell);
        if status.is_none() {
            problems.push(SchemaProblem::InvalidStatus(cell.clone()));
        }
        status
    });
    let row = id.map(|id| TrackerRow {
        id,
        status,
        line: location.line,
    });
    (row, problems)
}

/// The line of each ID's first occurrence.
type FirstLines<'a> = BTreeMap<&'a PainPointId, NonZeroU32>;

/// Findings for repeated IDs, and the line of each ID's first occurrence.
fn duplicate_ids<'a>(
    rows: &'a [TrackerRow],
    tracker_path: &Path,
) -> (Vec<Finding>, FirstLines<'a>) {
    let mut findings = Vec::new();
    let mut first_lines = FirstLines::new();
    for row in rows {
        match first_lines.entry(&row.id) {
            Entry::Vacant(slot) => {
                slot.insert(row.line);
            }
            Entry::Occupied(first) => findings.push(ids_error(
                Location::new(tracker_path, row.line),
                format!(
                    "duplicate ID {}; first defined at line {}",
                    row.id,
                    first.get()
                ),
            )),
        }
    }
    (findings, first_lines)
}

/// Findings for invalid and repeated lockfile lines, and for locked IDs that are no
/// longer in the tracker. Also returns every valid locked ID.
fn lockfile_lines(
    lockfile: &SourceText,
    tracker_first_lines: &FirstLines<'_>,
) -> (Vec<Finding>, BTreeSet<PainPointId>) {
    let mut findings = Vec::new();
    let mut first_lines: BTreeMap<PainPointId, NonZeroU32> = BTreeMap::new();
    for line in parse_lockfile(lockfile) {
        match line {
            LockLine::Invalid { text, location } => findings.push(ids_error(
                location,
                format!("lockfile line `{text}` is not a pain-point ID; expected one ID per line"),
            )),
            LockLine::Id { id, location } => match first_lines.entry(id) {
                Entry::Occupied(first) => findings.push(ids_error(
                    location,
                    format!(
                        "duplicate ID {} in the lockfile; first listed at line {}",
                        first.key(),
                        first.get()
                    ),
                )),
                Entry::Vacant(slot) => {
                    let line = location.line;
                    if !tracker_first_lines.contains_key(slot.key()) {
                        findings.push(ids_error(
                            location,
                            format!(
                                "{} was removed from the tracker; IDs are permanent: restore \
                                 the row or mark it `won't fix`",
                                slot.key()
                            ),
                        ));
                    }
                    slot.insert(line);
                }
            },
        }
    }
    (findings, first_lines.into_keys().collect())
}

/// Tracker IDs that are not in the lockfile, reported at their first occurrence.
fn unlocked_ids(
    rows: &[TrackerRow],
    tracker_first_lines: &FirstLines<'_>,
    locked: &BTreeSet<PainPointId>,
    tracker_path: &Path,
) -> Vec<Finding> {
    rows.iter()
        .filter(|row| tracker_first_lines.get(&row.id) == Some(&row.line))
        .filter(|row| !locked.contains(&row.id))
        .map(|row| {
            ids_error(
                Location::new(tracker_path, row.line),
                format!(
                    "new ID {} is not in the lockfile; run `cargo xtask docs lock-ids` and \
                     commit the lockfile",
                    row.id
                ),
            )
        })
        .collect()
}

fn ids_error(location: Location, message: String) -> Finding {
    Finding::error(Check::TrackerIds, Some(location), message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{report::Severity, tracker::load::TRACKER_PATH};

    /// One table per schema, plus text that looks like a table but is not one: a fenced
    /// example with a bad row, and a row-like line under no `ID` header.
    const CLEAN_TRACKER: &[&str] = &[
        "# Tracker",
        "",
        "| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |",
        "|---|---|---|---|---|---|---|",
        "| PP-01 | Too many components | H | One binary | — | [doc](a.md) | designed |",
        r"| PP-5H | Escaped \| pipe and `code \| pipe` | M | `a\|b` | — | — | won't fix |",
        "",
        "```",
        "| ID | Example |",
        "|---|---|",
        "| not-an-id | x |",
        "```",
        "| PP-R1 | not a table row: no header above |",
        "| ID | Pain point | Sev | Mitigation | Caused by | Doc | Status |",
        "|---|---|---|---|---|---|---|",
        "| PP-R1 | PSI is off | L | polling | PP-53 | [doc](b.md) | open |",
        "",
        "| ID | Item | Reason |",
        "|---|---|---|",
        "| PP-W6 | Parity | Impossible |",
    ];

    fn source(lines: &[&str]) -> SourceText {
        SourceText::new(TRACKER_PATH, lines.join("\n") + "\n")
    }

    fn lockfile(ids: &[&str]) -> Option<SourceText> {
        Some(SourceText::new(LOCKFILE_PATH, ids.join("\n") + "\n"))
    }

    fn input(tracker: &[&str], lock: &[&str]) -> TrackerInput {
        TrackerInput {
            tracker: source(tracker),
            lockfile: lockfile(lock),
        }
    }

    fn error_at(check: Check, path: &str, line: u32, message: &str) -> Finding {
        let line = NonZeroU32::new(line).unwrap();
        Finding::error(check, Some(Location::new(path, line)), message)
    }

    #[test]
    fn consistent_tracker_and_lockfile_report_nothing() {
        let findings = check(&input(CLEAN_TRACKER, &["PP-01", "PP-5H", "PP-R1", "PP-W6"]));

        assert_eq!(findings, []);
    }

    #[test]
    fn duplicate_tracker_id_is_reported_at_the_second_occurrence() {
        let tracker = [
            "| ID | Item | Reason |",
            "|---|---|---|",
            "| PP-W1 | a | r |",
            "| PP-W1 | b | r |",
        ];

        let findings = check(&input(&tracker, &["PP-W1"]));

        assert_eq!(
            findings,
            [error_at(
                Check::TrackerIds,
                TRACKER_PATH,
                4,
                "duplicate ID PP-W1; first defined at line 3"
            )]
        );
    }

    #[test]
    fn locked_id_removed_from_the_tracker_is_reported_at_the_lockfile_line() {
        let tracker = [
            "| ID | Item | Reason |",
            "|---|---|---|",
            "| PP-W2 | a | r |",
        ];

        let findings = check(&input(&tracker, &["PP-W1", "PP-W2"]));

        assert_eq!(
            findings,
            [error_at(
                Check::TrackerIds,
                LOCKFILE_PATH,
                1,
                "PP-W1 was removed from the tracker; IDs are permanent: restore the row or mark it `won't fix`"
            )]
        );
    }

    #[test]
    fn new_unlocked_id_is_reported_at_the_tracker_line_with_the_lock_ids_command() {
        let tracker = [
            "| ID | Item | Reason |",
            "|---|---|---|",
            "| PP-W1 | a | r |",
            "| PP-W2 | b | r |",
        ];

        let findings = check(&input(&tracker, &["PP-W1"]));

        assert_eq!(
            findings,
            [error_at(
                Check::TrackerIds,
                TRACKER_PATH,
                4,
                "new ID PP-W2 is not in the lockfile; run `cargo xtask docs lock-ids` and commit the lockfile"
            )]
        );
    }

    #[test]
    fn missing_lockfile_is_one_error_without_a_location() {
        let tracker = [
            "| ID | Item | Reason |",
            "|---|---|---|",
            "| PP-W1 | a | r |",
        ];
        let input = TrackerInput {
            tracker: source(&tracker),
            lockfile: None,
        };

        let findings = check(&input);

        assert_eq!(
            findings,
            [Finding::error(
                Check::TrackerIds,
                None,
                "lockfile `docs/12-critique/pain-point-ids.lock` is missing; run `cargo xtask docs lock-ids` and commit it"
            )]
        );
    }

    #[test]
    fn invalid_and_repeated_lockfile_lines_are_reported_at_their_lines() {
        let tracker = [
            "| ID | Item | Reason |",
            "|---|---|---|",
            "| PP-W1 | a | r |",
        ];

        let findings = check(&input(&tracker, &["PP-W1", "", "pp-w1", "PP-W1"]));

        assert_eq!(
            findings,
            [
                error_at(
                    Check::TrackerIds,
                    LOCKFILE_PATH,
                    2,
                    "lockfile line `` is not a pain-point ID; expected one ID per line"
                ),
                error_at(
                    Check::TrackerIds,
                    LOCKFILE_PATH,
                    3,
                    "lockfile line `pp-w1` is not a pain-point ID; expected one ID per line"
                ),
                error_at(
                    Check::TrackerIds,
                    LOCKFILE_PATH,
                    4,
                    "duplicate ID PP-W1 in the lockfile; first listed at line 1"
                ),
            ]
        );
    }

    #[test]
    fn malformed_rows_are_schema_errors_at_the_row_line() {
        let tracker = [
            "| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |",
            "|---|---|---|---|---|---|---|",
            "| PP-01 | too few cells | H | a | b | designed |",
            "| PP-1 | bad id | H | a | b | c | designed |",
            "| PP-02 | bad sev | X | a | b | c | designed |",
            "| PP-03 | bad status | H | a | b | c | won't fix (obsolete) |",
            "| PP-04 | fine | L | a | b | c | open |",
        ];

        // Every well-formed ID is locked, so only the schema errors remain: a row with
        // a bad cell must not also make its ID look removed.
        let findings = check(&input(&tracker, &["PP-01", "PP-02", "PP-03", "PP-04"]));

        let reported: Vec<_> = findings
            .iter()
            .map(|finding| {
                let line = finding
                    .location
                    .as_ref()
                    .map(|location| location.line.get());
                (
                    finding.check,
                    finding.severity,
                    line,
                    finding.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            reported,
            [
                (
                    Check::TrackerSchema,
                    Severity::Error,
                    Some(3),
                    "row has 6 cells but the table header has 7"
                ),
                (
                    Check::TrackerSchema,
                    Severity::Error,
                    Some(4),
                    "`PP-1` is not a pain-point ID; expected `PP-` and two characters, or `PP-R<n>` or `PP-W<n>`"
                ),
                (
                    Check::TrackerSchema,
                    Severity::Error,
                    Some(5),
                    "severity `X` must be one of H, M, L"
                ),
                (
                    Check::TrackerSchema,
                    Severity::Error,
                    Some(6),
                    "status `won't fix (obsolete)` must be one of designed, building, done, deferred, won't fix, open; the cell holds only the word, so put a reason in the Trade-off or Reason cell"
                ),
            ]
        );
    }

    #[test]
    fn a_tracker_without_id_tables_is_one_error_not_a_flood_of_removed_ids() {
        let misspelled_header = [
            "| Id | Item | Reason |",
            "|---|---|---|",
            "| PP-W1 | a | r |",
        ];
        let unclosed_fence = [
            "```",
            "| ID | Item | Reason |",
            "|---|---|---|",
            "| PP-W1 | a | r |",
        ];
        let locked = ["PP-W1", "PP-W2", "PP-W3"];

        let misspelled = check(&input(&misspelled_header, &locked));
        let unclosed = check(&input(&unclosed_fence, &locked));

        let expected = [error_at(
            Check::TrackerSchema,
            TRACKER_PATH,
            1,
            "no `ID` tables found in the tracker; likely causes: a header whose first cell is not exactly `ID` (such as `Id`), a header without a `|---|` separator row under it, or an unclosed code fence hiding the tables",
        )];
        assert_eq!(misspelled, expected);
        assert_eq!(unclosed, expected);
    }
}
