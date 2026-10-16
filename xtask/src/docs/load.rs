//! Reading the design library from disk. This is the only docs code that does I/O.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

use super::{
    model::{Doc, DocSet, RelPath},
    parse::parse_doc,
};
use crate::error::XtaskError;

const DOCS_DIR: &str = "docs";
const MARKDOWN_EXTENSION: &str = "md";

/// Reads and parses every `docs/**/*.md` under `repo_root`, and records which
/// non-markdown targets of relative links exist on disk.
///
/// Symbolic links are not followed. Existence is decided by the host filesystem, so on a
/// case-insensitive filesystem (default macOS) a link to a non-doc path with the wrong
/// case passes locally and fails on Linux CI. Links to docs are matched case-sensitively
/// everywhere.
pub fn load(repo_root: &Path) -> Result<DocSet, XtaskError> {
    let mut docs = BTreeMap::new();
    read_dir(&repo_root.join(DOCS_DIR), DOCS_DIR, &mut docs)?;

    let mut set = DocSet {
        docs,
        existing_targets: BTreeSet::new(),
    };
    let non_doc_targets: BTreeSet<_> = set
        .relative_links()
        .filter_map(|link| link.resolved)
        .filter(|resolved| !set.docs.contains_key(resolved))
        .collect();
    set.existing_targets = non_doc_targets
        .into_iter()
        .filter(|resolved| repo_root.join(resolved.as_str()).exists())
        .collect();
    Ok(set)
}

/// Parses the markdown files under `dir`, whose repository-relative path is `relative`.
fn read_dir(
    dir: &Path,
    relative: &str,
    docs: &mut BTreeMap<RelPath, Doc>,
) -> Result<(), XtaskError> {
    let io_error = |source| XtaskError::Io {
        path: dir.to_path_buf(),
        source,
    };
    for entry in fs::read_dir(dir).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(io_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(XtaskError::Io {
                path,
                source: io::Error::new(io::ErrorKind::InvalidData, "file name is not UTF-8"),
            });
        };
        let child = format!("{relative}/{name}");

        if file_type.is_dir() {
            read_dir(&path, &child, docs)?;
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext == MARKDOWN_EXTENSION)
        {
            let text = fs::read_to_string(&path).map_err(|source| XtaskError::Io {
                path: path.clone(),
                source,
            })?;
            docs.insert(RelPath::new(child), parse_doc(&text));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process};

    use super::*;

    /// A scratch repository under the system temp dir, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("xtask-docs-{name}-{}", process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn load_reads_nested_docs_and_records_existing_non_doc_targets() {
        let repo = Scratch::new("nested");
        repo.write(
            "docs/README.md",
            "[a](a/one.md) [root](../README.md) [dir](a/) [gone](missing.rs) [ext](https://x.y)",
        );
        repo.write("docs/a/one.md", "# One\n[up](../README.md#top)\n");
        repo.write("docs/a/notes.txt", "[ignored](nope.md)");
        repo.write("README.md", "# Root\n");

        let set = load(&repo.0).unwrap();

        let docs: Vec<_> = set.docs.keys().map(RelPath::as_str).collect();
        assert_eq!(docs, ["docs/README.md", "docs/a/one.md"]);
        let existing: Vec<_> = set.existing_targets.iter().map(RelPath::as_str).collect();
        assert_eq!(existing, ["README.md", "docs/a/"]);
    }

    #[test]
    fn load_without_a_docs_directory_is_an_io_error() {
        let repo = Scratch::new("empty");

        let err = load(&repo.0).unwrap_err();

        assert!(matches!(err, XtaskError::Io { ref path, .. } if path.ends_with("docs")));
    }
}
