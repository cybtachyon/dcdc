//! Shared helpers for unit tests.

use std::path::PathBuf;

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Creates a fresh, empty directory under the system
/// temp dir and returns its path.
///
/// The per-process counter keeps concurrent tests in the same
/// process from sharing a directory; callers are expected to remove
/// the directory when done.
pub fn temp_subdir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("dcdc-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&path).expect("failed to create the temp directory");
    path
}
