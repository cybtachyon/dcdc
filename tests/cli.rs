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
    run_dcdc_env(cwd, args, None)
}

/// Like `run_dcdc`, with an optional override of the dcdc home.
///
/// The home override keeps plugin sync and discovery inside a temp
/// directory, so tests never touch the real `~/.dcdc`.
fn run_dcdc_env(cwd: &Path, args: &[&str], home: Option<&Path>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dcdc"));
    cmd.current_dir(cwd);
    cmd.args(args);
    if let Some(home) = home {
        cmd.env("DCDC_HOME", home);
    }
    // Null stdin keeps the child non-interactive, so dcdc never
    // prompts in a test, whatever terminal the suite runs in.
    cmd.stdin(std::process::Stdio::null());
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to start the dcdc binary: {e:?}"))
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

/// Renders a command's stderr for use in a failure message.
fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn plugin_list_shows_the_shell_plugin_with_its_aliases_and_version() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = run_dcdc_env(&home, &["plugin", "list"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let stdout = stdout(&out);
    // The sub-command, its aliases, and the version read from the
    // TypeScript all appear in the listing.
    assert!(stdout.contains("shell"), "stdout: {stdout}");
    assert!(stdout.contains("bash, sh"), "stdout: {stdout}");
    assert!(stdout.contains("0.1.0"), "stdout: {stdout}");
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_local_shell_plugin_runs_on_the_host() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home2-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = run_dcdc_env(
        &home,
        &["-c", "local", "bash", "echo", "Hello world!"],
        Some(&home),
    );
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    let stdout = stdout(&out);
    assert!(stdout.contains("Hello world!"), "stdout: {stdout}");
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_plugin_without_a_container_or_config_reports_no_target() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home3-{}-{n}", std::process::id()));
    // A project with a config naming the container for the plugin.
    let dir = std::env::temp_dir().join(format!("dcdc-notarget-{}-{n}", std::process::id()));
    let cmd = dir.join(".dcdc").join("cmd");
    std::fs::create_dir_all(&cmd).unwrap();
    std::fs::write(
        dir.join("dcdc.toml"),
        "[plugin.shell]\ncontainer = \"local\"\n",
    )
    .unwrap();

    // With the config, the same call resolves to the host.
    let out = run_dcdc_env(&dir, &["bash", "echo", "configured"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("configured"),
        "stdout: {}",
        stdout(&out)
    );

    // Without it, the call fails with the hint to set one.
    let bare = std::env::temp_dir().join(format!("dcdc-notarget2-{}-{n}", std::process::id()));
    let bare_cmd = bare.join(".dcdc").join("cmd");
    std::fs::create_dir_all(&bare_cmd).unwrap();
    let out = run_dcdc_env(&bare, &["bash"], Some(&home));
    assert!(!out.status.success());
    let stderr = stderr(&out);
    assert!(stderr.contains("has no container"), "stderr: {stderr}");
    assert!(stderr.contains("plugin use"), "stderr: {stderr}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&bare).unwrap();
}

#[test]
fn plugin_use_records_the_default_container_in_the_project_file() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home4-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let dir = std::env::temp_dir().join(format!("dcdc-use-{}-{n}", std::process::id()));
    std::fs::create_dir_all(dir.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&dir, &["plugin", "use", "shell", "api"], Some(&home));
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = std::fs::read_to_string(dir.join("dcdc.toml")).unwrap_or_default();
    assert!(text.contains("[plugin.shell]"), "file: {text}");
    assert!(text.contains("container = \"api\""), "file: {text}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A fresh dcdc home syncs the default plugins, so this also covers
/// the barrier: a `dcdc plugin` call must finish the sync before it
/// acts.
#[test]
fn plugin_remove_refuses_a_default_plugin() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home5-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let dir = std::env::temp_dir().join(format!("dcdc-rm-{}-{n}", std::process::id()));
    std::fs::create_dir_all(dir.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&dir, &["plugin", "remove", "shell"], Some(&home));
    assert!(!out.status.success());
    let stderr = stderr(&out);
    assert!(stderr.contains("built-in plugin"), "stderr: {stderr}");
    // The sync installed it, and the refusal left it in place.
    let plugins = home.join(".dcdc").join("plugins");
    assert!(plugins.join("shell").join("shell.ts").is_file());
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn plugin_get_rejects_a_malformed_repository_reference() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home6-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let dir = std::env::temp_dir().join(format!("dcdc-get-{}-{n}", std::process::id()));
    std::fs::create_dir_all(dir.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&dir, &["plugin", "get", "not-a-repo"], Some(&home));
    assert!(!out.status.success());
    let stderr = stderr(&out);
    assert!(
        stderr.contains("invalid repository reference"),
        "stderr: {stderr}"
    );
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A fresh home makes `dcdc --help` wait out the sync, so the
/// Plugin Commands section is complete when it prints.
#[test]
fn help_lists_the_plugin_commands_before_the_command_list() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home8-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = run_dcdc_env(&home, &["--help"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let stdout = stdout(&out);
    assert!(stdout.contains("Plugin Commands:"), "stdout: {stdout}");
    assert!(
        stdout.contains("Run the target container's shell"),
        "stdout: {stdout}"
    );
    // The section leads the regular command list.
    let plugin = stdout.find("Plugin Commands:").unwrap();
    let commands = stdout.find("\nCommands:").unwrap();
    assert!(plugin < commands, "stdout: {stdout}");
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn help_for_a_plugin_command_shows_its_usage_and_example() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home9-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    // The alias resolves to the sub-command, whose own metadata is
    // what the help prints.
    let out = run_dcdc_env(&home, &["--help", "bash"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let stdout = stdout(&out);
    assert!(
        stdout.contains("Run the target container's shell"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("Usage:"), "stdout: {stdout}");
    assert!(
        stdout.contains("dcdc bash <command> [args...]"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("Example:"), "stdout: {stdout}");
    assert!(
        stdout.contains("dcdc bash echo \"Hello world!\""),
        "stdout: {stdout}"
    );
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn help_for_an_unknown_plugin_command_lists_the_available_ones() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home10-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = run_dcdc_env(&home, &["--help", "no-such-command"], Some(&home));
    assert!(!out.status.success());
    let stderr = stderr(&out);
    assert!(
        stderr.contains("no plugin sub-command named no-such-command"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("shell"), "stderr: {stderr}");
    std::fs::remove_dir_all(&home).unwrap();
}

/// A help flag after the command name is the command's own argument:
/// dcdc prints no help and the plugin receives the flag.
#[test]
fn a_help_flag_after_the_command_name_is_the_plugins_own() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home11-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let out = run_dcdc_env(&home, &["-c", "local", "bash", "--help"], Some(&home));
    assert!(!out.status.success());
    let stdout = stdout(&out);
    let stderr = stderr(&out);
    // The shell got the flag as a command line, not dcdc's help.
    assert!(stderr.contains("command not found"), "stderr: {stderr}");
    assert!(!stdout.contains("Plugin Commands"), "stdout: {stdout}");
    assert!(!stdout.contains("Usage:"), "stdout: {stdout}");
    std::fs::remove_dir_all(&home).unwrap();
}

/// Writes a one-file plugin into `dir`, whose sub-command echoes a
/// marker through the target shell.
fn plugin_echoing(base: &Path, name: &str, marker: &str) {
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("dup.ts"),
        &format!(
            r#"
export const description = "{name} plugin";
export default async (): Promise<number> => {{
  return dcdc.run(dcdc.shell, ["-c", "echo {marker}"]);
}};
"#
        ),
    )
    .unwrap();
}

/// Both the project and the home own a `dup` sub-command; the
/// project's must win inside the project, and the home's outside.
#[test]
fn a_project_plugin_shadows_a_home_plugin_of_the_same_name() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home12-{}-{n}", std::process::id()));
    let home_plugins = home.join(".dcdc").join("plugins");
    std::fs::create_dir_all(&home_plugins).unwrap();
    plugin_echoing(&home_plugins, "echohome", "from-home");

    let proj = std::env::temp_dir().join(format!("dcdc-proj12-{}-{n}", std::process::id()));
    let proj_plugins = proj.join(".dcdc").join("plugins");
    std::fs::create_dir_all(&proj_plugins).unwrap();
    plugin_echoing(&proj_plugins, "echoproj", "from-project");

    // Inside the project, the project plugin wins.
    let out = run_dcdc_env(&proj, &["-c", "local", "dup"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("from-project"),
        "stdout: {}",
        stdout(&out)
    );

    // Outside it, the home plugin answers.
    let other = std::env::temp_dir().join(format!("dcdc-out12-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&other).unwrap();
    let out = run_dcdc_env(&other, &["-c", "local", "dup"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("from-home"),
        "stdout: {}",
        stdout(&out)
    );
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
    std::fs::remove_dir_all(&other).unwrap();
}

/// `dcdc plugin list` reports one section per source, the
/// project's leading.
#[test]
fn plugin_list_shows_project_and_home_sections() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home13-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let proj = std::env::temp_dir().join(format!("dcdc-proj13-{}-{n}", std::process::id()));
    let proj_plugins = proj.join(".dcdc").join("plugins");
    std::fs::create_dir_all(&proj_plugins).unwrap();
    plugin_echoing(&proj_plugins, "echoproj", "x");

    let out = run_dcdc_env(&proj, &["plugin", "list"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let stdout = stdout(&out);
    let proj_section = format!("Installed plugins in {}", proj_plugins.display());
    let home_section = format!(
        "Installed plugins in {}",
        home.join(".dcdc").join("plugins").display()
    );
    assert!(stdout.contains(&proj_section), "stdout: {stdout}");
    assert!(stdout.contains(&home_section), "stdout: {stdout}");
    // The project section leads.
    assert!(
        stdout.find(&proj_section).unwrap() < stdout.find(&home_section).unwrap(),
        "stdout: {stdout}"
    );
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// `dcdc --help` inside a project lists the project's sub-commands
/// alongside the home's, a shadowed name once.
#[test]
fn help_lists_project_plugin_commands() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home14-{}-{n}", std::process::id()));
    let home_plugins = home.join(".dcdc").join("plugins");
    std::fs::create_dir_all(&home_plugins).unwrap();
    plugin_echoing(&home_plugins, "echohome", "x");

    let proj = std::env::temp_dir().join(format!("dcdc-proj14-{}-{n}", std::process::id()));
    let proj_plugins = proj.join(".dcdc").join("plugins");
    std::fs::create_dir_all(&proj_plugins).unwrap();
    plugin_echoing(&proj_plugins, "echoproj", "x");

    let out = run_dcdc_env(&proj, &["--help"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let stdout = stdout(&out);
    assert!(stdout.contains("Plugin Commands:"), "stdout: {stdout}");
    // The shadowed name appears once, with the project's
    // description; the home's shell sub-command does too.
    assert_eq!(stdout.matches("dup").count(), 1, "stdout: {stdout}");
    assert!(stdout.contains("echoproj plugin"), "stdout: {stdout}");
    assert!(stdout.contains("shell"), "stdout: {stdout}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// A repository qualifier names one plugin, so it bypasses the
/// project shadowing that a bare name obeys.
#[test]
fn a_repo_qualified_name_bypasses_project_shadowing() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home15-{}-{n}", std::process::id()));
    let home_plugins = home.join(".dcdc").join("plugins");
    // A downloaded repository version, with its marker, plus a
    // project plugin that shadows the bare name.
    let repo_dir = home_plugins.join("derek-shell-abcdef12");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(
        repo_dir.join("run.ts"),
        "export default async (): Promise<number> => {\n  return dcdc.run(dcdc.shell, [\"-c\", \"echo from-repo\"]);\n};\n",
    )
    .unwrap();
    let refs = home_plugins.join(".refs");
    std::fs::create_dir_all(&refs).unwrap();
    std::fs::write(refs.join("derek-shell"), "abcdef12\n").unwrap();

    let proj = std::env::temp_dir().join(format!("dcdc-proj15-{}-{n}", std::process::id()));
    let shim = proj.join(".dcdc").join("plugins").join("shim");
    std::fs::create_dir_all(&shim).unwrap();
    std::fs::write(
        shim.join("run.ts"),
        "export default async (): Promise<number> => {\n  return dcdc.run(dcdc.shell, [\"-c\", \"echo from-project\"]);\n};\n",
    )
    .unwrap();

    // The bare name obeys the layering.
    let out = run_dcdc_env(&proj, &["-c", "local", "run"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("from-project"),
        "stdout: {}",
        stdout(&out)
    );

    // The repository qualifier reaches the downloaded version.
    let out = run_dcdc_env(&proj, &["-c", "local", "derek/shell:run"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("from-repo"),
        "stdout: {}",
        stdout(&out)
    );
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// `default:` reaches the plugins dcdc syncs itself, even when a
/// project plugin shadows the bare name.
#[test]
fn a_default_qualified_name_bypasses_project_shadowing() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home16-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();

    let proj = std::env::temp_dir().join(format!("dcdc-proj16-{}-{n}", std::process::id()));
    let shim = proj.join(".dcdc").join("plugins").join("shim");
    std::fs::create_dir_all(&shim).unwrap();
    std::fs::write(
        shim.join("bash.ts"),
        r#"
export const name = "shim";
export const aliases = ["bash", "sh"];
export default async (args: string[]): Promise<number> => {
  return dcdc.run(dcdc.shell, ["-c", "echo shim-ran " + args.join(" ")]);
};
"#,
    )
    .unwrap();

    // The bare name goes to the project's shadow.
    let out = run_dcdc_env(&proj, &["-c", "local", "bash", "echo", "hi"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("shim-ran echo hi"),
        "stdout: {}",
        stdout(&out)
    );

    // The qualifier reaches the built-in instead.
    let out = run_dcdc_env(
        &proj,
        &["-c", "local", "default:bash", "echo", "hi"],
        Some(&home),
    );
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    let stdout = stdout(&out);
    assert!(stdout.contains("hi"), "stdout: {stdout}");
    assert!(!stdout.contains("shim-ran"), "stdout: {stdout}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// The project's own plugins can be called by the folder name of
/// the project's root; outside the project they are not installed.
#[test]
fn a_project_root_qualified_name_selects_the_projects_plugins() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home19-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();

    // The project's root folder name is the qualifier.
    let proj = std::env::temp_dir()
        .join(format!("dcdc-proj19-{}-{n}", std::process::id()))
        .join("myproj");
    let plug = proj.join(".dcdc").join("plugins").join("myplug");
    std::fs::create_dir_all(&plug).unwrap();
    std::fs::write(
        plug.join("tool.ts"),
        "export const description = \"Project tool sub-command\";\nexport default async (): Promise<number> => {\n  return dcdc.run(dcdc.shell, [\"-c\", \"echo from-project\"]);\n};\n",
    )
    .unwrap();

    // Inside the project, the qualifier reaches the project plugin.
    let out = run_dcdc_env(&proj, &["-c", "local", "myproj:tool"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("from-project"),
        "stdout: {}",
        stdout(&out)
    );

    // The name is also spelled in the help.
    let out = run_dcdc_env(&proj, &["--help", "myproj:tool"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("Project tool sub-command"),
        "stdout: {}",
        stdout(&out)
    );

    // A qualifier that is not the root name says what the name is.
    let out = run_dcdc_env(&proj, &["-c", "local", "nope:tool"], Some(&home));
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("not the project root's folder name (myproj)"),
        "stderr: {err}"
    );

    // Outside the project, its plugins are not installed.
    let outside = std::env::temp_dir().join(format!("dcdc-out19-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    let out = run_dcdc_env(&outside, &["-c", "local", "myproj:tool"], Some(&home));
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("no project plugins are installed"),
        "stderr: {err}"
    );
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
    std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("dcdc-proj19-{}-{n}", std::process::id())),
    )
    .unwrap();
}

/// `dcdc --help` of a qualified name shows the named plugin's own
/// metadata, not the shadowing one's.
#[test]
fn help_for_a_qualified_name_shows_the_named_plugin() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home17-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();

    let proj = std::env::temp_dir().join(format!("dcdc-proj17-{}-{n}", std::process::id()));
    let shim = proj.join(".dcdc").join("plugins").join("shim");
    std::fs::create_dir_all(&shim).unwrap();
    std::fs::write(
        shim.join("bash.ts"),
        r#"
export const name = "shim";
export const description = "Project shadow of the shell plugin";
export const aliases = ["bash", "sh"];
export default async (): Promise<number> => 0;
"#,
    )
    .unwrap();

    let out = run_dcdc_env(&proj, &["--help", "bash"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    assert!(
        stdout(&out).contains("Project shadow of the shell plugin"),
        "stdout: {}",
        stdout(&out)
    );

    let out = run_dcdc_env(&proj, &["--help", "default:bash"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let stdout = stdout(&out);
    assert!(
        stdout.contains("Run the target container's shell"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("Project shadow"), "stdout: {stdout}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// A qualified name that names nothing is an error: it never
/// falls through to the script list, with or without arguments.
#[test]
fn a_qualified_miss_is_an_error_not_a_script() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home18-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let proj = std::env::temp_dir().join(format!("dcdc-proj18-{}-{n}", std::process::id()));
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    for argv in [
        vec!["-c", "local", "default:nope"],
        vec!["-c", "local", "default:nope", "extra"],
        vec!["-c", "local", "nobody/nothing:cmd"],
    ] {
        let out = run_dcdc_env(&proj, &argv, Some(&home));
        assert!(!out.status.success(), "argv: {argv:?}");
        let stderr = stderr(&out);
        assert!(
            !stderr.contains("does not take arguments"),
            "argv: {argv:?} stderr: {stderr}"
        );
        assert!(
            !stderr.contains("script "),
            "argv: {argv:?} stderr: {stderr}"
        );
    }
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

#[test]
fn plugin_remove_of_an_uninstalled_repository_fails() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = std::env::temp_dir().join(format!("dcdc-home7-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let dir = std::env::temp_dir().join(format!("dcdc-rm2-{}-{n}", std::process::id()));
    std::fs::create_dir_all(dir.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&dir, &["plugin", "remove", "nobody/nothing"], Some(&home));
    assert!(!out.status.success());
    let stderr = stderr(&out);
    assert!(
        stderr.contains("no version of nobody/nothing is installed"),
        "stderr: {stderr}"
    );
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

// --- Overrides of dcdc's own commands ------------------------------

/// A throwaway directory under the system temp, unique per test.
fn tempdir(tag: &str, n: u64) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dcdc-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The plugins root of a home or project directory.
fn plugins_root(base: &Path) -> PathBuf {
    let plugins = base.join(".dcdc").join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    plugins
}

/// Writes one sub-command file of a plugin.
fn sub(plugin: &Path, file: &str, body: &str) {
    std::fs::create_dir_all(plugin).unwrap();
    std::fs::write(plugin.join(format!("{file}.ts")), body).unwrap();
}

/// A sub-command that echoes a marker plus its arguments, so a test
/// can tell which plugin ran.
fn echoer(marker: &str) -> String {
    format!(
        "export default async (args: string[]): Promise<number> => {{\n  \
         return dcdc.run(dcdc.shell, [\"-c\", \"echo {marker} \" + args.join(\" \")]);\n}};\n"
    )
}

/// A plugin claiming the name of a dcdc base command, with an
/// optional wrap flag.
fn claimer(plugins: &Path, plugin: &str, base: &str, wrap: bool, marker: &str) {
    let wrap_line = if wrap {
        "export const wrap_dcdc_command = true;\n"
    } else {
        ""
    };
    sub(
        &plugins.join(plugin),
        base,
        &format!(
            "export const name = \"{base}\";\n{wrap_line}{body}",
            body = echoer(marker)
        ),
    );
}

/// A SHA-stamped repository version, with its marker recording the
/// reference, so a qualified name can spell it back.
fn repo_plugin(
    home: &Path,
    key: &str,
    ref_: &str,
    sha: &str,
    file: &str,
    wrap: bool,
    marker: &str,
) {
    let plugins = plugins_root(home);
    let dir = plugins.join(format!("{key}-{sha}"));
    let wrap_line = if wrap {
        "export const wrap_dcdc_command = true;\n"
    } else {
        ""
    };
    sub(
        &dir,
        file,
        &format!(
            "export const name = \"{file}\";\n{wrap_line}{body}",
            body = echoer(marker)
        ),
    );
    let refs = plugins.join(".refs");
    std::fs::create_dir_all(&refs).unwrap();
    std::fs::write(refs.join(key), format!("{ref_}\n{sha}\n")).unwrap();
}

/// A plugin that claims a base command runs on the bare name, and
/// warns that it hides dcdc's own command.
#[test]
fn a_plugin_claiming_a_base_command_overrides_it_and_warns() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("ovr-home", n);
    claimer(&plugins_root(&home), "plover", "plugin", false, "plover");
    let proj = tempdir("ovr-proj", n);
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&proj, &["-c", "local", "plugin", "list"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    let so = stdout(&out);
    // The plugin wins the bare name; dcdc's own `plugin list` does
    // not run.
    assert!(so.contains("plover list"), "stdout: {so}");
    let se = stderr(&out);
    assert!(se.contains("overrides DCDC command plugin"), "stderr: {se}");
    assert!(se.contains("wrap_dcdc_command"), "stderr: {se}");

    // The other base command, `help`, is claimed the same way.
    claimer(&plugins_root(&home), "helpy", "help", false, "helpy");
    let out = run_dcdc_env(&proj, &["-c", "local", "help"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let se = stderr(&out);
    assert!(se.contains("overrides DCDC command help"), "stderr: {se}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// The wrap flag silences the warning, while the override stands.
#[test]
fn a_wrapping_plugin_overrides_without_a_warning() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("wrap-home", n);
    claimer(&plugins_root(&home), "wrapme", "plugin", true, "wrapme");
    let proj = tempdir("wrap-proj", n);
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&proj, &["-c", "local", "plugin", "list"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let so = stdout(&out);
    assert!(so.contains("wrapme list"), "stdout: {so}");
    // The override is silent now.
    let se = stderr(&out);
    assert!(!se.contains("overrides DCDC"), "stderr: {se}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// A project plugin claiming a base name beats a home plugin that
/// wraps the same name, and still warns, because it does not wrap.
#[test]
fn a_project_claim_beats_a_home_wrapper_and_still_warns() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("beat-home", n);
    claimer(&plugins_root(&home), "theirs", "plugin", true, "theirs");
    let proj = tempdir("beat-proj", n);
    claimer(&plugins_root(&proj), "mine", "plugin", false, "mine");

    let out = run_dcdc_env(&proj, &["-c", "local", "plugin", "list"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let so = stdout(&out);
    // The project's un-wrapped claim still shadows the home's
    // wrapped one.
    assert!(so.contains("mine list"), "stdout: {so}");
    assert!(!so.contains("theirs"), "stdout: {so}");
    // And it warns, because the winning plugin does not wrap.
    let se = stderr(&out);
    assert!(se.contains("overrides DCDC command plugin"), "stderr: {se}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// Among several home plugins claiming a name, the one that wraps
/// wins, with no warning because the name is not a base command.
#[test]
fn a_home_wrapper_breaks_a_tie_between_home_plugins() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("tie-home", n);
    sub(&plugins_root(&home).join("aaa"), "tool", &echoer("aaa"));
    sub(
        &plugins_root(&home).join("zzz"),
        "tool",
        &format!(
            "export const name = \"tool\";\nexport const wrap_dcdc_command = true;{}\n",
            echoer("zzz")
        ),
    );
    let proj = tempdir("tie-proj", n);
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&proj, &["-c", "local", "tool"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let so = stdout(&out);
    assert!(so.contains("zzz"), "stdout: {so}");
    assert!(!so.contains("aaa"), "stdout: {so}");
    // A non-base name never warns, even when it is wrapped.
    let se = stderr(&out);
    assert!(!se.contains("overrides DCDC"), "stderr: {se}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// Two wrapping plugins in one scope leave the bare name a
/// collision; only a fully namespaced, qualified call runs one.
#[test]
fn a_two_wrapper_collision_errors_and_the_qualified_name_works() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("coll-home", n);
    repo_plugin(
        &home,
        "derek-shell",
        "derek/shell",
        "abcd1234",
        "tool",
        true,
        "derek",
    );
    repo_plugin(
        &home,
        "jane-tools",
        "jane/tools",
        "ef01abcd",
        "tool",
        true,
        "jane",
    );
    let proj = tempdir("coll-proj", n);
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    // The bare name is refused and points at the qualified forms.
    let out = run_dcdc_env(&proj, &["-c", "local", "tool"], Some(&home));
    assert!(!out.status.success());
    let se = stderr(&out);
    assert!(se.contains("matches more than one plugin"), "stderr: {se}");
    assert!(se.contains("fully namespaced"), "stderr: {se}");

    // Each qualified name reaches exactly one plugin.
    let out = run_dcdc_env(&proj, &["-c", "local", "derek/shell:tool"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let so = stdout(&out);
    assert!(so.contains("derek"), "stdout: {so}");
    assert!(!so.contains("jane"), "stdout: {so}");

    let out = run_dcdc_env(&proj, &["-c", "local", "jane/tools:tool"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let so = stdout(&out);
    assert!(so.contains("jane"), "stdout: {so}");
    assert!(!so.contains("derek"), "stdout: {so}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

// --- The verbose mode -----------------------------------------------

/// The verbose mode names the file that runs, and the sub-commands
/// of the same name that the selection hides.
#[test]
fn a_verbose_run_reports_the_selected_sub_command_and_the_matches_it_hides() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("verb-home", n);
    let home_plugins = plugins_root(&home);
    plugin_echoing(&home_plugins, "echohome", "from-home");

    let proj = tempdir("verb-proj", n);
    let proj_plugins = proj.join(".dcdc").join("plugins");
    std::fs::create_dir_all(&proj_plugins).unwrap();
    plugin_echoing(&proj_plugins, "echoproj", "from-project");

    // Verbose: the selected file, and the home copy it shadows.
    let out = run_dcdc_env(&proj, &["-c", "local", "-v", "dup"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    let se = stderr(&out);
    assert!(se.contains("verbose:"), "stderr: {se}");
    let selected = proj_plugins.join("echoproj").join("dup.ts");
    let hidden = home_plugins.join("echohome").join("dup.ts");
    assert!(
        se.contains(&format!(
            "dup resolves to plugin echoproj (project) in {}",
            selected.display()
        )),
        "stderr: {se}"
    );
    assert!(
        se.contains("other sub-command(s) use the same name and were not selected"),
        "stderr: {se}"
    );
    assert!(
        se.contains(&format!("(home) in {}", hidden.display())),
        "stderr: {se}"
    );
    // The plugin's own output stays on stdout.
    assert!(
        stdout(&out).contains("from-project"),
        "stdout: {}",
        stdout(&out)
    );

    // Without the flag, the run is silent about the resolution.
    let out = run_dcdc_env(&proj, &["-c", "local", "dup"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("verbose"),
        "stderr: {}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// A sub-command with no rivals gets the selection line alone.
#[test]
fn a_verbose_run_of_a_solo_sub_command_names_only_itself() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("verb2-home", n);
    let dir = plugins_root(&home).join("solo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tool.ts"), echoer("tool-ran")).unwrap();
    let proj = tempdir("verb2-proj", n);
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&proj, &["-c", "local", "-v", "tool"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    let se = stderr(&out);
    let file = dir.join("tool.ts");
    assert!(
        se.contains(&format!(
            "tool resolves to plugin solo (home) in {}",
            file.display()
        )),
        "stderr: {se}"
    );
    // Nothing else matched, so there is no hidden list.
    assert!(!se.contains("were not selected"), "stderr: {se}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// The sub-command's own word list is passed through untouched, so a
/// plugin can accept the same `-c` and `-v` names for itself.
#[test]
fn a_sub_command_receives_the_flags_after_its_name() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("verb3-home", n);
    let dir = plugins_root(&home).join("solo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tool.ts"), echoer("tool-ran")).unwrap();
    let proj = tempdir("verb3-proj", n);
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    // The same names, placed after the sub-command name, reach the
    // plugin as its own arguments, and do not turn on dcdc's verbose
    // mode.
    for (argv, expect) in [
        (vec!["-c", "local", "tool", "-v", "x"], "tool-ran -v x"),
        (
            vec!["-c", "local", "tool", "--verbose", "x"],
            "tool-ran --verbose x",
        ),
        (vec!["-c", "local", "tool", "-c", "web"], "tool-ran -c web"),
    ] {
        let out = run_dcdc_env(&proj, &argv, Some(&home));
        assert!(
            out.status.success(),
            "argv: {argv:?}\nstdout: {}\nstderr: {}",
            stdout(&out),
            stderr(&out)
        );
        assert!(
            stdout(&out).contains(expect),
            "argv: {argv:?} expected {expect:?}, stdout: {}",
            stdout(&out)
        );
        // Not dcdc's verbose flag, so no resolution account is printed.
        assert!(
            !stderr(&out).contains("verbose"),
            "argv: {argv:?} stderr: {}",
            stderr(&out)
        );
    }
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// A dcdc flag placed before the sub-command name is dcdc's own and
/// stays out of the sub-command's arguments.
#[test]
fn a_leading_flag_is_dcdcs_and_stays_out_of_the_sub_command() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("verb4-home", n);
    let dir = plugins_root(&home).join("solo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tool.ts"), echoer("tool-ran")).unwrap();
    let proj = tempdir("verb4-proj", n);
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    // `-v` before the name turns on dcdc's verbose mode and is not a
    // plugin argument; `-c` before the name sets dcdc's container.
    let out = run_dcdc_env(&proj, &["-c", "local", "-v", "tool", "x"], Some(&home));
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
    assert!(
        stdout(&out).contains("tool-ran x"),
        "stdout: {}",
        stdout(&out)
    );
    assert!(!stdout(&out).contains("-v"), "stdout: {}", stdout(&out));
    assert!(
        stderr(&out).contains("verbose:"),
        "stderr: {}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}

/// `--help` of a claimed base command shows the claiming plugin's
/// help, not the one clap renders for dcdc's own command.
#[test]
fn help_for_a_claimed_base_command_shows_the_plugin() {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let home = tempdir("help-home", n);
    sub(
        &plugins_root(&home).join("plover"),
        "plugin",
        "export const name = \"plugin\";\nexport const description = \"Claims the plugin name\";\nexport default async (): Promise<number> => 0;\n",
    );
    let proj = tempdir("help-proj", n);
    std::fs::create_dir_all(proj.join(".dcdc")).unwrap();

    let out = run_dcdc_env(&proj, &["--help", "plugin"], Some(&home));
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let so = stdout(&out);
    // The plugin's own description, not clap's "Manage plugins".
    assert!(so.contains("Claims the plugin name"), "stdout: {so}");
    assert!(!so.contains("Manage plugins"), "stdout: {so}");
    std::fs::remove_dir_all(&home).unwrap();
    std::fs::remove_dir_all(&proj).unwrap();
}
