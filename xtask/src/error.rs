//! Failures of xtask itself. A check that finds a problem is not an error: it returns a
//! [`crate::report::Finding`].
//!
//! The error enum is written by hand rather than with `thiserror`, because design 0001
//! limits xtask's dependencies to `clap`, `serde`, `toml` and `serde_json` to keep the
//! cold CI build short.

use std::{error::Error, fmt, io, path::PathBuf, process::ExitStatus};

#[derive(Debug)]
pub enum XtaskError {
    /// Reading or writing a repository file or the job-summary file.
    Io { path: PathBuf, source: io::Error },
    /// A subprocess (`git`, `cargo`) could not be started.
    Spawn { program: String, source: io::Error },
    /// A subprocess ran and exited unsuccessfully.
    CommandFailed {
        command: String,
        status: ExitStatus,
        stderr: String,
    },
    /// A subprocess printed output that is not UTF-8.
    NonUtf8Output { command: String },
    /// `--range` is not of the form `<base>..<head>`.
    InvalidRange { input: String },
}

impl fmt::Display for XtaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Spawn { program, source } => write!(f, "cannot run `{program}`: {source}"),
            Self::CommandFailed {
                command,
                status,
                stderr,
            } => write!(f, "`{command}` failed ({status}): {}", stderr.trim_end()),
            Self::NonUtf8Output { command } => write!(f, "`{command}` printed non-UTF-8 output"),
            Self::InvalidRange { input } => {
                write!(f, "invalid range `{input}`: expected `<base>..<head>`")
            }
        }
    }
}

impl Error for XtaskError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::Spawn { source, .. } => Some(source),
            Self::CommandFailed { .. } | Self::NonUtf8Output { .. } | Self::InvalidRange { .. } => {
                None
            }
        }
    }
}
