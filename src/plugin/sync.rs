//! Syncs the default plugins embedded in the binary into the dcdc
//! home.
//!
//! The `plugins/` directory of the source tree is embedded at build
//! time. This is the post-update function of the plan: however the
//! binary arrived on the machine (installer, package, self-update),
//! a stale or missing installation is re-extracted.
//!
//! The sync runs in the background so that ordinary commands never
//! wait for it. A lock file keeps at most one sync running across
//! all dcdc processes; `dcdc plugin` commands wait for the sync to
//! finish before proceeding and show a progress bar while they do.

use std::path::Path;
use std::time::Duration;

use include_dir::{Dir, DirEntry, include_dir};

use crate::error::{Error, Result};

/// The default plugins embedded at build time.
static DEFAULT_PLUGINS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/plugins");

/// Stamp file recording which dcdc version installed the defaults.
const STAMP: &str = ".sync-version";

/// How long a lock may live before a later process takes it over.
///
/// A crash mid-sync leaves the lock behind; a minute is well beyond
/// the few milliseconds the extraction takes, so a still-fresh lock
/// means a live process.
const LOCK_STALE_AFTER: Duration = Duration::from_secs(60);

/// A plugin sync started by this process, if any.
pub struct BackgroundSync {
    /// The thread running this process's sync, when one was started.
    handle: Option<std::thread::JoinHandle<()>>,
    /// The result of that sync, reported over a channel because a
    /// std JoinHandle cannot be polled.
    rx: Option<std::sync::mpsc::Receiver<Result<Vec<String>>>>,
    /// True when another process holds the sync lock.
    external: bool,
}

impl BackgroundSync {
    /// A sync that is not running anywhere, used when starting one
    /// failed.
    pub fn none() -> Self {
        Self {
            handle: None,
            rx: None,
            external: false,
        }
    }
    /// Whether a sync may still be changing the home tree: one
    /// running in this process, or one held by another process.
    pub fn is_running(&self) -> bool {
        self.rx.is_some() || self.external
    }
    /// Waits until the default plugin sync has completed.
    ///
    /// Used only by the `dcdc plugin` commands, which must not act
    /// on a half-installed set. A progress bar shows while waiting.
    /// Returns the plugin names this process installed.
    ///
    /// Borrowing, not consuming, so a caller may wait once before
    /// deciding and the wait stays available for the caller that
    /// finally needs it; a repeat after the work is done is a
    /// no-op.
    pub fn wait(&mut self) -> Result<Vec<String>> {
        let root = super::plugins_root()?;
        if is_fresh(&root) {
            return Ok(Vec::new());
        }
        let bar = spinner("syncing default plugins…");
        let Some(rx) = self.rx.take() else {
            return self.wait_external(&bar);
        };
        let result = loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => break result,
                // The sender dropped without reporting: the thread
                // panicked, so join it to find out.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = self.handle.take().map(|h| h.join());
                    bar.finish_and_clear();
                    return Err(Error::PluginRuntime {
                        name: "sync".into(),
                        message: "the sync thread panicked".into(),
                    });
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    bar.tick();
                }
            }
        };
        bar.finish_and_clear();
        // Join for cleanliness once the work is done.
        let _ = self.handle.take().map(|h| h.join());
        result
    }

    /// Waits on a sync that another process is running.
    ///
    /// Polls the shared lock until the stamp becomes fresh, the
    /// lock goes away, or the lock goes stale, whichever comes
    /// first.
    fn wait_external(&self, bar: &indicatif::ProgressBar) -> Result<Vec<String>> {
        let root = super::plugins_root()?;
        if self.external {
            let lock = lock_path();
            loop {
                if is_fresh(&root) {
                    break;
                }
                match read_lock(&lock) {
                    Some((_, start)) if !is_stale(start) => {
                        bar.set_message("waiting for another dcdc to finish the plugin sync…");
                        bar.tick();
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    _ => break,
                }
            }
        }
        bar.finish_and_clear();
        Ok(Vec::new())
    }
}

