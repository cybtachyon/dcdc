mod docker;
mod error;
mod local;
mod root;
mod script;
#[cfg(test)]
mod testutil;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use owo_colors::OwoColorize;

use error::Result;

/// CLI entry point.
///
/// It keeps the exit code of the executed script, so a failing
/// script makes the CLI fail too, and maps every other failure to
/// exit code 1 with a message on stderr.
fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from((code & 0xff) as u8),
        Err(err) => {
            eprintln!("dcdc: {err}");
            ExitCode::from(1)
        }
    }
}

/// Dispatches on the command line.
///
/// No arguments lists the available scripts, `--help` prints the
/// usage, and a single argument names the script to run.
fn run() -> Result<i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    // Borrowed view for the match; the owning vec above keeps the
    // strings alive for as long as the borrows last.
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => list(),
        ["--help" | "-h"] => {
            help();
            Ok(0)
        }
        [name] => run_script(name),
        _ => Err(error::Error::Usage),
    }
}

/// Prints every discovered script with where it would run.
///
/// Listing is how the script discovery is visible without running
/// anything, which keeps the load step inspectable on its own.
fn list() -> Result<i32> {
    let cwd = env::current_dir()?;
    let root = root::find(&cwd)?;
    let cmd_dir = cmd_dir(&root);
    let scripts = script::load(&cmd_dir)?;

    if scripts.is_empty() {
        println!("No scripts found in {}.", cmd_dir.display());
        println!();
        println!("Place bash scripts under .dcdc/cmd to run them with dcdc.");
        return Ok(0);
    }

    println!("Scripts in {}:", cmd_dir.display());
    let width = scripts.iter().map(|s| s.name.len()).max().unwrap_or(0);
    for s in &scripts {
        let mode = match &s.subdir {
            None => "local".green().to_string(),
            Some(service) => format!("docker: {service}").yellow().to_string(),
        };
        println!("  {:<width$}  {mode}", s.name);
    }
    Ok(0)
}

/// Finds the named script and runs it, locally or inside
/// the matching docker compose service.
fn run_script(name: &str) -> Result<i32> {
    let cwd = env::current_dir()?;
    let root = root::find(&cwd)?;
    let scripts = script::load(&cmd_dir(&root))?;
    let script = script::select(&scripts, name)?;
    match &script.subdir {
        None => local::run(script),
        Some(service) => docker::run(script, service, &root),
    }
}

/// Returns the `.dcdc/cmd` directory of a project root.
fn cmd_dir(root: &Path) -> PathBuf {
    root.join(".dcdc").join("cmd")
}

/// Prints a short usage summary.
fn help() {
    println!("dcdc - Dcdc Compose Dev CLI");
    println!();
    println!("Usage: dcdc [script]");
    println!();
    println!("With no arguments, lists the bash scripts found under the nearest");
    println!(".dcdc/cmd directory. With a script name, runs it: scripts in the");
    println!("root cmd directory run locally in the current working directory,");
    println!("and scripts in a subdirectory of cmd run inside the matching docker");
    println!("compose service, when a container for that service is running.");
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_usage_error_names_the_expected_invocation() {
        assert_eq!(
            crate::error::Error::Usage.to_string(),
            "usage: dcdc [script]"
        );
    }
}
