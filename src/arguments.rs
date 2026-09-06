//! dcdc's own command-line flags, and where they end.
//!
//! Two flags belong to dcdc itself rather than to the sub-command it
//! runs: `-c/--container`, which names the container a sub-command
//! runs in, and `-v/--verbose`, which turns on the resolution
//! account. An external sub-command swallows every word after its
//! name, so these flags can only be read from the front of the
//! command line, in the leading region before the sub-command name.
//!
//! That boundary is the sub-command name itself, the first bare word.
//! Everything from it on is the sub-command's own argument list,
//! returned word for word, so a plugin is free to declare and accept
//! the very same `-c` and `-v` for itself.

use crate::error::Error;

/// dcdc's own flags, read from the front of the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnFlags {
    /// The `-c/--container` value, or none when it was not given.
    pub container: Option<String>,
    /// Whether `-v/--verbose` was given.
    pub verbose: bool,
}

/// The leading run of the post-split arguments: the dash-prefixed
/// words ahead of the sub-command name, which is the first bare word.
///
/// Called on the output of [`split`], whose leading region holds only
/// foreign flags dcdc does not own, such as `-h` and `--version`; the
/// sub-command's own arguments, which may contain bare words, begin
/// after it.
pub fn leading(args: &[String]) -> &[String] {
    let end = args
        .iter()
        .position(|a| !a.starts_with('-'))
        .unwrap_or(args.len());
    &args[..end]
}

/// The dcdc flags at the front of the line, and the arguments behind
/// them.
///
/// Reads `-c/--container` and `-v/--verbose` from the leading region,
/// in any order, and drops them. Every other leading word, such as
/// `-h` and `--version`, which dcdc answers for itself elsewhere, is
/// kept. The leading region ends at the first bare word, the
/// sub-command name; that word and everything after it, including any
/// flags, is the sub-command's and is returned word for word, so a
/// plugin may accept the same `-c` and `-v` names for itself.
///
/// A leading `-c` or `--container` with no value is a usage error: it
/// names no sub-command to receive the rest, so it is a
/// [`Error::MissingValue`].
pub fn split(raw: &[String]) -> Result<(OwnFlags, Vec<String>), Error> {
    let mut flags = OwnFlags::default();
    let mut out = Vec::new();
    let mut i = 0;
    // A bare `-c` or `--container` takes its value from the next word,
    // even one that starts with a dash.
    let mut wants_value = false;
    while i < raw.len() {
        if wants_value {
            flags.container = Some(raw[i].clone());
            wants_value = false;
            i += 1;
            continue;
        }
        let word = raw[i].as_str();
        match word {
            "-c" | "--container" => {
                if i + 1 >= raw.len() {
                    return Err(Error::MissingValue("-c/--container".to_string()));
                }
                wants_value = true;
                i += 1;
            }
            _ if word.starts_with("--container=") => {
                flags.container = Some(word["--container=".len()..].to_string());
                i += 1;
            }
            "-v" | "--verbose" => {
                flags.verbose = true;
                i += 1;
            }
            // The sub-command name, a bare word; it and everything
            // after it belong to the sub-command.
            _ if !word.starts_with('-') => {
                out.extend_from_slice(&raw[i..]);
                break;
            }
            // A meta or foreign flag dcdc does not own; keep it and
            // keep reading the leading region.
            _ => {
                out.push(raw[i].clone());
                i += 1;
            }
        }
    }
    Ok((flags, out))
}

