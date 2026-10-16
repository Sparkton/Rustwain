//! `--range <base>..<head>`.

use std::str::FromStr;

use crate::error::XtaskError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRange {
    pub base: String,
    pub head: String,
}

impl FromStr for CommitRange {
    type Err = XtaskError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        todo!()
    }
}
