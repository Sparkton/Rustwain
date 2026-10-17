//! A scratch directory for tests that touch the file system. It is named from the process
//! id and the test, so parallel tests never share one, and it is removed when dropped.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

pub(crate) struct TempDir {
    root: PathBuf,
}

impl TempDir {
    /// `test_name` must be unique across the crate's tests.
    pub(crate) fn new(test_name: &str) -> Self {
        let root = env::temp_dir().join(format!("xtask-{}-{test_name}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    /// Writes `text` to `relative`, creating the parent directories.
    pub(crate) fn write(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub(crate) fn read(&self, relative: &str) -> Option<String> {
        fs::read_to_string(self.root.join(relative)).ok()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
