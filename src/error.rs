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

    /// Usage: the command line did not match the expected shape.
    Usage,

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
            Error::Usage => write!(f, "usage: dcdc [script]"),
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
