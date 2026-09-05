//! CLI integration tests.
//!
//! Each test builds a throwaway project in the system temp dir,
//! runs the dcdc binary against it, and checks the output. Docker
//! behavior is covered only for the failure paths, because the test
//! environment does not guarantee matching containers.

use std::path::{Path, PathBuf};
use std::process::Command;

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Builds a temp project with `.dcdc/cmd/hello.sh` and
/// `.dcdc/cmd/api/build.sh`, and returns its path.
///
/// The caller removes the directory.
fn project(tag: &str) -> PathBuf {
    // The counter keeps parallel test threads from sharing a
    // directory.
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("dcdc-{tag}-{}-{n}", std::process::id()));
    let cmd = dir.join(".dcdc").join("cmd");
    std::fs::create_dir_all(cmd.join("api")).unwrap();
    std::fs::write(
        cmd.join("hello.sh"),
        "#!/bin/bash\necho \"hello from $(pwd)\"\n",
    )
    .unwrap();
    std::fs::write(
        cmd.join("api").join("build.sh"),
        "#!/bin/bash\necho \"build\"\n",
    )
    .unwrap();
    dir
}

/// Runs the dcdc binary from `cwd` with the given args.
fn run_dcdc(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dcdc"))
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("failed to start the dcdc binary")
}

#[test]
fn no_arguments_lists_the_discovered_scripts() {
    let dir = project("list");
    let out = run_dcdc(&dir, &[]);
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let stdout = stdout(&out);
    assert!(stdout.contains("hello"), "stdout: {stdout}");
    assert!(stdout.contains("build"), "stdout: {stdout}");
    assert!(stdout.contains("docker: api"), "stdout: {stdout}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_local_script_runs_in_the_calling_directory() {
    let dir = project("local");
    // Run from a subdirectory, not the project root, so the test
    // proves the working directory is inherited, not the root.
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let out = run_dcdc(&sub, &["hello"]);
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let stdout = stdout(&out);
    assert!(
        stdout.contains("hello from") && stdout.contains(&sub.display().to_string()),
        "stdout: {stdout}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_extensionless_script_is_discovered_and_runs() {
    // The shipped example names its scripts without a `.sh`
    // extension, so a bare file under `.dcdc/cmd` must be runnable
    // by name.
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("dcdc-ext-{}-{n}", std::process::id()));
    let cmd = dir.join(".dcdc").join("cmd");
    std::fs::create_dir_all(&cmd).unwrap();
    std::fs::write(cmd.join("hello-world"), "#!/bin/sh\necho \"hi\"\n").unwrap();
    let out = run_dcdc(&dir, &["hello-world"]);
    let stdout = stdout(&out);
    assert!(out.status.success(), "stdout: {stdout}");
    assert!(stdout.contains("hi"), "stdout: {stdout}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unknown_script_name_reports_the_available_scripts() {
    let dir = project("unknown");
    let out = run_dcdc(&dir, &["nope"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("script nope not found"), "stderr: {stderr}");
    assert!(stderr.contains("hello"), "stderr: {stderr}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_service_script_fails_cleanly_without_a_matching_compose_setup() {
    let dir = project("dockerless");
    // The temp project defines no compose file, so the docker path
    // must fail regardless of whether docker itself is installed.
    let out = run_dcdc(&dir, &["build"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.trim().is_empty(), "stderr: {stderr}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// Renders a command's stdout for use in a failure message.
fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}
