//! Pain-point tracker integrity: row schema, and ID stability against an append-only
//! lockfile (`docs/12-critique/pain-point-ids.lock`).
//!
//! [`load`] reads the raw files; parsing and checking are pure, in [`check`].
//! [`lock_ids`] is the one writer, and it only appends.

mod checks;
mod load;
mod lockfile;
mod model;

pub use checks::{check, parse_rows, tracker_ids, tracker_schema};
pub use load::{SourceText, TrackerInput, load};
pub use lockfile::lock_ids;
pub use model::{PainPointId, PainPointStatus, TrackerRow};
