//! The docs checks: pure functions over a [`DocSet`].

use super::model::DocSet;
use crate::report::Finding;

/// Runs `DocLinks`, `DocAnchors`, `DocIndex` and `Markers`.
pub fn check(set: &DocSet) -> Vec<Finding> {
    todo!()
}

/// Relative links whose target is neither a doc in the set nor an existing path.
pub fn doc_links(set: &DocSet) -> Vec<Finding> {
    todo!()
}

/// Anchors with no matching heading in the doc they point into.
pub fn doc_anchors(set: &DocSet) -> Vec<Finding> {
    todo!()
}

/// Docs that cannot be reached from `docs/README.md` by following relative links.
pub fn doc_index(set: &DocSet) -> Vec<Finding> {
    todo!()
}

/// Marker-shaped inline code spans whose word is not a known marker.
pub fn markers(set: &DocSet) -> Vec<Finding> {
    todo!()
}
