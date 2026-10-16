//! The tracker checks: pure functions over a [`TrackerInput`].

use super::{
    load::{SourceText, TrackerInput},
    model::TrackerRow,
};
use crate::report::Finding;

/// Runs `TrackerSchema` and `TrackerIds`.
pub fn check(input: &TrackerInput) -> Vec<Finding> {
    todo!()
}

/// Parses every row of every ID table; malformed rows become `TrackerSchema` findings.
pub fn parse_rows(tracker: &SourceText) -> (Vec<TrackerRow>, Vec<Finding>) {
    todo!()
}

/// Rows whose cells do not match their table's schema.
pub fn tracker_schema(tracker: &SourceText) -> Vec<Finding> {
    todo!()
}

/// Duplicate IDs, IDs missing from the lockfile, and locked IDs removed from the tracker.
pub fn tracker_ids(rows: &[TrackerRow], input: &TrackerInput) -> Vec<Finding> {
    todo!()
}
