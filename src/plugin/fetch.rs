//! Downloads a plugin repository from GitHub and unpacks it.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use super::repo::Repo;
use crate::error::{Error, Result};

/// The User-Agent of the HTTP requests.
///
/// The GitHub API rejects requests without one, and a
/// product-bearing agent keeps support traffic attributable.
const USER_AGENT: &str = "dcdc";

/// Downloads a repository at a branch or tag and unpacks it into a
/// fresh temp directory, which it returns.
///
/// A `None` branch resolves through the GitHub API to the
/// repository's default branch. The unpacked directory is renamed
/// to `owner-repo-SHA`, the commit the archive was cut from, so the
/// store can key the installation by version. The caller removes
/// the temp directory.
pub fn fetch(repo: &Repo, branch: Option<&str>) -> Result<PathBuf> {
    let ref_name = match branch {
        Some(b) => b.to_string(),
        None => default_branch(repo)?,
    };
    let sha = commit_sha(repo, &ref_name)?;
    let url = repo.tarball_url(&ref_name);
    eprintln!("dcdc: downloading {url}");
    let body = get_bytes(&url)?;

    let temp = std::env::temp_dir().join(format!(
        "dcdc-fetch-{}-{}-{}",
        repo.key(),
        std::process::id(),
        counter()
    ));
    std::fs::create_dir_all(&temp)?;
    let gz = flate2::read::GzDecoder::new(Cursor::new(body));
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(&temp)
        .map_err(|e| Error::BadArchive(e.to_string()))?;

    // The archive directory is named after the ref, so rename it to
    // the SHA-stamped name the store expects.
    let top = single_dir(&temp)?;
    let dest = temp.join(format!("{}-{sha}", repo.key()));
    std::fs::rename(&top, &dest)?;
    Ok(temp)
}

/// The default branch of a public repository, from the GitHub API.
///
/// Anonymous calls are rate limited, so a failure falls back to
/// `main`, the most common default, which the download error will
/// surface if it is wrong.
fn default_branch(repo: &Repo) -> Result<String> {
    let url = format!("https://api.github.com/repos/{}/{}", repo.owner, repo.repo);
    let body = get_bytes(&url)?;
    #[derive(serde::Deserialize)]
    struct Info {
        default_branch: String,
    }
    serde_json::from_slice::<Info>(&body)
        .map(|i| i.default_branch)
        .map_err(|e| Error::DownloadFailed(format!("parsing the repository info: {e}")))
}

/// The commit SHA a branch or tag points at, from the GitHub API.
fn commit_sha(repo: &Repo, ref_name: &str) -> Result<String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/commits/{}",
        repo.owner, repo.repo, ref_name
    );
    let body = get_bytes(&url)?;
    #[derive(serde::Deserialize)]
    struct Commit {
        sha: String,
    }
    serde_json::from_slice::<Commit>(&body)
        .map(|c| c.sha)
        .map_err(|e| {
            Error::DownloadFailed(format!(
                "could not resolve the commit SHA for {ref_name} ({e}); \
                 the GitHub API may be rate limited"
            ))
        })
}

/// The single directory unpacked into `dir`, when there is exactly
/// one.
fn single_dir(dir: &Path) -> Result<PathBuf> {
    let mut top = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().is_dir() {
            if top.is_some() {
                return Err(Error::BadArchive(
                    "the archive unpacked more than one directory".to_string(),
                ));
            }
            top = Some(entry.path());
        }
    }
    top.ok_or_else(|| Error::BadArchive("the archive unpacked no top-level directory".to_string()))
}

/// The shared blocking client, with the dcdc User-Agent.
fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("a valid client")
}

/// Downloads `url`, requiring a successful status.
fn get_bytes(url: &str) -> Result<Vec<u8>> {
    let response = client()
        .get(url)
        .send()
        .map_err(|e| Error::DownloadFailed(format!("GET {url}: {e}")))?;
    let status = response.status();
    let body = response
        .bytes()
        .map_err(|e| Error::DownloadFailed(format!("GET {url}: {e}")))?;
    if !status.is_success() {
        return Err(Error::DownloadFailed(format!(
            "HTTP {status} for {url} (wrong branch or tag, or the repository is not public?)"
        )));
    }
    Ok(body.to_vec())
}

/// A per-process counter that keeps concurrent fetch temp dirs apart.
fn counter() -> u64 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}
