use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Locates the project root, the nearest ancestor of `start`
/// that contains a `.dcdc` directory.
///
/// Searching upward lets a script run from any subdirectory of the
/// project, not only the root itself.
pub fn find(start: &Path) -> Result<PathBuf> {
    let mut dir = start;
    loop {
        if dir.join(".dcdc").is_dir() {
            return Ok(dir.to_path_buf());
        }
        // `parent` returns `None` only at the filesystem root,
        // which ends the search.
        let Some(parent) = dir.parent() else {
            return Err(Error::NoProjectRoot(start.to_path_buf()));
        };
        dir = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    #[test]
    fn finds_the_nearest_root_above_nested_directories() {
        let base = testutil::temp_subdir("root");
        let inner = base.join("outer").join("inner");
        // Two roots on the path: the inner one must win.
        std::fs::create_dir_all(base.join("outer").join(".dcdc")).unwrap();
        std::fs::create_dir_all(inner.join(".dcdc")).unwrap();
        let workdir = inner.join("deep");
        std::fs::create_dir_all(&workdir).unwrap();

        assert_eq!(find(&workdir).unwrap(), inner);

        std::fs::remove_dir_all(base).unwrap();
    }
}
