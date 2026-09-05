use std::process::Command;

use crate::error::Result;
use crate::script::Script;

/// Executes a script with Bash in the current working directory.
///
/// The working directory is inherited, so a local script runs exactly
/// where the user invoked dcdc, which may be a subdirectory of the
/// project. The path is passed as an argument, so the file itself
/// does not need the executable bit set.
pub fn run(script: &Script) -> Result<i32> {
    let status = Command::new("bash").arg(&script.path).status()?;
    // `code` is `None` only when bash itself was killed by a signal,
    // which is not a script failure worth reporting separately.
    Ok(status.code().unwrap_or(1))
}
