//! The arguments dcdc reads from the front of the command line.
//!
//! External sub-commands swallow all arguments, so dcdc's own are
//! read from the leading region: the dash words before the first
//! bare word, the sub-command name. Everything from that word on
//! belongs to the sub-command.

use crate::error::Error;

/// Explains option and flag arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Takes one value, like `-c/--container`.
    Option,
    /// Takes no value, like `-v/--verbose`.
    Flag,
}

/// The static description of one argument: its kind, its words, and
/// the help text shown in the general help.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spec {
    kind: Kind,
    /// The short word, like `-c`.
    word: &'static str,
    /// The long word, like `--container`.
    long_word: &'static str,
    /// The help text shown in the general help.
    help: &'static str,
}

/// Every argument dcdc owns, defined once, in the order the help
/// prints them: the options, then the flags, each alphabetical.
const SPECS: &[Spec] = &[
    Spec {
        kind: Kind::Option,
        word: "-c",
        long_word: "--container",
        help: "Run in the named container, or `local` for the host",
    },
    Spec {
        kind: Kind::Flag,
        word: "-h",
        long_word: "--help",
        help: "Print help",
    },
    Spec {
        kind: Kind::Flag,
        word: "-v",
        long_word: "--verbose",
        help: "Print plugin sub-command resolution details",
    },
    Spec {
        kind: Kind::Flag,
        word: "-V",
        long_word: "--version",
        help: "Print version information",
    },
];

/// The value an argument currently holds.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    /// Not set: a flag that is off, or an option with no value.
    Unset,
    /// A flag that is on.
    Flag,
    /// An option that has a value.
    Text(String),
}

/// One argument dcdc owns: its static spec, and the value it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Argument {
    spec: &'static Spec,
    value: Value,
}

impl Argument {
    /// The short word, like `-c`.
    pub fn word(&self) -> &str {
        self.spec.word
    }

    /// The long word, like `--container`.
    pub fn long_word(&self) -> &str {
        self.spec.long_word
    }

    /// The help text shown in the general help.
    pub fn help(&self) -> &str {
        self.spec.help
    }

    /// Whether the argument is an option or a flag.
    pub fn kind(&self) -> Kind {
        self.spec.kind
    }

    /// True if the word names the argument: its short word, its long
    /// word, or a long word with a `=value` form, for an option
    /// only. A flag with a value names no argument.
    pub fn matches(&self, word: &str) -> bool {
        if self.spec.word == word || self.spec.long_word == word {
            return true;
        }
        match self.spec.kind {
            Kind::Flag => false,
            Kind::Option => word.starts_with(&format!("{}=", self.spec.long_word)),
        }
    }

    /// Stores the value: an option takes it, a flag is marked
    /// present.
    pub fn set(&mut self, value: Option<&str>) {
        self.value = match (self.spec.kind, value) {
            (Kind::Flag, _) => Value::Flag,
            (Kind::Option, Some(value)) => Value::Text(value.to_string()),
            (Kind::Option, None) => Value::Unset,
        }
    }

    /// True if the argument is set: a flag that was seen, an option
    /// that has a value.
    pub fn is_set(&self) -> bool {
        !matches!(self.value, Value::Unset)
    }

    /// The value an option carries, or None for a flag or an unset
    /// option.
    pub fn value(&self) -> Option<&str> {
        match &self.value {
            Value::Text(text) => Some(text.as_str()),
            _ => None,
        }
    }
}

/// The arguments dcdc owns, in the order the help prints them: the
/// options, then the flags, each alphabetical.
fn owned() -> Vec<Argument> {
    SPECS
        .iter()
        .map(|spec| Argument {
            spec,
            value: Value::Unset,
        })
        .collect()
}

/// The words dcdc owns, like `-c/--container`, in the order the help
/// prints them.
pub fn owned_words() -> Vec<String> {
    owned()
        .iter()
        .map(|a| format!("{}/{}", a.word(), a.long_word()))
        .collect()
}

