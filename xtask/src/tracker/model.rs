//! Pain-point IDs, statuses and tracker rows.

use std::num::NonZeroU32;

/// `PP-[0-9A-F][0-9A-Z]`, `PP-R<n>` or `PP-W<n>`; valid by construction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PainPointId(String);

impl PainPointId {
    pub fn parse(text: &str) -> Option<Self> {
        todo!()
    }

    pub fn as_str(&self) -> &str {
        todo!()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PainPointStatus {
    Designed,
    Building,
    Done,
    Deferred,
    WontFix,
    Open,
}

impl PainPointStatus {
    /// The status for a Status cell, such as `won't fix`.
    pub fn parse(cell: &str) -> Option<Self> {
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerRow {
    pub id: PainPointId,
    /// `None` for tables without a Status column (the won't-fix table).
    pub status: Option<PainPointStatus>,
    pub line: NonZeroU32,
}
