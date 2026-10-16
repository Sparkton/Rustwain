//! The append-only pain-point ID lockfile.

use std::path::Path;

use super::model::PainPointId;
use crate::error::XtaskError;

/// Appends tracker IDs that are missing from the lockfile, in tracker order, creating
/// the file if needed. Existing lines are never rewritten or reordered. Returns the
/// appended IDs.
pub fn lock_ids(repo_root: &Path) -> Result<Vec<PainPointId>, XtaskError> {
    todo!()
}
