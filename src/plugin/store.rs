//! Stores plugins that were downloaded from repositories.
//!
//! Each downloaded plugin lives in `<plugins-root>/owner-repo-SHA/`,
//! the commit SHA naming the exact version that was fetched. A
//! marker file per repository, under `<plugins-root>/.refs/`,
//! records the repository reference and the SHA that is currently
//! the active version, so `use` and `remove` can resolve a
//! repository to its directory and a collision error can spell the
//! repository back as a qualified name.
//!
//! The default plugins, synced from the binary, live in plain
//! `name/` directories without a SHA; this module never touches
//! them.

use std::path::{Path, PathBuf};

use super::repo::Repo;
use crate::error::{Error, Result};

/// The directory that holds per-repository markers.
const REFS_DIR: &str = ".refs";

/// The marker file of one repository inside the refs directory.
pub fn ref_file(root: &Path, repo: &Repo) -> PathBuf {
    root.join(REFS_DIR).join(repo.key())
}

/// The repository reference and SHA a marker records: the
/// reference is the first line when it is not a SHA, the SHA the
/// last line that looks like one. A one-line marker is a SHA with
/// no reference, as older installs wrote it.
fn parse_marker(text: &str) -> (Option<&str>, Option<&str>) {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let ref_ = lines.first().filter(|l| !looks_like_sha(l)).copied();
    let sha = lines.iter().rev().find(|l| looks_like_sha(l)).copied();
    (ref_, sha)
}

/// The SHA recorded as the active version of a repository, if any.
pub fn current_sha(root: &Path, repo: &Repo) -> Result<Option<String>> {
    let text = std::fs::read_to_string(ref_file(root, repo)).ok();
    Ok(text.and_then(|t| parse_marker(&t).1.map(str::to_string)))
}

/// The repository reference the marker recorded for the active
/// version of a repository key, when the install wrote one.
pub fn current_ref(root: &Path, key: &str) -> Result<Option<String>> {
    let text = std::fs::read_to_string(root.join(REFS_DIR).join(key)).ok();
    Ok(text.and_then(|t| parse_marker(&t).0.map(str::to_string)))
}

/// The installed directory of a repository's active version, if any.
pub fn installed_dir(root: &Path, repo: &Repo) -> Result<Option<PathBuf>> {
    let Some(sha) = current_sha(root, repo)? else {
        return Ok(None);
    };
    let dir = root.join(format!("{}-{}", repo.key(), sha));
    Ok(dir.is_dir().then_some(dir))
}

/// Moves an extracted plugin directory into place as
/// `owner-repo-SHA` and records the SHA as the active version.
///
/// `extracted` must contain a single top-level directory named
/// `owner-repo-SHA`, as the codeload archive provides.
pub fn install(root: &Path, repo: &Repo, extracted: &Path) -> Result<String> {
    std::fs::create_dir_all(root)?;
    let top = extracted
        .read_dir()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    if top.len() != 1 {
        return Err(Error::BadArchive(
            "expected exactly one top-level directory".to_string(),
        ));
    }
    let top = &top[0];
    let Some(name) = top.file_name().and_then(|n| n.to_str()) else {
        return Err(Error::BadArchive("directory has no name".to_string()));
    };
    // The directory name is owner-repo-SHA; the SHA is everything
    // after the last dash.
    let Some(dash) = name.rfind('-') else {
        return Err(Error::BadArchive(format!("{name} has no SHA suffix")));
    };
    let sha = &name[dash + 1..];
    if !looks_like_sha(sha) {
        return Err(Error::BadArchive(format!(
            "{sha} does not look like a commit SHA"
        )));
    }

    let dest = root.join(format!("{}-{}", repo.key(), sha));
    if !dest.exists() {
        std::fs::rename(top, &dest)?;
    }

    let refs = root.join(REFS_DIR);
    std::fs::create_dir_all(&refs)?;
    // The reference first, the SHA second: a marker without the
    // reference, as older installs wrote it, still reads.
    std::fs::write(
        ref_file(root, repo),
        format!("{}/{}\n{sha}\n", repo.owner, repo.repo),
    )?;
    Ok(sha.to_string())
}

/// Removes the active version of a repository and its marker.
pub fn remove(root: &Path, repo: &Repo) -> Result<()> {
    if let Some(dir) = installed_dir(root, repo)? {
        std::fs::remove_dir_all(dir)?;
    }
    let marker = ref_file(root, repo);
    if marker.is_file() {
        std::fs::remove_file(&marker)?;
    }
    Ok(())
}

