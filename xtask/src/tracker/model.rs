//! Pain-point IDs, statuses and tracker rows.

use std::{fmt, num::NonZeroU32};

const ID_PREFIX: &str = "PP-";

/// `PP-[0-9A-F][0-9A-Z]`, `PP-R<n>` or `PP-W<n>`; valid by construction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PainPointId(String);

impl PainPointId {
    /// Accepts `PP-` followed by two characters (`[0-9A-F]` then `[0-9A-Z]`), or by `R`
    /// or `W` and a decimal number of at least 1 without a leading zero.
    pub fn parse(text: &str) -> Option<Self> {
        let suffix = text.strip_prefix(ID_PREFIX)?;
        let valid = match suffix.as_bytes() {
            [b'0'..=b'9' | b'A'..=b'F', second] => {
                second.is_ascii_digit() || second.is_ascii_uppercase()
            }
            [b'R' | b'W', number @ ..] => is_positive_decimal(number),
            _ => false,
        };
        valid.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PainPointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_positive_decimal(digits: &[u8]) -> bool {
    matches!(digits, [b'1'..=b'9', rest @ ..] if rest.iter().all(u8::is_ascii_digit))
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
    pub const ALL: [Self; 6] = [
        Self::Designed,
        Self::Building,
        Self::Done,
        Self::Deferred,
        Self::WontFix,
        Self::Open,
    ];

    /// The status as the tracker's legend writes it, such as `won't fix`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Designed => "designed",
            Self::Building => "building",
            Self::Done => "done",
            Self::Deferred => "deferred",
            Self::WontFix => "won't fix",
            Self::Open => "open",
        }
    }

    /// The status for a Status cell, such as `won't fix`.
    pub fn parse(cell: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.name() == cell)
    }
}

/// How much a pain point hurts Kubernetes users, in the Sev column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PainPointSeverity {
    High,
    Medium,
    Low,
}

impl PainPointSeverity {
    pub const ALL: [Self; 3] = [Self::High, Self::Medium, Self::Low];

    pub fn name(self) -> &'static str {
        match self {
            Self::High => "H",
            Self::Medium => "M",
            Self::Low => "L",
        }
    }

    pub fn parse(cell: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|severity| severity.name() == cell)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerRow {
    pub id: PainPointId,
    /// `None` for tables without a Status column (the won't-fix table), and for a row
    /// whose Status cell is invalid (reported as a `TrackerSchema` error).
    pub status: Option<PainPointStatus>,
    pub line: NonZeroU32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pain_point_id_accepts_the_three_documented_shapes() {
        let valid = [
            "PP-01", "PP-5H", "PP-6C", "PP-96", "PP-A0", "PP-FZ", "PP-R1", "PP-R10", "PP-W6",
        ];

        let rejected: Vec<_> = valid
            .into_iter()
            .filter(|text| PainPointId::parse(text).is_none())
            .collect();

        assert_eq!(rejected, Vec::<&str>::new());
    }

    #[test]
    fn pain_point_id_rejects_everything_else() {
        let invalid = [
            "", "PP-", "PP-1", "PP-123", "pp-01", "PP-G0", "PP-0a", "PP-R", "PP-R0", "PP-R01",
            "PP-W", "PP-Wx", "PP-X1", "PP-R1 ", " PP-01", "PP-0é",
        ];

        let accepted: Vec<_> = invalid
            .into_iter()
            .filter(|text| PainPointId::parse(text).is_some())
            .collect();

        assert_eq!(accepted, Vec::<&str>::new());
    }
}
