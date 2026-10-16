//! Command-line interface (design 0001, section 5.3). T2 adds `host-facts` and `bench`.

use clap::{Args, Parser, Subcommand};

use crate::{policy::CommitRange, report::Check};

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Rustwain repository automation")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the blocking checks locally: docs, policy, fmt, clippy and test.
    Ci(CiArgs),
    /// Design-library integrity.
    #[command(subcommand)]
    Docs(DocsCommand),
    /// Repository policy.
    #[command(subcommand)]
    Policy(PolicyCommand),
}

#[derive(Debug, Subcommand)]
pub enum DocsCommand {
    /// Check links, anchors, the index, decision markers and the pain-point tracker.
    Check(SkipArgs),
    /// Print `[verify]`, `[open]` and `[proposed]` counts per doc as markdown.
    Report,
    /// Append new pain-point IDs to the lockfile.
    LockIds,
}

#[derive(Debug, Subcommand)]
pub enum PolicyCommand {
    /// Check for non-Rust tooling files, AI attribution in commits and the branch name.
    Check(PolicyCheckArgs),
}

#[derive(Debug, Args)]
pub struct CiArgs {
    #[command(flatten)]
    pub skip: SkipArgs,
}

#[derive(Debug, Args)]
pub struct PolicyCheckArgs {
    /// Commits to inspect, as `<base>..<head>`. Defaults to `origin/main..HEAD`.
    #[arg(long)]
    pub range: Option<CommitRange>,
    #[command(flatten)]
    pub skip: SkipArgs,
}

/// `--skip` exists for false failures from an xtask bug (design 0001, section 6). The
/// reason is mandatory and is printed with the report and in the job summary.
#[derive(Debug, Default, Args)]
pub struct SkipArgs {
    /// Skip a check; repeatable. Requires `--skip-reason`.
    #[arg(long = "skip", value_enum, requires = "skip_reason")]
    pub checks: Vec<Check>,
    /// Why the checks are skipped.
    #[arg(long)]
    pub skip_reason: Option<String>,
}
