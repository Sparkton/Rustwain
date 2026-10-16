//! The parsed design library.

use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
};

/// Every markdown file under `docs/`, parsed once by [`super::load`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocSet {
    pub docs: BTreeMap<RelPath, Doc>,
    /// Non-markdown paths that relative links point at and that exist on disk (files or
    /// directories), so [`super::doc_links`] stays pure.
    pub existing_targets: BTreeSet<RelPath>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doc {
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    pub markers: Vec<(Marker, NonZeroU32)>,
    /// Inline code spans shaped like a marker (`` `[word]` ``) whose word is not one of
    /// the known markers.
    pub unknown_markers: Vec<(String, NonZeroU32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub text: String,
    /// GitHub's anchor for the heading, with `-1`, `-2`, … appended to duplicates.
    pub anchor: String,
    pub line: NonZeroU32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub target: LinkTarget,
    pub line: NonZeroU32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A URL with a scheme (`https:`, `mailto:`, …). Not checked: external link
    /// checking is a non-goal of design 0001.
    External(String),
    /// `#anchor` within the same doc.
    SameDoc { anchor: String },
    /// A path relative to the linking doc's directory, with an optional `#anchor`.
    Relative {
        path: String,
        anchor: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Marker {
    Proposed,
    Decided,
    Open,
    Later,
    Skip,
    Verify,
}

impl Marker {
    /// The marker for a bracketed word, such as `verify` for `[verify]`.
    pub fn from_word(word: &str) -> Option<Self> {
        todo!()
    }

    pub fn word(self) -> &'static str {
        todo!()
    }
}

/// A forward-slash path relative to the repository root, such as `docs/README.md`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(String);

impl RelPath {
    pub fn new(path: impl Into<String>) -> Self {
        todo!()
    }

    pub fn as_str(&self) -> &str {
        todo!()
    }
}