/// The arguments dcdc reads from the front of the line.
///
/// The value of every argument dcdc owns, in the order the help
/// prints them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnArguments(Vec<Argument>);
impl Default for OwnArguments {
    /// The arguments dcdc owns, all unset.
    fn default() -> Self {
        Self(owned())
    }
}
impl OwnArguments {
    /// The argument a word names, by the rule of `Argument::matches`.
    fn argument(&self, word: &str) -> Option<&Argument> {
        self.0.iter().find(|a| a.matches(word))
    }

    /// The argument a word names, for the split to set it.
    fn argument_mut(&mut self, word: &str) -> Option<&mut Argument> {
        self.0.iter_mut().find(|a| a.matches(word))
    }

    /// True if the argument named by the word is set: a flag that
    /// was seen, an option that has a value.
    pub fn is_set(&self, word: &str) -> bool {
        self.argument(word).is_some_and(Argument::is_set)
    }

    /// The value of the argument named by the word, or None if it is
    /// a flag or an unset option.
    pub fn value(&self, word: &str) -> Option<&str> {
        self.argument(word).and_then(Argument::value)
    }
}

/// Prints the `Options:` and `Flags:` sections of the general help.
///
/// The words and text come from the arguments themselves, so a new
/// argument documents itself: an option gets a `<VALUE>` placeholder
/// spelling its name, a flag does not.
pub fn print_argument_help(args: &OwnArguments) {
    let options: Vec<&Argument> = args.0.iter().filter(|a| a.kind() == Kind::Option).collect();
    let flags: Vec<&Argument> = args.0.iter().filter(|a| a.kind() == Kind::Flag).collect();
    let width = options
        .iter()
        .chain(&flags)
        .map(|a| rendered_arg(a).len())
        .max()
        .unwrap_or(0);
    let line = |a: &Argument| format!("  {:<width$}  {}", rendered_arg(a), a.help());
    if !options.is_empty() {
        println!("Options:");
        for a in options.iter().copied() {
            println!("{}", line(a));
        }
    }
    if !flags.is_empty() {
        println!("Flags:");
        for a in flags.iter().copied() {
            println!("{}", line(a));
        }
    }
}

/// The word of an argument as the help renders it: `-c,
/// --container`, plus a `<CONTAINER>` placeholder naming the value
/// an option takes.
fn rendered_arg(arg: &Argument) -> String {
    let base = format!("{}, {}", arg.word(), arg.long_word());
    match arg.kind() {
        Kind::Flag => base,
        Kind::Option => {
            let name = arg
                .long_word()
                .trim_start_matches("--")
                .to_uppercase()
                .replace('-', "_");
            format!("{base} <{name}>")
        }
    }
}

/// Splits dcdc arguments from the words following.
///
/// Drops leading arguments. An option takes a value, a flag is
/// boolean. The leading region ends at the first bare word: the
/// sub-command name.
pub fn split(raw: &[String]) -> Result<(OwnArguments, Vec<String>), Error> {
    let mut args = OwnArguments::default();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let word = raw[i].as_str();
        if !word.starts_with('-') {
            // The sub-command name; it and everything after it
            // belong to the sub-command.
            rest.extend_from_slice(&raw[i..]);
            break;
        }
        let Some(arg) = args.argument_mut(word) else {
            return Err(Error::UnknownArgument(word.to_string()));
        };
        match arg.kind() {
            Kind::Flag => {
                arg.set(None);
                i += 1;
            }
            Kind::Option => {
                let equals = format!("{}=", arg.long_word());
                if let Some(value) = word.strip_prefix(equals.as_str()) {
                    // The `--container=api` form: the value in the word.
                    arg.set(Some(value));
                    i += 1;
                } else {
                    // The bare form takes the next word, even one that
                    // starts with a dash.
                    if i + 1 >= raw.len() {
                        return Err(Error::MissingValue(format!(
                            "{}/{}",
                            arg.word(),
                            arg.long_word()
                        )));
                    }
                    arg.set(Some(&raw[i + 1]));
                    i += 2;
                }
            }
        }
    }
    Ok((args, rest))
}

