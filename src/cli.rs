//! The command line surface.
//!
//! `dcdc` keeps two dispatch lanes. Named commands, like `dcdc
//! plugin`, parse structurally. Everything else is an external
//! sub-command: the first word is a plugin sub-command (or alias) or
//! a legacy script name, and the rest of the words are its
//! arguments.

use clap::{Parser, Subcommand};

/// The names of dcdc's own commands: the named sub-commands and the
/// auto `help` sub-command clap renders. A plugin sub-command can
/// claim one of these names and override the command, and dcdc
/// warns about it when the plugin runs.
pub const BASE_COMMANDS: [&str; 2] = ["plugin", "help"];

#[derive(Debug, Parser)]
#[command(name = "dcdc", about = "⎓⎓dcdc Compose Dev CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage plugins.
    Plugin(PluginCmd),

    /// A plugin sub-command (or alias) or a script name, followed
    /// by its arguments.
    ///
    /// `-c/--container` and `-v/--verbose` are dcdc's own flags, read
    /// from the front of the line before the sub-command name; clap
    /// cannot mix a declared option with an external sub-command, so
    /// everything after the name is passed to it word for word, and
    /// it may accept those same names for itself.
    #[command(external_subcommand)]
    Sub(Vec<String>),
}

#[derive(Debug, Parser)]
pub struct PluginCmd {
    #[command(subcommand)]
    pub command: Option<PluginSub>,
}

#[derive(Debug, Subcommand)]
pub enum PluginSub {
    /// List installed plugins and their sub-commands.
    List,

    /// Download and install a plugin from a GitHub repository.
    Get {
        /// Repository as `owner/repo`, with or without a scheme.
        repo: String,
        /// Branch or tag to fetch; the repository's default branch
        /// when omitted.
        #[arg(long)]
        branch: Option<String>,
    },

    /// Activate a plugin and assign its default container.
    Use {
        /// Plugin name, or a repository reference like `get`.
        repo: String,
        /// Default container; defaults to `local`.
        container: Option<String>,
    },

    /// Remove an installed plugin.
    Remove {
        /// Plugin name, or a repository reference like `get`.
        repo: String,
        /// Remove only this version, when the plugin is stored with
        /// a commit SHA.
        #[arg(long)]
        sha: Option<String>,
    },

    /// Scaffold a new plugin directory with the template files.
    New {
        /// The plugin name, in lowercase.
        name: String,
        /// The directory to create; the name, in the current
        /// directory, when omitted.
        #[arg(long)]
        dir: Option<String>,
    },
}
