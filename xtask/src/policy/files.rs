//! `RustOnlyFiles`: no tooling in another language.

use std::{ffi::OsStr, num::NonZeroU32, path::PathBuf};

use crate::report::{Check, Finding, Location};

const MESSAGE: &str = "non-Rust tooling file; automation lives in `cargo xtask` (AGENTS.md)";

/// Compared case-insensitively.
const FOREIGN_EXTENSIONS: &[&str] = &[
    "py", "pyi", "ipynb", "sh", "bash", "zsh", "fish", "ps1", "psm1", "bat", "cmd", "js", "mjs",
    "cjs", "ts", "mts", "cts", "jsx", "tsx", "go", "rb", "pl", "pm", "php", "lua", "java", "kt",
    "kts", "scala", "groovy", "gradle", "swift", "c", "h", "cc", "cpp", "cxx", "hpp", "mk",
    "cmake",
];

/// Build and package manifests of other ecosystems; compared exactly.
const FOREIGN_FILE_NAMES: &[&str] = &[
    "Makefile",
    "makefile",
    "GNUmakefile",
    "CMakeLists.txt",
    "justfile",
    "Justfile",
    "package.json",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "requirements.txt",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "Pipfile",
    "Gemfile",
    "go.mod",
    "build.gradle",
    "Rakefile",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedFile {
    /// Repository-relative, as printed by `git ls-files`.
    pub path: PathBuf,
    /// The file starts with `#!`. The loader reads it only for files without an
    /// extension, which are the only scripts the extension list cannot recognise.
    pub has_shebang: bool,
}

impl TrackedFile {
    pub fn new(path: impl Into<PathBuf>, has_shebang: bool) -> Self {
        Self {
            path: path.into(),
            has_shebang,
        }
    }

    fn is_foreign_tooling(&self) -> bool {
        let extension = self.path.extension().and_then(OsStr::to_str);
        let name = self.path.file_name().and_then(OsStr::to_str);
        extension.is_some_and(|extension| {
            FOREIGN_EXTENSIONS
                .iter()
                .any(|foreign| foreign.eq_ignore_ascii_case(extension))
        }) || name.is_some_and(|name| FOREIGN_FILE_NAMES.contains(&name))
            || (extension.is_none() && self.has_shebang)
    }
}

pub fn rust_only_files(files: &[TrackedFile]) -> Vec<Finding> {
    files
        .iter()
        .filter(|file| file.is_foreign_tooling())
        .map(|file| {
            let location = Location::new(&file.path, NonZeroU32::MIN);
            Finding::error(Check::RustOnlyFiles, Some(location), MESSAGE)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged_paths(files: &[TrackedFile]) -> Vec<String> {
        rust_only_files(files)
            .into_iter()
            .map(|finding| {
                let location = finding.location.unwrap();
                format!("{}:{}", location.path.display(), location.line)
            })
            .collect()
    }

    #[test]
    fn rust_and_data_files_pass() {
        let files = [
            TrackedFile::new("src/main.rs", false),
            TrackedFile::new("README.md", false),
            TrackedFile::new("Cargo.toml", false),
            TrackedFile::new("Cargo.lock", false),
            TrackedFile::new(".github/workflows/ci.yml", false),
            TrackedFile::new("bench/data.json", false),
            TrackedFile::new("LICENSE", false),
        ];

        assert_eq!(rust_only_files(&files), []);
    }

    #[test]
    fn tooling_files_are_flagged_by_extension_name_and_shebang() {
        let files = [
            TrackedFile::new("scripts/build.py", false),
            TrackedFile::new("scripts/Deploy.SH", false),
            TrackedFile::new("Makefile", false),
            TrackedFile::new("web/package.json", false),
            TrackedFile::new("bin/tool", true),
            TrackedFile::new("docs/example.md", true),
            TrackedFile::new("src/lib.rs", false),
        ];

        assert_eq!(
            flagged_paths(&files),
            [
                "scripts/build.py:1",
                "scripts/Deploy.SH:1",
                "Makefile:1",
                "web/package.json:1",
                "bin/tool:1",
            ]
        );
        assert_eq!(
            rust_only_files(&files)[0],
            Finding::error(
                Check::RustOnlyFiles,
                Some(Location::new("scripts/build.py", NonZeroU32::MIN)),
                "non-Rust tooling file; automation lives in `cargo xtask` (AGENTS.md)"
            )
        );
    }
}
