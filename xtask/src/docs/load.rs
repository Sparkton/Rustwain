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
/// Symbolic links are not followed when walking `docs/`. A non-doc target exists only if
/// every path segment matches a directory entry exactly, so a link with the wrong case
/// fails on a case-insensitive filesystem (default macOS) just as it does on Linux, and
/// links to docs are matched exactly everywhere.
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
        .filter(|resolved| exists_with_exact_case(repo_root, resolved))
        .collect();
    Ok(set)
}

/// Whether `target` names an existing file or directory, comparing each segment's case
/// against the directory listing instead of trusting the filesystem's lookup. A trailing
/// `/` requires a directory.
fn exists_with_exact_case(repo_root: &Path, target: &RelPath) -> bool {
    let mut path = repo_root.to_path_buf();
    for segment in target.as_str().split('/').filter(|s| !s.is_empty()) {
        let listed = fs::read_dir(&path)
            .is_ok_and(|entries| entries.flatten().any(|entry| entry.file_name() == segment));
        if !listed {
            return false;
        }
        path.push(segment);
    }
    !target.as_str().ends_with('/') || path.is_dir()
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

    // A mis-cased link must fail here even where the filesystem ignores case (macOS), so
    // local and Linux CI results match.
    #[test]
    fn load_requires_exact_case_for_non_doc_targets() {
        let repo = Scratch::new("case");
        repo.write(
            "docs/README.md",
            "[ok](../README.md) [file](../sub/x.rs) [dir](../sub/) \
             [bad1](../readme.md) [bad2](../sub/X.rs) [bad3](../Sub/) [bad4](../sub/x.rs/)",
        );
        repo.write("README.md", "# Root\n");
        repo.write("sub/x.rs", "");

        let set = load(&repo.0).unwrap();

        let existing: Vec<_> = set.existing_targets.iter().map(RelPath::as_str).collect();
        assert_eq!(existing, ["README.md", "sub/", "sub/x.rs"]);
    }

    #[test]
    fn load_without_a_docs_directory_is_an_io_error() {
        let repo = Scratch::new("empty");

        let err = load(&repo.0).unwrap_err();

        assert!(matches!(err, XtaskError::Io { ref path, .. } if path.ends_with("docs")));
    }
}