/// Starts the default plugin sync in the background, if it is due.
///
/// A fresh installation starts nothing; a stale one either starts
/// this process's sync (acquiring the lock) or observes another
/// process holding it.
pub fn start_background() -> Result<BackgroundSync> {
    let root = super::plugins_root()?;
    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if is_fresh(&root) {
        return Ok(BackgroundSync {
            handle: None,
            rx: None,
            external: false,
        });
    }

    let lock = lock_path();
    match read_lock(&lock) {
        Some((pid, start)) if !is_stale(start) && pid_alive(pid) => {
            return Ok(BackgroundSync {
                handle: None,
                rx: None,
                external: true,
            });
        }
        // No lock, or a stale one: take over.
        _ => acquire_lock(&lock)?,
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let root = root.clone();
    let lock = lock.clone();
    let handle = std::thread::spawn(move || {
        let names = sync_now(&root);
        let _ = std::fs::remove_file(&lock);
        let _ = tx.send(names);
    });
    Ok(BackgroundSync {
        handle: Some(handle),
        rx: Some(rx),
        external: false,
    })
}

/// The sync lock lives in the dcdc home, next to the plugins
/// directory, so re-extracting the plugins cannot destroy it.
fn lock_path() -> std::path::PathBuf {
    crate::config::home_dir()
        .map(|home| home.join(".dcdc").join(".sync.lock"))
        .unwrap_or_default()
}

/// Whether the installed default set matches the dcdc version that
/// built it.
pub fn is_fresh(root: &Path) -> bool {
    let stamp = root.join(STAMP);
    root.is_dir()
        && stamp.is_file()
        && std::fs::read_to_string(&stamp)
            .map(|s| s.trim() == env!("CARGO_PKG_VERSION"))
            .unwrap_or(false)
}

/// Re-extracts the embedded default plugins into `root` and writes
/// the stamp, returning the names of the plugin directories written.
///
/// Overwrites files in place, so a repeat after an interrupted sync
/// repairs rather than duplicates.
pub fn sync_now(root: &Path) -> Result<Vec<String>> {
    std::fs::create_dir_all(root)?;
    let mut names = Vec::new();
    for entry in DEFAULT_PLUGINS.entries() {
        let DirEntry::Dir(dir) = entry else {
            continue;
        };
        // `dir.path()` is relative to the include root, so only the
        // last segment names the plugin directory.
        let name = dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        copy_dir(&root.join(name), dir)?;
        names.push(name.to_string());
    }
    std::fs::write(root.join(STAMP), env!("CARGO_PKG_VERSION"))?;
    Ok(names)
}

/// Recursively copies the embedded directory `src` into `dest`,
/// overwriting files as it goes.
///
/// The embedded paths are relative to the include root, so each
/// level keeps only the last segment and joins it onto `dest`.
fn copy_dir(dest: &Path, src: &Dir<'_>) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in src.entries() {
        match entry {
            DirEntry::Dir(sub) => {
                let name = sub
                    .path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if !name.is_empty() {
                    copy_dir(&dest.join(name), sub)?;
                }
            }
            DirEntry::File(file) => {
                let name = file
                    .path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if !name.is_empty() {
                    std::fs::write(dest.join(name), file.contents())?;
                }
            }
        }
    }
    Ok(())
}

/// The lock, as a process id and a start timestamp, if present and
/// parseable.
fn read_lock(path: &Path) -> Option<(i32, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.parse().ok()?;
    let start = lines.next()?.parse().ok()?;
    Some((pid, start))
}

fn acquire_lock(path: &Path) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n{now}\n", std::process::id()))?;
    Ok(())
}

fn is_stale(start: u64) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(start) > LOCK_STALE_AFTER.as_secs()
}

/// Whether the process id of a lock is still running.
///
/// On unix, a zero-signal probe reports it; elsewhere, liveness
/// cannot be probed, so a lock can only go stale by age.
fn pid_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        if pid <= 1 {
            return false;
        }
        unsafe { libc::kill(pid, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// A spinner bar for the wait of a `dcdc plugin` command.
///
/// The message must be static: indicatif 0.18 stores the message by
/// reference in the bar's state.
fn spinner(message: &'static str) -> indicatif::ProgressBar {
    let bar = indicatif::ProgressBar::new_spinner();
    bar.set_style(
        indicatif::ProgressStyle::with_template("{msg} {spinner}").expect("valid template"),
    );
    bar.set_message(message);
    bar.enable_steady_tick(Duration::from_millis(100));
    bar
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    #[test]
    fn the_embedded_tree_contains_the_shell_plugin() {
        // A guard on the build-time embedding: if the `plugins/`
        // directory ever loses its sub-command layout, this fails
        // instead of shipping an empty default set.
        let names: Vec<String> = DEFAULT_PLUGINS
            .entries()
            .iter()
            .filter_map(|e| match e {
                DirEntry::Dir(d) => d.path().file_name()?.to_str()?.to_string().into(),
                DirEntry::File(_) => None,
            })
            .collect();
        assert!(names.contains(&"shell".to_string()), "names: {names:?}");
        assert!(names.contains(&"sdk".to_string()), "names: {names:?}");
    }

    #[test]
    fn sync_now_writes_the_tree_and_the_stamp() {
        let base = testutil::temp_subdir("sync-now");
        let root = base.join("plugins");
        let names = sync_now(&root).unwrap();
        assert!(names.contains(&"shell".to_string()), "names: {names:?}");
        assert!(root.join("shell").join("shell.ts").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join(STAMP)).unwrap().trim(),
            env!("CARGO_PKG_VERSION")
        );
        assert!(is_fresh(&root));
        std::fs::remove_dir_all(base).unwrap();
    }
}
