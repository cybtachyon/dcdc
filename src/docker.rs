use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};
use crate::script::Script;

/// run copies a service script into its container and executes it.
///
/// `service` is the name of the `cmd` subdirectory that holds the
/// script. The subdirectory only targets docker when compose defines
/// a matching service and a running container exists for it.
pub fn run(script: &Script, service: &str, root: &Path) -> Result<i32> {
    let services = compose_services(root)?;
    if !services.iter().any(|name| name == service) {
        return Err(Error::NoService(service.to_string()));
    }
    let expected = container_name(root, service, 1)?;
    let Some(container) = container_for_service(root, service)? else {
        return Err(Error::NoContainer {
            service: service.to_string(),
            expected,
        });
    };
    copy_and_exec(&container, script)
}

/// compose_services lists the service names defined by the compose
/// file of `root`, as resolved by docker itself.
///
/// Querying docker rather than parsing the YAML means anchors,
/// profiles, and extra compose files are handled by compose, so the
/// check cannot drift from what `docker compose up` would create.
pub fn compose_services(root: &Path) -> Result<Vec<String>> {
    let out = query(Some(root), &["compose", "config", "--services"])?;
    Ok(lines(out))
}

/// exec_in runs a command inside a container with the given working
/// directory, with stdio inherited, and returns its exit code.
pub fn exec_in(container: &str, workdir: &str, argv: &[String]) -> Result<i32> {
    let mut args: Vec<String> = vec!["exec".into(), "-w".into(), workdir.into(), container.into()];
    args.extend(argv.iter().cloned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    execute(&refs)
}

/// Runs a command in a container and captures its stdout, for
/// version probes and the like.
pub fn exec_captured(container: &str, workdir: &str, argv: &[String]) -> Result<String> {
    let mut args: Vec<&str> = vec!["exec", "-w", workdir, container];
    args.extend(argv.iter().map(String::as_str));
    query(None, &args)
}

/// Whether `name` names a place a plugin sub-command could run:
/// `local` (the host), a compose service of `root`, or a running
/// container by that exact name.
///
/// A missing or broken compose file yields an empty service list,
/// so it cannot make a valid name fail; docker being absent only
/// hides the running-container fallback.
pub fn scope_found(name: &str, root: Option<&Path>) -> bool {
    if name == "local" {
        return true;
    }
    if let Some(root) = root
        && compose_services(root)
            .ok()
            .is_some_and(|services| services.iter().any(|s| s == name))
    {
        return true;
    }
    running_containers()
        .ok()
        .is_some_and(|names| names.iter().any(|n| n == name))
}

/// Resolves the target of a plugin sub-command: a compose service
/// name or a literal running container name, together with the
/// working directory to run in and whether the cwd is bind-mounted
/// into the container.
///
/// The workdir is the mapped container path of the cwd when a bind
/// mount carries it, which is what makes the cwd's mise.toml
/// visible inside the container; otherwise it is the container's
/// configured working directory.
pub fn resolve_container(name: &str, root: &Path, cwd: &Path) -> Result<(String, String, bool)> {
    // A missing or broken compose file should not stop a literal
    // container name from resolving, so a compose failure yields an
    // empty service list rather than an error.
    let services = compose_services(root).unwrap_or_default();
    let container = if services.iter().any(|s| s == name) {
        let expected = container_name(root, name, 1)?;
        let Some(container) = container_for_service(root, name)? else {
            return Err(Error::NoContainer {
                service: name.to_string(),
                expected,
            });
        };
        container
    } else {
        let names = running_containers()?;
        if !names.iter().any(|n| n == name) {
            return Err(Error::NoService(name.to_string()));
        }
        name.to_string()
    };

    let mounted = cwd_mount_point(&container, cwd)?;
    let cwd_mounted = mounted.is_some();
    let workdir = match mounted {
        Some(mapped) => mapped,
        None => container_workdir(&container)?,
    };
    Ok((container, workdir, cwd_mounted))
}

/// Maps the host `cwd` into the container through a bind mount,
/// returning the mount's destination path when it carries the cwd.
fn cwd_mount_point(container: &str, cwd: &Path) -> Result<Option<String>> {
    let raw = query(
        None,
        &["inspect", "--format", "{{json .Mounts}}", container],
    )?;
    let mounts: Vec<Mount> = serde_json::from_str(&raw).map_err(|e| Error::DockerFailed {
        command: format!("inspect {container}"),
        message: format!("bad mounts data: {e}"),
    })?;
    // Compare canonicalized paths; when the cwd cannot be
    // canonicalized, fall back to a raw string comparison.
    let want = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    Ok(mounts
        .iter()
        .filter(|m| m.kind == "bind")
        .find(|m| {
            let source = std::path::PathBuf::from(&m.source);
            source == want
                || std::fs::canonicalize(&source)
                    .map(|s| s == want)
                    .unwrap_or(false)
                || m.source == want.to_string_lossy()
        })
        .map(|m| m.destination.clone()))
}

/// One entry of a container's `Mounts` list.
#[derive(Debug, serde::Deserialize)]
struct Mount {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(rename = "Source")]
    source: String,
    #[serde(rename = "Destination")]
    destination: String,
}

