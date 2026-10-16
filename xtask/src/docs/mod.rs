//! Design-library integrity: relative links, anchors, index reachability and decision
//! markers over every `docs/**/*.md`.
//!
//! [`load`] is the only code here that touches the filesystem. It parses each doc once
//! into a [`DocSet`]; the checks are pure functions over it.

mod checks;
mod load;
mod marker_report;
mod model;
mod slug;

pub use checks::{check, doc_anchors, doc_index, doc_links, markers};
pub use load::{load, parse_doc};
pub use marker_report::marker_report;
pub use model::{Doc, DocSet, Heading, Link, LinkTarget, Marker, RelPath};
pub use slug::github_slug;
