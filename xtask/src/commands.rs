//! Command dispatch: composes loaders, checks and output for each `cargo xtask`
//! command, applies `--skip`, and runs the `ci` aggregate.
//!
//! [`run`] is the entry point. `dispatch` holds the commands themselves and `ci` the
//! aggregate that runs the blocking checks in sequence.

mod ci;
mod dispatch;

pub use dispatch::run;
