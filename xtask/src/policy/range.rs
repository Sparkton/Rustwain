//! `--range <base>..<head>`.

use std::str::FromStr;

use crate::error::XtaskError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRange {
    pub base: String,
    pub head: String,
}

impl CommitRange {
    pub fn new(base: impl Into<String>, head: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            head: head.into(),
        }
    }

    /// CI passes an all-zero base on the first push of a branch, when there is no
    /// previous tip to start from.
    pub fn is_first_push(&self) -> bool {
        !self.base.is_empty() && self.base.chars().all(|c| c == '0')
    }
}

impl FromStr for CommitRange {
    type Err = XtaskError;

    /// Both sides must be non-empty and free of whitespace. They are not resolved here;
    /// `git` rejects a revision that does not exist.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let is_revision = |side: &str| !side.is_empty() && !side.contains(char::is_whitespace);
        match text.split_once("..") {
            Some((base, head)) if is_revision(base) && is_revision(head) => {
                Ok(Self::new(base, head))
            }
            _ => Err(XtaskError::InvalidRange {
                input: text.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_splits_into_base_and_head() {
        let range = "origin/main..HEAD".parse::<CommitRange>().unwrap();

        assert_eq!(range, CommitRange::new("origin/main", "HEAD"));
    }

    #[test]
    fn range_rejects_missing_sides_separators_and_whitespace() {
        let invalid = [
            "", "main", "..HEAD", "main..", "..", "a b..c", "a..b c", " .. ",
        ];

        let accepted: Vec<_> = invalid
            .into_iter()
            .filter(|text| text.parse::<CommitRange>().is_ok())
            .collect();

        assert_eq!(accepted, Vec::<&str>::new());
        assert!(matches!(
            "main".parse::<CommitRange>(),
            Err(XtaskError::InvalidRange { input }) if input == "main"
        ));
    }
}
