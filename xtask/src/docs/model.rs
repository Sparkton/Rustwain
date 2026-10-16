//! The parsed design library.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
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
    /// The word of inline code spans shaped like a marker (`` `[word]` ``) whose word is
    /// not a known marker but is a near miss of one, such as `verfiy`. Other words are
    /// not recorded.
    pub unknown_markers: Vec<(String, NonZeroU32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// The heading as written, without the leading and closing `#`s.
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
    /// A path relative to the linking doc's directory (or to the repository root when it
    /// starts with `/`), with an optional non-empty `#anchor`.
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
    pub const ALL: [Self; 6] = [
        Self::Proposed,
        Self::Decided,
        Self::Open,
        Self::Later,
        Self::Skip,
        Self::Verify,
    ];

    /// The marker for a bracketed word, such as `verify` for `[verify]`.
    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|marker| marker.word() == word)
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Decided => "decided",
            Self::Open => "open",
            Self::Later => "later",
            Self::Skip => "skip",
            Self::Verify => "verify",
        }
    }
}

/// A forward-slash path relative to the repository root, such as `docs/README.md`.
/// A trailing `/` marks a directory, so it never equals the key of a doc.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(String);

impl RelPath {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The repository-relative path that a link written in this doc points at, with `.`
    /// and `..` segments resolved. `None` when the link climbs above the repository root.
    pub fn resolve_link(&self, target: &str) -> Option<Self> {
        let (base, target) = match target.strip_prefix('/') {
            Some(from_root) => ("", from_root),
            None => (self.directory(), target),
        };
        let mut segments: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
        for segment in target.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    segments.pop()?;
                }
                named => segments.push(named),
            }
        }
        let mut resolved = segments.join("/");
        if target.ends_with('/') && !resolved.is_empty() {
            resolved.push('/');
        }
        Some(Self(resolved))
    }

    fn directory(&self) -> &str {
        self.0
            .rsplit_once('/')
            .map_or("", |(directory, _)| directory)
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A `Relative` link with its target resolved, as the checks and the loader need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelativeLink<'a> {
    pub doc: &'a RelPath,
    pub line: NonZeroU32,
    pub path: &'a str,
    pub anchor: Option<&'a str>,
    /// `None` when the link climbs above the repository root.
    pub resolved: Option<RelPath>,
}

impl RelativeLink<'_> {
    /// The target as written in the doc, anchor included.
    pub fn written(&self) -> String {
        match self.anchor {
            Some(anchor) => format!("{}#{anchor}", self.path),
            None => self.path.to_owned(),
        }
    }
}

impl DocSet {
    /// Every relative link of every doc, in path then line order.
    pub fn relative_links(&self) -> impl Iterator<Item = RelativeLink<'_>> {
        self.docs.iter().flat_map(|(doc, parsed)| {
            parsed
                .links
                .iter()
                .filter_map(move |link| match &link.target {
                    LinkTarget::Relative { path, anchor } => Some(RelativeLink {
                        doc,
                        line: link.line,
                        path,
                        anchor: anchor.as_deref(),
                        resolved: doc.resolve_link(path),
                    }),
                    LinkTarget::External(_) | LinkTarget::SameDoc { .. } => None,
                })
        })
    }

    /// Whether a resolved link target is a doc or an existing non-doc path.
    pub fn target_exists(&self, resolved: &RelPath) -> bool {
        self.docs.contains_key(resolved) || self.existing_targets.contains(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(doc: &str, target: &str) -> Option<String> {
        RelPath::new(doc)
            .resolve_link(target)
            .map(|resolved| resolved.as_str().to_owned())
    }

    #[test]
    fn links_resolve_against_the_linking_docs_directory() {
        let doc = "docs/05-networking/01-network-model.md";
        assert_eq!(
            resolve(doc, "02-pod-networking.md"),
            Some("docs/05-networking/02-pod-networking.md".to_owned())
        );
        assert_eq!(
            resolve(doc, "./../13-implementation/./03-roadmap.md"),
            Some("docs/13-implementation/03-roadmap.md".to_owned())
        );
        assert_eq!(
            resolve(doc, "../../README.md"),
            Some("README.md".to_owned())
        );
        assert_eq!(
            resolve(doc, "/docs/README.md"),
            Some("docs/README.md".to_owned())
        );
    }

    #[test]
    fn trailing_slash_keeps_a_directory_distinct_from_a_file() {
        assert_eq!(
            resolve("docs/README.md", "05-networking/"),
            Some("docs/05-networking/".to_owned())
        );
    }

    #[test]
    fn links_above_the_repository_root_do_not_resolve() {
        assert_eq!(resolve("docs/README.md", "../../outside.md"), None);
    }
}