/// The shell preferred in a container: `bash` when present, else
/// `sh`.
pub fn shell_in(container: &str, workdir: &str) -> Result<String> {
    let (status, _, _) = run_captured(
        None,
        &["exec", "-w", workdir, container, "bash", "--version"],
    )?;
    Ok(if status.success() {
        "bash".into()
    } else {
        "sh".into()
    })
}

/// The shell preferred on the host: `bash` when on the PATH, else
/// `sh`.
///
/// The probe's stdio is nulled, since its version banner must not
/// leak into the output of the command being run.
pub fn shell_local() -> String {
    match std::process::Command::new("bash")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) => {
            if status.success() {
                "bash".into()
            } else {
                "sh".into()
            }
        }
        Err(_) => "sh".into(),
    }
}

/// running_containers lists the names of the currently running
/// containers.
fn running_containers() -> Result<Vec<String>> {
    Ok(lines(query(None, &["ps", "--format", "{{.Names}}"])?))
}

/// container_for_service finds the running container that compose
/// would name for `service` in the project `root`.
///
/// The container name follows the compose format
/// `{dirname}-{service}-{n}`, where `dirname` is the project
/// directory name and `n` is the instance number, starting at 1.
/// When several instances run, the lowest number wins.
fn container_for_service(root: &Path, service: &str) -> Result<Option<String>> {
    let dirname = dirname(root)?;
    let names = running_containers()?;
    Ok(match_container(&names, dirname, service))
}

/// container_name renders the compose-style name
/// `{dirname}-{service}-{n}` for a given instance number.
fn container_name(root: &Path, service: &str, n: u32) -> Result<String> {
    let dirname = dirname(root)?;
    Ok(format!("{dirname}-{service}-{n}"))
}

/// dirname returns the project directory name, i.e., the name of the
/// directory that contains `.dcdc`.
fn dirname(root: &Path) -> Result<&str> {
    root.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::BadProjectName(root.to_path_buf()))
}

/// container_workdir returns the working directory the container
/// runs in, defaulting to `/` when the image sets none.
fn container_workdir(container: &str) -> Result<String> {
    let dir = query(
        None,
        &["inspect", "--format", "{{.Config.WorkingDir}}", container],
    )?;
    Ok(if dir.is_empty() { "/".into() } else { dir })
}

/// copy_and_exec mirrors a script into the container's `.dcdc/cmd`
/// directory and runs it with bash there.
///
/// Both steps anchor on the container's working directory, because
/// `docker cp` resolves relative paths from `/` while `docker exec`
/// runs in the working directory. Anchoring on the working directory
/// keeps the layout mirrored from the host and gives the script the
/// same relative paths the application sees.
fn copy_and_exec(container: &str, script: &Script) -> Result<i32> {
    let workdir = container_workdir(container)?;
    let cmd_dir = join(&workdir, ".dcdc/cmd");
    let filename = script
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let source = script.path.to_string_lossy().to_string();
    let dest = format!("{container}:{cmd_dir}/");
    let target = format!("{cmd_dir}/{filename}");

    // The directory is created first, per the spec, so a later
    // failure never leaves a half-mirrored layout.
    execute(&["exec", container, "mkdir", "-p", &cmd_dir])?;
    execute(&["cp", &source, &dest])?;
    execute(&["exec", container, "bash", &target])
}

/// query runs a docker subcommand and returns its trimmed stdout.
///
/// Output is captured through temporary files rather than pipes: in
/// some sandboxed environments the supervisor rewires a vforked
/// child's pipes, and piped output silently escapes the capture.
/// Files survive that, on this machine and ordinary ones alike.
fn query(root: Option<&Path>, args: &[&str]) -> Result<String> {
    let (status, out, err) = run_captured(root, args)?;
    if !status.success() {
        let message = err.trim().to_string();
        return Err(Error::DockerFailed {
            command: args.join(" "),
            message: if message.is_empty() {
                format!("{status}")
            } else {
                message
            },
        });
    }
    Ok(out.trim().to_string())
}

