//! Reading and parsing the design library.

use std::path::Path;

use super::model::{Doc, DocSet};
use crate::error::XtaskError;

/// Reads and parses every `docs/**/*.md` under `repo_root`, and records which
/// non-markdown targets of relative links exist on disk.
pub fn load(repo_root: &Path) -> Result<DocSet, XtaskError> {
    todo!()
}

/// Parses one markdown document. Fenced code blocks and inline code spans are not
/// scanned for headings or links.
pub fn parse_doc(text: &str) -> Doc {
    todo!()
}