/// The dcdc help request in the leading flags, if any.
///
/// A `--help` or `-h` placed before the sub-command name is dcdc's
/// own and is answered by dcdc. Returns the command the help is for:
/// `None` for the general help, else the first bare word. A help flag
/// after a command name is that command's argument, so it is not
/// found here.
pub fn find_help(args: &[String]) -> Option<Option<String>> {
    let wants_help = leading(args).iter().any(|a| a == "--help" || a == "-h");
    if !wants_help {
        return None;
    }
    let target = args
        .iter()
        .position(|a| !a.starts_with('-'))
        .map(|i| args[i].clone());
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::{OwnFlags, find_help, leading, split};
    use crate::error::Error;

    /// Builds a `Vec<String>` from a slice of string literals.
    fn words(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Builds an `OwnFlags` for the split tests.
    fn own_flags(container: Option<&str>, verbose: bool) -> OwnFlags {
        OwnFlags {
            container: container.map(str::to_string),
            verbose,
        }
    }

    #[test]
    fn dcdcs_flags_are_read_from_the_front_only() {
        // Before the sub-command name the flags are dcdc's; after it
        // they belong to the sub-command and pass through untouched.
        let (flags, rest) = split(&words(&["-c", "local", "bash", "-c", "web", "echo"])).unwrap();
        assert_eq!(flags, own_flags(Some("local"), false));
        assert_eq!(rest, words(&["bash", "-c", "web", "echo"]));

        let (flags, rest) = split(&words(&["-v", "bash", "-v", "x"])).unwrap();
        assert_eq!(flags, own_flags(None, true));
        assert_eq!(rest, words(&["bash", "-v", "x"]));

        // The `--container=` form, and the flags in any order.
        let (flags, rest) = split(&words(&["-v", "--container=api", "bash"])).unwrap();
        assert_eq!(flags, own_flags(Some("api"), true));
        assert_eq!(rest, words(&["bash"]));
    }

    #[test]
    fn a_sub_command_with_no_leading_flags_passes_through_whole() {
        let (flags, rest) = split(&words(&["bash", "-c", "web", "--verbose", "x"])).unwrap();
        assert_eq!(flags, own_flags(None, false));
        assert_eq!(rest, words(&["bash", "-c", "web", "--verbose", "x"]));
    }

    #[test]
    fn foreign_leading_flags_are_left_in_place() {
        // `-h` and `--version` are not dcdc's to extract; they stay in
        // the leading region for dcdc to answer for itself elsewhere,
        // and a dcdc flag may still follow them.
        let (flags, rest) = split(&words(&["--help", "-c", "web", "bash"])).unwrap();
        assert_eq!(flags, own_flags(Some("web"), false));
        assert_eq!(rest, words(&["--help", "bash"]));
    }

    #[test]
    fn a_dangling_leading_container_is_a_missing_value() {
        // A `-c` with no value names no sub-command, so it is a
        // usage error, not a sub-command argument.
        let err = split(&words(&["-c"])).unwrap_err();
        assert!(
            matches!(err, Error::MissingValue(ref flag) if flag == "-c/--container"),
            "{err:?}"
        );
    }

    #[test]
    fn leading_stops_at_the_first_bare_word() {
        // On post-split output the leading run is the foreign flags,
        // and the sub-command name is the first bare word.
        assert_eq!(
            leading(&words(&["--help", "bash", "-x"])),
            words(&["--help"])
        );
        // No bare word at all: the whole line is leading.
        assert_eq!(leading(&words(&["-h", "-v"])), words(&["-h", "-v"]));
        assert_eq!(leading(&words(&[])), Vec::<String>::new());
    }

    #[test]
    fn find_help_reads_only_the_leading_region() {
        assert_eq!(find_help(&words(&["--help"])), Some(None));
        assert_eq!(find_help(&words(&["-h"])), Some(None));
        assert_eq!(
            find_help(&words(&["--help", "bash"])),
            Some(Some("bash".into()))
        );
        assert_eq!(
            find_help(&words(&["-h", "bash", "echo"])),
            Some(Some("bash".into()))
        );
        assert_eq!(
            find_help(&words(&["--help", "plugin"])),
            Some(Some("plugin".into()))
        );
        // A help flag after a command name is that command's own
        // argument, so it is not a dcdc help request.
        assert_eq!(find_help(&words(&["bash", "--help"])), None);
        assert_eq!(find_help(&words(&["bash", "-h"])), None);
        assert_eq!(find_help(&words(&["bash", "echo", "--help"])), None);
        assert_eq!(find_help(&words(&[])), None);
    }
}