/// execute runs a docker subcommand with inherited stdio and returns
/// its exit code.
///
/// Inheriting stdio streams the script output straight to the
/// terminal, like a local run does.
fn execute(args: &[&str]) -> Result<i32> {
    let status = Command::new("docker")
        .args(args)
        .spawn()
        .map_err(spawn_error)?
        .wait()?;
    Ok(status.code().unwrap_or(1))
}

/// run_captured runs a docker subcommand with stdout and stderr
/// redirected to temporary files, and returns the exit status along
/// with the files' contents.
fn run_captured(
    root: Option<&Path>,
    args: &[&str],
) -> Result<(std::process::ExitStatus, String, String)> {
    let out_path = temp_output("out");
    let err_path = temp_output("err");
    let mut command = Command::new("docker");
    command.args(args);
    if let Some(root) = root {
        command.current_dir(root);
    }
    let status = command
        .stdout(Stdio::from(std::fs::File::create(&out_path)?))
        .stderr(Stdio::from(std::fs::File::create(&err_path)?))
        .spawn()
        .map_err(spawn_error)?
        .wait()?;
    // A failed read just yields empty output, which the caller
    // reports as a docker failure.
    let out = std::fs::read_to_string(&out_path).unwrap_or_default();
    let err = std::fs::read_to_string(&err_path).unwrap_or_default();
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
    Ok((status, out, err))
}

/// spawn_error maps a failed docker spawn to the right error, where a
/// missing binary is reported as `DockerMissing`.
///
/// @todo Detect whether docker is installed before entering the docker
/// path, and decide between falling back to a local run and printing
/// a setup hint, instead of surfacing the error from mid-flight.
fn spawn_error(err: std::io::Error) -> Error {
    match err.kind() {
        std::io::ErrorKind::NotFound => Error::DockerMissing,
        _ => Error::from(err),
    }
}

/// temp_output returns a fresh temp file path for captured output.
///
/// The counter keeps concurrent calls in the same process from
/// clashing on a name; the caller creates the file.
fn temp_output(kind: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("dcdc-query-{kind}-{}-{n}", std::process::id()))
}

/// match_container picks the running container named
/// `{dirname}-{service}-{n}` with the smallest `n`, if any.
///
/// `n` must be a plain number, so names that merely share the prefix
/// (for example `{dirname}-{service}-2b`) are ignored.
fn match_container(names: &[String], dirname: &str, service: &str) -> Option<String> {
    let prefix = format!("{dirname}-{service}-");
    names
        .iter()
        .filter_map(|name| {
            let number = name.strip_prefix(&prefix)?.parse::<u32>().ok()?;
            Some((number, name.clone()))
        })
        .min_by_key(|(number, _)| *number)
        .map(|(_, name)| name)
}

/// join glues `base` and `rest` with exactly one `/`, tolerating a
/// trailing slash on `base`.
fn join(base: &str, rest: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{rest}")
    } else {
        format!("{base}/{rest}")
    }
}

/// lines splits a captured docker command's output into non-empty
/// lines.
fn lines(out: String) -> Vec<String> {
    out.lines()
        .map(str::to_string)
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{match_container, scope_found};

    #[test]
    fn picks_the_lowest_running_instance() {
        let names = vec![
            "proj-api-2".to_string(),
            "proj-web-1".to_string(),
            "proj-api-1".to_string(),
        ];
        assert_eq!(
            match_container(&names, "proj", "api"),
            Some("proj-api-1".to_string())
        );
    }

    #[test]
    fn ignores_names_with_trailing_garbage_after_the_number() {
        let names = vec!["proj-api-1x".to_string(), "proj-api-2".to_string()];
        assert_eq!(
            match_container(&names, "proj", "api"),
            Some("proj-api-2".to_string())
        );
    }

    #[test]
    fn returns_none_when_no_container_matches() {
        let names = vec!["proj-web-1".to_string()];
        assert_eq!(match_container(&names, "proj", "api"), None);
    }

    #[test]
    fn the_local_scope_is_found_without_any_query() {
        // The host is always a valid scope, no docker required.
        assert!(scope_found("local", None));
        assert!(scope_found(
            "local",
            Some(std::path::Path::new("/nonexistent"))
        ));
    }
}
