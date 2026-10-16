//! Repository automation for Rustwain, run as `cargo xtask <command>`.
//!
//! The contract is `docs/design/0001-ci-pipeline.md`. Loaders own all file and subprocess
//! I/O and run once per command; every check is a pure function over what they return,
//! so checks are unit-testable without a filesystem. Checks never print or exit: they
//! return [`report::Finding`]s, and `commands` and `output` turn those into terminal
//! text, GitHub annotations, the job summary and the exit status.

pub mod cli;
pub mod commands;
pub mod docs;
pub mod error;
pub mod output;
pub mod policy;
pub mod report;
pub mod tracker;
