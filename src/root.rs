use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Locates the project root, the nearest ancestor of `start`
/// that contains a `.dcdc` directory.
///
/// Searching upward lets a script run from any subdirectory of the
/// project, not only the root itself. The dcdc home's own `.dcdc`
/// directory is not a project marker, so directories above it keep
/// the home-level configuration in force.
pub fn find(start: &Path) -> Result<PathBuf> {
    let home = crate::config::home_dir().ok();
    let mut dir = start;
    loop {
        if dir.join(".dcdc").is_dir() && home.as_deref() != Some(dir) {
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
    fn the_dcdc_home_is_not_a_project_marker() {
        let base = testutil::temp_subdir("root-home");
        // The home holds the dcdc home's own `.dcdc` directory; a
        // subdirectory of it is not a project.
        let home = base.join("home");
        std::fs::create_dir_all(home.join(".dcdc").join("plugins")).unwrap();
        let sub = home.join("somewhere");
        std::fs::create_dir_all(&sub).unwrap();

        // Edition 2024 makes set_var unsafe; the lock keeps this
        // write from overlapping the other environment tests.
        let _lock = testutil::env_lock();
        unsafe { std::env::set_var("DCDC_HOME", &home) };
        let err = find(&sub).unwrap_err();
        assert!(matches!(err, Error::NoProjectRoot(_)), "{err:?}");
        std::fs::remove_dir_all(base).unwrap();
    }

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
