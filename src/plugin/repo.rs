//! Normalizes GitHub repository references and builds the URLs used
//! to fetch them.
//!
//! Accepted forms, with or without a scheme and with or without a
//! `.git` suffix:
//!
//! - `owner/repo`
//! - `github.com/owner/repo`
//! - `https://github.com/owner/repo`

use crate::error::{Error, Result};

/// A normalized GitHub repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub repo: String,
}

/// Normalizes a user-supplied repository reference.
///
/// Returns `InvalidRepoRef` when the value does not name a
/// `github.com` repository with an owner and a name.
pub fn normalize(input: &str) -> Result<Repo> {
    let mut ref_ = input.trim();
    // Drop a scheme, if present.
    if let Some(idx) = ref_.find("://") {
        ref_ = &ref_[idx + 3..];
    }
    // A bare `owner/repo` is kept as-is; a `github.com/owner/repo`
    // drops the host. A dot in the first segment means a foreign
    // host, since GitHub owner names never contain dots.
    if let Some(rest) = ref_.strip_prefix("github.com/") {
        ref_ = rest;
    } else if let Some(first) = ref_.split('/').next()
        && first.contains('.')
    {
        return Err(Error::InvalidRepoRef(input.to_string()));
    }
    // Drop any trailing slash, then a .git suffix.
    let ref_ = ref_.trim_end_matches('/');
    let ref_ = ref_.strip_suffix(".git").unwrap_or(ref_);
    let slash = ref_.find('/');
    let Some(slash) = slash else {
        return Err(Error::InvalidRepoRef(input.to_string()));
    };
    let owner = ref_[..slash].to_string();
    let repo = ref_[slash + 1..].to_string();
    if owner.is_empty() || repo.is_empty() {
        return Err(Error::InvalidRepoRef(input.to_string()));
    }
    Ok(Repo { owner, repo })
}

impl Repo {
    /// The directory name the plugin is stored under, without the
    /// commit SHA.
    pub fn key(&self) -> String {
        format!("{}-{}", self.owner, self.repo)
    }

    /// The codeload URL for the tarball of a branch or tag.
    ///
    /// GitHub serves a plain tarball, gzipped, from the codeload
    /// host; no git client or credentials are needed. The bare ref
    /// form is used so that tags work as well as branches.
    pub fn tarball_url(&self, ref_name: &str) -> String {
        format!(
            "https://codeload.github.com/{}/{}/tar.gz/{}",
            self.owner, self.repo, ref_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_every_accepted_form() {
        for input in [
            "derek/shell",
            "derek/shell.git",
            "github.com/derek/shell",
            "github.com/derek/shell.git",
            "https://github.com/derek/shell",
            "http://github.com/derek/shell.git/",
        ] {
            let repo = normalize(input).unwrap();
            assert_eq!(repo.owner, "derek", "{input}");
            assert_eq!(repo.repo, "shell", "{input}");
        }
    }

    #[test]
    fn rejects_repos_on_other_hosts_and_malformed_refs() {
        for input in [
            "gitlab.com/derek/shell",
            "https://github.com/derek",
            "derek",
            "/repo",
            "owner/",
        ] {
            assert!(normalize(input).is_err(), "{input}");
        }
    }

    #[test]
    fn builds_the_codeload_url() {
        let repo = normalize("derek/shell").unwrap();
        assert_eq!(
            repo.tarball_url("main"),
            "https://codeload.github.com/derek/shell/tar.gz/main"
        );
    }
}