/// A commit SHA is a hex string of at least the short length git
/// uses; the check keeps a malformed archive dir from being treated
/// as a version.
pub fn looks_like_sha(sha: &str) -> bool {
    sha.len() >= 7
        && sha.len() <= 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Splits a directory name of the form `owner-repo-SHA` into the
/// repository key and the SHA, when it has that shape.
pub fn split_dir_name(dir_name: &str) -> Option<(String, String)> {
    let dash = dir_name.rfind('-')?;
    let key = &dir_name[..dash];
    let sha = &dir_name[dash + 1..];
    if key.is_empty() || !looks_like_sha(sha) {
        return None;
    }
    Some((key.to_string(), sha.to_string()))
}

/// Removes the marker file of one repository key.
pub fn clear_marker(root: &Path, key: &str, sha: &str) -> Result<()> {
    let marker = root.join(REFS_DIR).join(key);
    if current_sha_by_key(root, key)?.as_deref() == Some(sha) {
        std::fs::remove_file(&marker)?;
    }
    Ok(())
}

/// The SHA recorded for a repository key, when the caller knows the
/// key but not a `Repo`.
pub fn current_sha_by_key(root: &Path, key: &str) -> Result<Option<String>> {
    let text = std::fs::read_to_string(root.join(REFS_DIR).join(key)).ok();
    Ok(text.and_then(|t| parse_marker(&t).1.map(str::to_string)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    #[test]
    fn install_moves_the_directory_and_records_the_sha() {
        let base = testutil::temp_subdir("store-install");
        let root = base.join("plugins");
        let extracted = base.join("extracted");
        let top = extracted.join("derek-shell-abcd1234");
        std::fs::create_dir_all(&top).unwrap();
        std::fs::write(top.join("shell.ts"), "// sub\n").unwrap();

        let repo = Repo {
            owner: "derek".into(),
            repo: "shell".into(),
        };
        let sha = install(&root, &repo, &extracted).unwrap();
        assert_eq!(sha, "abcd1234");
        assert!(root.join("derek-shell-abcd1234").join("shell.ts").is_file());
        assert_eq!(
            current_sha(&root, &repo).unwrap().as_deref(),
            Some("abcd1234")
        );
        // The marker records the reference too, so a qualified name
        // can spell the repository back.
        assert_eq!(
            current_ref(&root, "derek-shell").unwrap().as_deref(),
            Some("derek/shell")
        );
        assert!(installed_dir(&root, &repo).unwrap().is_some());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_one_line_marker_from_an_older_install_still_reads() {
        let base = testutil::temp_subdir("store-legacy");
        let root = base.join("plugins");
        let refs = root.join(REFS_DIR);
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("derek-shell"), "abcd1234\n").unwrap();
        let repo = Repo {
            owner: "derek".into(),
            repo: "shell".into(),
        };
        assert_eq!(
            current_sha(&root, &repo).unwrap().as_deref(),
            Some("abcd1234")
        );
        assert_eq!(
            current_sha_by_key(&root, "derek-shell").unwrap().as_deref(),
            Some("abcd1234")
        );
        // No reference was written by that install.
        assert_eq!(current_ref(&root, "derek-shell").unwrap(), None);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn remove_drops_the_directory_and_the_marker() {
        let base = testutil::temp_subdir("store-remove");
        let root = base.join("plugins");
        let extracted = base.join("extracted");
        let top = extracted.join("derek-shell-abcd1234");
        std::fs::create_dir_all(&top).unwrap();
        let repo = Repo {
            owner: "derek".into(),
            repo: "shell".into(),
        };
        install(&root, &repo, &extracted).unwrap();

        remove(&root, &repo).unwrap();
        assert!(!root.join("derek-shell-abcd1234").exists());
        assert!(!ref_file(&root, &repo).is_file());
        assert!(installed_dir(&root, &repo).unwrap().is_none());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_an_archive_without_a_sha_stamped_directory() {
        let base = testutil::temp_subdir("store-bad");
        let root = base.join("plugins");
        let extracted = base.join("extracted");
        std::fs::create_dir_all(extracted.join("no-sha")).unwrap();
        let repo = Repo {
            owner: "derek".into(),
            repo: "shell".into(),
        };
        let err = install(&root, &repo, &extracted).unwrap_err();
        assert!(matches!(err, Error::BadArchive(_)), "{err:?}");
        std::fs::remove_dir_all(base).unwrap();
    }
}
