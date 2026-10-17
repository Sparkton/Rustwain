//! Pain-point tracker integrity: row schema, and ID stability against an append-only
//! lockfile (`docs/12-critique/pain-point-ids.lock`).
//!
//! [`load`] reads the raw files; parsing and checking are pure, in [`check`].
//! [`lock_ids`] is the one writer, and it only appends. [`append_only`] compares the
//! lockfile with its base-commit version, which the policy loader reads from `git`.

mod append_only;
mod checks;
mod load;
mod lockfile;
mod model;
mod table;
#[cfg(test)]
pub(crate) mod temp_dir;

pub use append_only::{LockfileChange, append_only};
pub use checks::{check, parse_rows, tracker_ids, tracker_schema};
pub use load::{LOCKFILE_PATH, SourceText, TRACKER_PATH, TrackerInput, load, load_lockfile};
pub use lockfile::lock_ids;
pub use model::{PainPointId, PainPointSeverity, PainPointStatus, TrackerRow};
