use std::fmt;
use std::io;
use std::path::PathBuf;

/// The result type used across dcdc, carrying `Error`.
pub type Result<T> = std::result::Result<T, Error>;

/// The single error type returned by every dcdc function.
///
/// Each variant names the failure precisely, so the printed message
/// tells the user what to fix.
#[derive(Debug)]
pub enum Error {
    /// NoProjectRoot: no `.dcdc` directory found in the start
    /// directory or any parent up to the filesystem root.
    NoProjectRoot(PathBuf),

    /// BadProjectName: the project directory has no usable name, so
    /// no compose-style container name can be built.
    BadProjectName(PathBuf),

    /// TooManyArgs: a script name was given arguments; scripts take
    /// none, plugin sub-commands do.
    TooManyArgs(String),

    /// ScriptNotFound: no loaded script has the requested name.
    ScriptNotFound {
        name: String,
        available: Vec<String>,
    },

    /// AmbiguousScript: the requested name matches scripts in more
    /// than one location.
    AmbiguousScript { name: String, matches: Vec<String> },

    /// NoService: a script subdirectory does not match any docker
    /// compose service in the project configuration.
    NoService(String),

    /// NoContainer: the compose service matches, but no running
    /// container was found for it.
    NoContainer { service: String, expected: String },

    /// DockerMissing: the docker binary is missing or not on the
    /// PATH.
    DockerMissing,

    /// DockerFailed: a docker subcommand exited with a failure.
    DockerFailed { command: String, message: String },

    /// NoHome: no home directory could be determined, so the
    /// user-level dcdc state directory cannot be located.
    NoHome,

    /// BadConfig: dcdc.toml exists but could not be parsed.
    BadConfig { path: PathBuf, message: String },

    /// PluginSubcommandNotFound: no installed plugin sub-command or
    /// alias matches the requested name.
    PluginSubcommandNotFound {
        name: String,
        available: Vec<String>,
    },

    /// AmbiguousPluginSubcommand: the requested name matches
    /// sub-commands of more than one plugin.
    AmbiguousPluginSubcommand { name: String, matches: Vec<String> },

    /// NoTarget: a plugin sub-command was invoked without a
    /// container, and none is configured either.
    NoTarget { plugin: String },

    /// BadUsage: a command shape the parser accepted but that is not
    /// meaningful.
    BadUsage(String),

    /// MissingValue: a flag that requires a value was given without
    /// one.
    MissingValue(String),

    /// InvalidRepoRef: a repository reference dcdc cannot read.
    InvalidRepoRef(String),

    /// DownloadFailed: fetching a plugin repository failed.
    DownloadFailed(String),

    /// BadArchive: a downloaded plugin archive did not hold the
    /// expected layout.
    BadArchive(String),

    /// PluginRuntime: the TypeScript of a plugin sub-command failed
    /// inside the runtime.
    PluginRuntime { name: String, message: String },

    /// Io: a filesystem or process failure from the standard
    /// library.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // `PathBuf` has no `Display` impl, so wrap it for formatting.
            Error::NoProjectRoot(dir) => {
                let d = dir.display();
                write!(f, "no .dcdc directory found in {d} or any parent")
            }
            Error::BadProjectName(dir) => {
                let d = dir.display();
                write!(f, "the project directory {d} has no usable name")
            }
            Error::TooManyArgs(name) => write!(
                f,
                "script {name} does not take arguments; use a plugin sub-command for that"
            ),
            Error::ScriptNotFound { name, available } => {
                writeln!(f, "script {name} not found")?;
                if !available.is_empty() {
                    writeln!(f, "available scripts:")?;
                    for script in available {
                        writeln!(f, "  {script}")?;
                    }
                }
                Ok(())
            }
            Error::AmbiguousScript { name, matches } => {
                writeln!(f, "script {name} matches more than one location:")?;
                for script in matches {
                    writeln!(f, "  {script}")?;
                }
                Ok(())
            }
            Error::NoService(service) => write!(
                f,
                "no docker compose service named {service} in the project configuration"
            ),
            Error::NoContainer { service, expected } => write!(
                f,
                "no running container named {expected} for service {service}; \
                 start it with `docker compose up`"
            ),
            Error::DockerMissing => {
                write!(f, "docker is not installed or is not on the PATH")
            }
            Error::DockerFailed { command, message } => {
                write!(f, "docker {command} failed: {message}")
            }
            Error::NoHome => write!(f, "no home directory found for dcdc user state"),
            Error::BadConfig { path, message } => {
                let p = path.display();
                write!(f, "cannot parse {p}: {message}")
            }
            Error::PluginSubcommandNotFound { name, available } => {
                writeln!(f, "sub-command {name} not found")?;
                if !available.is_empty() {
                    writeln!(f, "available plugin sub-commands:")?;
                    for item in available {
                        writeln!(f, "  {item}")?;
                    }
                } else {
                    writeln!(f, "no plugins are installed")?;
                    writeln!(
                        f,
                        "install one with `dcdc plugin get <github-repo>`, or run a script instead"
                    )?;
                }
                Ok(())
            }
            Error::AmbiguousPluginSubcommand { name, matches } => {
                writeln!(f, "sub-command {name} matches more than one plugin:")?;
                for item in matches {
                    writeln!(f, "  {item}")?;
                }
                // A same-scope collision stays a collision: the bare
                // name is refused, and a sub-command is reachable
                // only by its fully namespaced, qualified name.
                writeln!(
                    f,
                    "the bare name is a collision; call one of them by its \
                     fully namespaced name, like <plugin>:<command>"
                )
            }
            Error::NoTarget { plugin } => write!(
                f,
                "plugin {plugin} has no container; pass -c/--container or run \
                 `dcdc plugin use {plugin} [container]`"
            ),
            Error::BadUsage(message) => write!(f, "{message}"),
            Error::MissingValue(flag) => write!(f, "{flag} requires a value"),
            Error::InvalidRepoRef(input) => {
                write!(
                    f,
                    "invalid repository reference {input} \
                     (expected owner/repo on github.com)"
                )
            }
            Error::DownloadFailed(detail) => {
                write!(f, "download failed: {detail}")
            }
            Error::BadArchive(detail) => {
                write!(f, "bad plugin archive: {detail}")
            }
            Error::PluginRuntime { name, message } => {
                write!(f, "plugin {name} failed: {message}")
            }
            Error::Io(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}