#[cfg(test)]
mod tests {
    use super::{OwnArguments, split};
    use crate::error::Error;

    /// Builds a `Vec<String>` from a slice of string literals.
    fn words(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Builds an `OwnArguments` for the split tests, all unset except
    /// the arguments the test names, set the way the split sets them.
    fn own(container: Option<&str>, verbose: bool, help: bool, version: bool) -> OwnArguments {
        let mut args = OwnArguments::default();
        if let Some(container) = container {
            args.argument_mut("--container")
                .unwrap()
                .set(Some(container));
        }
        if verbose {
            args.argument_mut("--verbose").unwrap().set(None);
        }
        if help {
            args.argument_mut("--help").unwrap().set(None);
        }
        if version {
            args.argument_mut("--version").unwrap().set(None);
        }
        args
    }

    #[test]
    fn dcdcs_arguments_are_read_from_the_front_only() {
        // Before the sub-command name the flags are dcdc's; after it
        // they belong to the sub-command and pass through untouched.
        let (args, rest) = split(&words(&["-c", "local", "bash", "-c", "web", "echo"])).unwrap();
        assert_eq!(args, own(Some("local"), false, false, false));
        assert_eq!(rest, words(&["bash", "-c", "web", "echo"]));

        let (args, rest) = split(&words(&["-v", "bash", "-v", "x"])).unwrap();
        assert_eq!(args, own(None, true, false, false));
        assert_eq!(rest, words(&["bash", "-v", "x"]));

        // The `--container=` form, and the arguments in any order.
        let (args, rest) = split(&words(&["-v", "--container=api", "bash"])).unwrap();
        assert_eq!(args, own(Some("api"), true, false, false));
        assert_eq!(rest, words(&["bash"]));
    }

    #[test]
    fn a_sub_command_with_no_leading_arguments_passes_through_whole() {
        let (args, rest) = split(&words(&["bash", "-c", "web", "--verbose", "x"])).unwrap();
        assert_eq!(args, own(None, false, false, false));
        assert_eq!(rest, words(&["bash", "-c", "web", "--verbose", "x"]));
    }

    #[test]
    fn help_and_version_are_read_from_the_front() {
        // Like the other dcdc arguments, a help or version flag
        // before the sub-command name is dcdc's own.
        let (args, rest) = split(&words(&["--help", "-c", "web", "bash"])).unwrap();
        assert_eq!(args, own(Some("web"), false, true, false));
        assert_eq!(rest, words(&["bash"]));

        let (args, rest) = split(&words(&["-V"])).unwrap();
        assert_eq!(args, own(None, false, false, true));
        assert_eq!(rest, words(&[]));

        // After the sub-command name they belong to the sub-command.
        let (args, rest) = split(&words(&["bash", "--help", "-V"])).unwrap();
        assert_eq!(args, own(None, false, false, false));
        assert_eq!(rest, words(&["bash", "--help", "-V"]));
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
    fn an_unknown_leading_word_is_an_error() {
        // A dash word dcdc does not own, before the sub-command
        // word, is a usage error: it is no argument of any command.
        let err = split(&words(&["--bogus", "bash"])).unwrap_err();
        assert!(
            matches!(err, Error::UnknownArgument(ref word) if word == "--bogus"),
            "{err:?}"
        );
        let err = split(&words(&["-x"])).unwrap_err();
        assert!(
            matches!(err, Error::UnknownArgument(ref word) if word == "-x"),
            "{err:?}"
        );
    }

    #[test]
    fn a_flag_with_a_value_is_an_error() {
        // Only an option takes a `=value`; a flag with one is not a
        // dcdc argument.
        let err = split(&words(&["--verbose=on", "bash"])).unwrap_err();
        assert!(
            matches!(err, Error::UnknownArgument(ref word) if word == "--verbose=on"),
            "{err:?}"
        );
    }
}
