//! The smoke test of the dcdc CLI.
//!
//! Builds dcdc in the profile smoke was built in, and runs the
//! binary against an example project under `examples/`, with
//! `DCDC_HOME` pointed at a throwaway directory so the run never
//! touches the real dcdc home.
//!
//! The steps, in order: the version, the bare listing, the plugin
//! listing, then every local script of the example. A script that
//! targets a docker compose service is skipped, since a docker
//! daemon is not guaranteed.
//!
//! Run it with `cargo run --bin smoke`, or
//! `cargo run --bin smoke -- --example <name>` for another example.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The example smoke runs when `--example` is not given.
const DEFAULT_EXAMPLE: &str = "next-js-example";

/// Builds dcdc and runs it against the example, reporting each step.
fn main() -> ExitCode {
    let example = match parse_example() {
        Ok(name) => name,
        Err(message) => {
            eprintln!("smoke: {message}");
            eprintln!();
            print_usage();
            return ExitCode::from(2);
        }
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));

    // A release build of smoke drives the release build of dcdc,
    // and a debug one the debug one: building the other profile
    // would add a long compile the user did not ask for. Not every
    // toolchain sets the `release` cfg, so the absence of debug
    // assertions is the reliable marker of an optimized build.
    let release = cfg!(release) || !cfg!(debug_assertions);
    let profile = if release { "release" } else { "debug" };

    println!("smoke: building dcdc ({profile})");
    let mut build = Command::new("cargo");
    build.arg("build");
    if release {
        build.arg("--release");
    }
    // The cargo home lives in the in-repo `.cargo` directory, per
    // the development notes, so the build stays inside the checkout.
    build.current_dir(repo);
    build.env("CARGO_HOME", repo.join(".cargo"));
    match build.status() {
        Ok(status) if status.success() => {}
        _ => {
            eprintln!("smoke: the dcdc build failed");
            return ExitCode::FAILURE;
        }
    }
    let bin = repo.join("target").join(profile).join("dcdc");
    if !bin.is_file() {
        eprintln!(
            "smoke: the build finished, but {} is not there",
            bin.display()
        );
        return ExitCode::FAILURE;
    }

    let example = match example_dir(repo, &example) {
        Ok(dir) => dir,
        Err(message) => {
            eprintln!("smoke: {message}");
            return ExitCode::FAILURE;
        }
    };

    // The throwaway dcdc home the steps run under: the default
    // plugin sync and the user-level configuration stay out of the
    // real one.
    let home = std::env::temp_dir().join(format!("dcdc-smoke-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&home) {
        eprintln!("smoke: cannot create the dcdc home {}: {e}", home.display());
        return ExitCode::FAILURE;
    }
    println!("smoke: example: {}", example.display());
    println!("smoke: dcdc home: {}", home.display());

    let mut steps: Vec<Vec<String>> = vec![
        vec!["--version".to_string()],
        Vec::new(),
        vec!["plugin".to_string(), "list".to_string()],
    ];
    steps.extend(local_scripts(&example).into_iter().map(|name| vec![name]));
    for args in &steps {
        if !run_step(&bin, &example, &home, args) {
            eprintln!(
                "smoke: the dcdc home is kept at {} for inspection",
                home.display()
            );
            return ExitCode::FAILURE;
        }
    }

    let _ = std::fs::remove_dir_all(&home);
    println!("smoke: ok");
    ExitCode::SUCCESS
}

/// Runs one dcdc step in the example directory, echoing its output.
///
/// The step runs with the throwaway home in force. Returns whether
/// the step exited successfully.
fn run_step(bin: &Path, example: &Path, home: &Path, args: &[String]) -> bool {
    let shown = if args.is_empty() {
        "dcdc (list)".to_string()
    } else {
        format!("dcdc {}", args.join(" "))
    };
    println!("smoke: {shown}");
    let output = match Command::new(bin)
        .current_dir(example)
        .args(args)
        .env("DCDC_HOME", home)
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            eprintln!("smoke: failed to start {shown}: {e}");
            return false;
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    eprint!("{stderr}");
    if output.status.success() {
        return true;
    }
    eprintln!(
        "smoke: FAILED: {shown} exited {}",
        output.status.code().unwrap_or(-1)
    );
    false
}

/// Resolves the requested example to a directory under `examples/`.
///
/// The example must be a dcdc project: a directory that holds a
/// `.dcdc` of its own. A name that matches nothing lists the
/// examples that do exist.
fn example_dir(repo: &Path, name: &str) -> Result<PathBuf, String> {
    let examples = repo.join("examples");
    let dir = examples.join(name);
    if !dir.is_dir() {
        let mut available: Vec<std::ffi::OsString> = std::fs::read_dir(&examples)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name())
                    .collect()
            })
            .unwrap_or_default();
        available.sort();
        if available.is_empty() {
            return Err(format!("no examples in {}", examples.display()));
        }
        let list = available
            .iter()
            .map(|n| n.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "no example named {name} in {} (available: {list})",
            examples.display()
        ));
    }
    if !dir.join(".dcdc").is_dir() {
        return Err(format!(
            "the example {name} is not a dcdc project: it has no .dcdc directory"
        ));
    }
    Ok(dir)
}

/// The names of the example's local scripts: the `.sh` and
/// extensionless files in the top level of `.dcdc/cmd`, in name
/// order.
///
/// A subdirectory of `cmd` targets a docker compose service, which
/// the smoke test does not exercise.
fn local_scripts(example: &Path) -> Vec<String> {
    let cmd = example.join(".dcdc").join("cmd");
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&cmd) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') || !path.is_file() {
                continue;
            }
            let extension = path.extension().and_then(|e| e.to_str());
            if !matches!(extension, Some("sh") | None) {
                continue;
            }
            // dcdc drops a .sh extension from the name.
            names.push(name.strip_suffix(".sh").unwrap_or(name).to_string());
        }
    }
    names.sort();
    names
}

/// Reads the example name from the command line.
fn parse_example() -> Result<String, String> {
    let mut example = DEFAULT_EXAMPLE.to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "--example" => {
                example = args
                    .next()
                    .filter(|v| !v.is_empty())
                    .ok_or("--example requires a value")?;
            }
            other if other.starts_with("--example=") => {
                let value = &other["--example=".len()..];
                if value.is_empty() {
                    return Err("--example requires a value".to_string());
                }
                example = value.to_string();
            }
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    if example.trim().is_empty() {
        return Err("--example requires a value".to_string());
    }
    Ok(example)
}

/// Prints the usage line and the description of each flag.
fn print_usage() {
    println!("Usage: smoke [--example <name>]");
    println!();
    println!("  --example <name>  The example to run, a directory under examples/");
    println!("                    (default: {DEFAULT_EXAMPLE})");
    println!("  -h, --help        Print this help");
}
