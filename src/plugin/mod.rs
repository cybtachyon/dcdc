//! Discovers and resolves the installed plugins.
//!
//! A plugin is a directory holding at least one sub-command `.ts`
//! file; no configuration file is required. The plugin is named
//! after its directory, and every sub-command's name, description,
//! version, and aliases are TypeScript exports of its file.
//!
//! Plugins load from two places: the nearest project's
//! `.dcdc/plugins`, first, and the dcdc home's `plugins`, second. A
//! sub-command owned by a project plugin shadows a home sub-command
//! of the same name.
//!
//! A sub-command can also be called through its plugin's name,
//! `qualifier:command`: `default` names the plugins dcdc syncs
//! itself, a qualifier with a slash names an installed repository,
//! and any other qualifier names the current project, by the folder
//! name of its root. A qualified call always reaches the named
//! plugin, so it is how you reach a sub-command another plugin has
//! shadowed.
//!
//! A sub-command can also claim the name of a dcdc base command
//! (`plugin` or `help`), or the name of another sub-command. A
//! claim on a base command overrides it: the bare word runs the
//! plugin, and dcdc warns on every run until the sub-command says
//! it means to, with `wrap_dcdc_command = true`. Among several
//! sub-commands of one scope claiming a name, a single one that
//! wraps wins; two or more, or none, leave the name a collision
//! that only a qualified call can make.

pub mod fetch;
pub mod repo;
pub mod runtime;
pub mod store;
pub mod sync;
pub mod tools;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cli;
use crate::config;
use crate::config::Loaded;
use crate::error::{Error, Result};

/// The place a plugin was discovered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The nearest project's `.dcdc/plugins`.
    Project,
    /// The dcdc home's `plugins` directory.
    Home,
}

/// A plugin installed in a plugins root.
#[derive(Debug, Clone)]
pub struct InstalledPlugin {
    /// The directory's name; for a downloaded repository, the
    /// `owner-repo` part, without the commit SHA.
    pub name: String,
    /// Directory inside a plugins root.
    pub dir: PathBuf,
    /// Where the plugin lives; project plugins shadow home ones.
    pub source: Source,
    /// Sub-command name to info, in name order.
    pub subcommands: BTreeMap<String, Subcommand>,
}

/// One sub-command of a plugin, implemented by a `.ts` file.
#[derive(Debug, Clone)]
pub struct Subcommand {
    /// The stem of the `.ts` file; the command name unless the
    /// file's TypeScript overrides it.
    pub name: String,
    pub path: PathBuf,
}

impl Subcommand {
    /// The sub-command's metadata, read from its TypeScript exports.
    pub fn meta(&self) -> runtime::SubcommandMeta {
        runtime::read_meta(&self.path)
    }
}

/// A name resolved to a concrete plugin sub-command.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub plugin: InstalledPlugin,
    pub subcommand: Subcommand,
}

/// The plugins directory inside the dcdc home.
pub fn plugins_root() -> Result<PathBuf> {
    Ok(crate::config::home_dir()?.join(".dcdc").join("plugins"))
}

/// The plugins directory of the nearest project, if the current
/// directory is inside one.
pub fn project_plugins_root() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let root = crate::root::find(&cwd).ok()?;
    Some(root.join(".dcdc").join("plugins"))
}

/// Discovers every installed plugin: the project's first, then the
/// home's, each in name order.
///
/// A directory counts as a plugin when it holds at least one
/// sub-command file; SDK and helper directories without one are
/// skipped.
pub fn discover() -> Result<Vec<InstalledPlugin>> {
    let mut plugins = Vec::new();
    if let Some(root) = project_plugins_root() {
        plugins.extend(discover_at(&root, Source::Project)?);
    }
    plugins.extend(discover_at(&plugins_root()?, Source::Home)?);
    Ok(plugins)
}

/// Discovers the plugins in one root, in name order.
fn discover_at(root: &Path, source: Source) -> Result<Vec<InstalledPlugin>> {
    let mut plugins = Vec::new();
    if !root.is_dir() {
        return Ok(plugins);
    }
    let mut dirs: Vec<PathBuf> = root
        .read_dir()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .filter(|p| {
            !p.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .starts_with('.')
        })
        .collect();
    dirs.sort();
    for dir in dirs {
        if let Some(plugin) = discover_one(&dir, source)? {
            plugins.push(plugin);
        }
    }
    Ok(plugins)
}

/// Discovers one plugin directory, if it holds a sub-command.
fn discover_one(dir: &Path, source: Source) -> Result<Option<InstalledPlugin>> {
    let mut subcommands = BTreeMap::new();
    for entry in dir.read_dir()? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || !is_subcommand_file(&path) {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        subcommands.insert(
            name.to_string(),
            Subcommand {
                name: name.to_string(),
                path,
            },
        );
    }
    if subcommands.is_empty() {
        return Ok(None);
    }
    Ok(Some(InstalledPlugin {
        name: plugin_name_from_dir(dir),
        dir: dir.to_path_buf(),
        source,
        subcommands,
    }))
}

/// The plugin name a directory carries: the directory's name, with
/// the commit SHA dropped for stored repository versions.
pub fn plugin_name_from_dir(dir: &Path) -> String {
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    match store::split_dir_name(dir_name) {
        Some((key, _)) => key,
        None => dir_name.to_string(),
    }
}

/// A discoverable sub-command file: a `.ts` file, excluding `.d.ts`
/// typings and dotfiles.
fn is_subcommand_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with('.') {
        return false;
    }
    path.extension().and_then(|e| e.to_str()) == Some("ts") && !name.ends_with(".d.ts")
}

/// Resolves a sub-command name or alias to its plugin and
/// sub-command.
///
/// A name can match a sub-command's file stem, its exported name,
/// or one of its exported aliases. A name matching several
/// sub-commands of one source is an error that lists the matches;
/// a name matching none is `PluginSubcommandNotFound`, which the
/// caller can use to fall back to the legacy scripts.
///
/// A match in a project plugin shadows a home sub-command of the
/// same name, so a name is ambiguous only within one source.
///
/// A name with a colon is a qualified call, `qualifier:command`,
/// that names one plugin instead of a bare name: `default` names
/// the plugins dcdc itself syncs, a GitHub repository reference
/// names an installed version of that repository, and any other
/// qualifier names the current project, by the folder name of its
/// root.
pub fn resolve(plugins: &[InstalledPlugin], name: &str) -> Result<Resolution> {
    if let Some((qualifier, cmd)) = name.rsplit_once(':') {
        return resolve_qualified(plugins, qualifier, cmd, name);
    }
    let matches: Vec<Resolution> = plugins
        .iter()
        .flat_map(|plugin| {
            plugin
                .subcommands
                .values()
                .filter(|sub| sub_matches(sub, name))
                .map(|sub| Resolution {
                    plugin: plugin.clone(),
                    subcommand: sub.clone(),
                })
        })
        .collect();
    // A project match wins over a home one of the same name; the
    // scoped set is what the ambiguity rule applies to.
    let scoped = if matches.iter().any(|m| m.plugin.source == Source::Project) {
        matches
            .into_iter()
            .filter(|m| m.plugin.source == Source::Project)
            .collect()
    } else {
        matches
    };
    match scoped.as_slice() {
        [] => Err(Error::PluginSubcommandNotFound {
            name: name.to_string(),
            available: labels(plugins),
        }),
        [res] => Ok(res.clone()),
        // A tie among the matches is broken by a wrapper: exactly
        // one sub-command that says it means to own the name wins
        // it. Several wrappers, or none, leave the name a
        // collision.
        many => {
            let wrappers: Vec<Resolution> = many
                .iter()
                .filter(|m| m.subcommand.meta().wrap_dcdc_command)
                .cloned()
                .collect();
            if let [winner] = wrappers.as_slice() {
                return Ok(winner.clone());
            }
            Err(Error::AmbiguousPluginSubcommand {
                name: name.to_string(),
                matches: many
                    .iter()
                    .map(|m| labeled(&m.plugin, &m.subcommand))
                    .collect(),
            })
        }
    }
}

/// A sub-command's display label, with the qualified name that
/// calls it, when that can be spelled.
fn labeled(plugin: &InstalledPlugin, sub: &Subcommand) -> String {
    let label = label(sub);
    match qualified_call(plugin, sub) {
        Some(call) => format!("{label} (call it {call})"),
        None => label,
    }
}

/// Whether a sub-command matches a name: its file stem, its
/// exported name, or one of its exported aliases.
fn sub_matches(sub: &Subcommand, name: &str) -> bool {
    let meta = sub.meta();
    sub.name == name || meta.name == name || meta.aliases.iter().any(|a| a == name)
}

/// Resolves a qualified call: a qualifier that names one plugin,
/// and the sub-command to run inside it.
///
/// `default` names the plugins dcdc itself syncs into the home, a
/// qualifier with a slash names a GitHub repository's installed
/// version, and any other qualifier names the current project, by
/// the folder name of its root. Each form looks only at one place,
/// so a qualified call is never shadowed by another plugin's name.
/// A project named `default` cannot use this form: the keyword
/// wins.
fn resolve_qualified(
    plugins: &[InstalledPlugin],
    qualifier: &str,
    cmd: &str,
    full: &str,
) -> Result<Resolution> {
    let scoped: Vec<&InstalledPlugin> = if qualifier == "default" {
        // The defaults are the home's plugins that are not a
        // downloaded repository version.
        let scoped: Vec<&InstalledPlugin> = plugins
            .iter()
            .filter(|p| p.source == Source::Home && !is_repo_version(p))
            .collect();
        if scoped.is_empty() {
            return Err(Error::BadUsage(
                "no default plugins are installed in the dcdc home".to_string(),
            ));
        }
        scoped
    } else if qualifier.contains('/') {
        let repo = repo::normalize(qualifier)?;
        let key = repo.key();
        let mut scoped: Vec<&InstalledPlugin> = plugins
            .iter()
            .filter(|p| p.source == Source::Home && p.name == key)
            .collect();
        if scoped.is_empty() {
            return Err(Error::BadUsage(format!(
                "no version of {qualifier} is installed; \
                 install it with `dcdc plugin get {qualifier}`"
            )));
        }
        // Several versions can coexist; the marker, in the plugins
        // root the versions live in, names the active one, so
        // prefer it when it is still installed. A marker pointing
        // at a removed version is ignored.
        if let Some(root) = scoped[0].dir.parent()
            && let Ok(Some(dir)) = store::installed_dir(root, &repo)
            && scoped.iter().any(|p| p.dir == dir)
            && scoped.len() > 1
        {
            scoped.retain(|p| p.dir == dir);
        }
        scoped
    } else {
        // The remaining form names the current project, by the
        // folder name of its root. A project plugin's directory is
        // `<root>/.dcdc/plugins/<plugin>`, so the root name the
        // plugins themselves carry is authoritative.
        let scoped = plugins
            .iter()
            .filter(|p| p.source == Source::Project)
            .collect::<Vec<_>>();
        let root_name = scoped
            .first()
            .and_then(|p| p.dir.parent().and_then(Path::parent).and_then(Path::parent))
            .and_then(|root| root.file_name())
            .and_then(|n| n.to_str());
        match root_name {
            Some(name) if name == qualifier => scoped,
            Some(name) => {
                return Err(Error::BadUsage(format!(
                    "{qualifier} is not the project root's folder name ({name})"
                )));
            }
            // The set holds no project plugins: say which case that
            // is.
            None => {
                return Err(Error::BadUsage(match project_plugins_root() {
                    Some(root) => format!(
                        "no plugins in the project's .dcdc/plugins: {}",
                        root.display()
                    ),
                    None => format!(
                        "{qualifier} is not the name of a project root, \
                         and no project plugins are installed"
                    ),
                }));
            }
        }
    };
    let matches: Vec<Resolution> = scoped
        .iter()
        .copied()
        .flat_map(|plugin| {
            plugin
                .subcommands
                .values()
                .filter(|sub| sub_matches(sub, cmd))
                .map(|sub| Resolution {
                    plugin: plugin.clone(),
                    subcommand: sub.clone(),
                })
        })
        .collect();
    match matches.as_slice() {
        [] => Err(Error::PluginSubcommandNotFound {
            name: full.to_string(),
            available: scoped.iter().copied().flat_map(plugin_labels).collect(),
        }),
        [res] => Ok(res.clone()),
        many => Err(Error::AmbiguousPluginSubcommand {
            name: full.to_string(),
            matches: many.iter().map(|r| label(&r.subcommand)).collect(),
        }),
    }
}

/// Whether a plugin is a downloaded repository version, by the
/// commit SHA on its directory name.
fn is_repo_version(plugin: &InstalledPlugin) -> bool {
    plugin
        .dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(store::split_dir_name)
        .is_some()
}

/// The qualified name that calls this sub-command, when it can be
/// spelled: the project's root folder name, `default`, or the
/// plugin's repository reference, with the sub-command's own name.
///
/// A repository version whose marker predates the reference, with
/// no reference recorded, has none to spell.
pub fn qualified_call(plugin: &InstalledPlugin, sub: &Subcommand) -> Option<String> {
    let cmd = sub.meta().name;
    let qualifier = match plugin.source {
        // The project plugin directory is `<root>/.dcdc/plugins/<p>`,
        // so the root folder name sits three parents up.
        Source::Project => plugin
            .dir
            .parent()?
            .parent()?
            .parent()?
            .file_name()?
            .to_str()?
            .to_string(),
        // A downloaded version is addressed by its repository
        // reference, when the install recorded one.
        Source::Home if is_repo_version(plugin) => {
            let root = plugin.dir.parent()?;
            store::current_ref(root, &plugin.name).ok()??
        }
        // A default plugin is addressed by the reserved keyword.
        Source::Home => "default".to_string(),
    };
    Some(format!("{qualifier}:{cmd}"))
}

/// Whether any sub-command of any installed plugin claims a name.
pub fn name_claimed(plugins: &[InstalledPlugin], name: &str) -> bool {
    plugins
        .iter()
        .any(|p| p.subcommands.values().any(|s| sub_matches(s, name)))
}

/// The dcdc base command a sub-command claims by name or alias, if
/// any. Running the sub-command overrides that command, and dcdc
/// warns about it on every run until the sub-command wraps.
pub fn overridden_base_command(sub: &Subcommand) -> Option<&'static str> {
    cli::BASE_COMMANDS
        .iter()
        .copied()
        .find(|cmd| sub_matches(sub, cmd))
}

/// Labels every sub-command of every plugin for display, in the
/// form `name (alias, alias) [plugin]`.
pub fn labels(plugins: &[InstalledPlugin]) -> Vec<String> {
    plugins.iter().flat_map(plugin_labels).collect()
}

/// Labels the sub-commands of one plugin for display.
fn plugin_labels(plugin: &InstalledPlugin) -> Vec<String> {
    let name = plugin.name.clone();
    plugin
        .subcommands
        .values()
        .map(move |s| format!("{} [{name}]", label(s)))
        .collect()
}

/// Renders one sub-command with its aliases, from its TypeScript
/// exports.
pub fn label(sub: &Subcommand) -> String {
    let meta = sub.meta();
    if meta.aliases.is_empty() {
        meta.name
    } else {
        format!("{} ({})", meta.name, meta.aliases.join(", "))
    }
}

/// Implements the `dcdc plugin` command group.
pub fn cmd(
    cmd: &crate::cli::PluginCmd,
    plugins: &[InstalledPlugin],
    loaded: &Loaded,
) -> Result<i32> {
    let Some(sub) = &cmd.command else {
        return Err(Error::BadUsage(
            "usage: dcdc plugin <list|get|use|remove>".to_string(),
        ));
    };
    match sub {
        crate::cli::PluginSub::List => list(plugins, loaded),
        crate::cli::PluginSub::Get { repo, branch } => get(repo, branch.as_deref()),
        crate::cli::PluginSub::Use { repo, container } => {
            use_plugin(repo, container.as_deref().unwrap_or("local"), loaded)
        }
        crate::cli::PluginSub::Remove { repo, sha } => remove_plugin(repo, sha.as_deref()),
        crate::cli::PluginSub::New { name, dir } => new_plugin(name, dir.as_deref()),
    }
}

/// Implements `dcdc plugin new <name> [--dir <dir>]`.
///
/// Writes the template files into the target directory, so a plugin
/// starts with a working skeleton: one sub-command that runs its
/// arguments in the shell, and an empty tool set. The plugin is the
/// directory itself; nothing else is required.
fn new_plugin(name: &str, dir: Option<&str>) -> Result<i32> {
    let name = name.trim().to_lowercase();
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        return Err(Error::BadUsage(
            "the plugin name must start with a lowercase letter".to_string(),
        ));
    }
    let target = std::env::current_dir()?.join(dir.unwrap_or(name.as_str()));
    if target.exists() {
        return Err(Error::BadUsage(format!(
            "{} already exists",
            target.display()
        )));
    }
    let project_plugin = project_plugins_root()
        .as_ref()
        .is_some_and(|root| target.starts_with(root));
    std::fs::create_dir_all(&target)?;
    std::fs::write(
        target.join("mise.toml"),
        "# Tools the plugin needs, with semantic version constraints.\n[tools]\n",
    )?;
    std::fs::write(
        target.join(format!("{name}.ts")),
        r#"
// The sub-command's metadata. The command name is the file's stem
// unless a `name` export overrides it; the version is shown by
// `dcdc plugin list`.
export const version = "0.1.0";
export const description = "A dcdc plugin";
export const aliases: string[] = [];

export default async (args: string[]): Promise<number> => {
  // The arguments the user typed after the command.
  const quoted = args.map((a) => `'${a.replaceAll("'", "'\\''")}'`).join(" ");
  return dcdc.run(dcdc.shell, ["-c", quoted]);
};
"#,
    )?;
    println!("Scaffolded plugin {name} in {}.", target.display());
    if project_plugin {
        println!("It is a project plugin and is active in this project.");
    } else {
        println!("Add it to your dcdc home, e.g.:");
        println!(
            "  cp -r {} {}",
            target.display(),
            plugins_root()?.join(name).display()
        );
    }
    Ok(0)
}

/// Implements `dcdc plugin get <repo> [--branch <ref>]`.
///
/// Downloads the repository, stores it under its commit SHA, and
/// records that SHA as the repository's active version.
fn get(repo_ref: &str, branch: Option<&str>) -> Result<i32> {
    let repo = repo::normalize(repo_ref)?;
    let root = plugins_root()?;
    let extracted = fetch::fetch(&repo, branch)?;
    let sha = store::install(&root, &repo, &extracted)?;
    let _ = std::fs::remove_dir_all(&extracted);
    let dir = root.join(format!("{}-{sha}", repo.key()));
    if looks_like_plugin(&dir) {
        let name = plugin_name_from_dir(&dir);
        println!(
            "Installed {}/{} at {sha} as plugin {name} in {}.",
            repo.owner,
            repo.repo,
            root.display()
        );
    } else {
        eprintln!(
            "dcdc: {}/{} at {sha} is stored in {}, but it is not a \
             dcdc plugin (no sub-command files), so it is not active.",
            repo.owner,
            repo.repo,
            root.display()
        );
    }
    Ok(0)
}

/// Whether a directory could hold a plugin, by the same rule the
/// discovery uses: at least one sub-command file.
fn looks_like_plugin(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.file_name())).any(|n| {
                let n = n.to_string_lossy();
                n.ends_with(".ts") && !n.ends_with(".d.ts") && !n.starts_with('.')
            })
        })
        .unwrap_or(false)
}

/// Implements `dcdc plugin use <plugin-or-repo> [container]`.
///
/// Saves the default container in the `dcdc.toml` that applies to
/// the current directory: the project-level file inside a project,
/// else the user-level one.
fn use_plugin(ref_or_name: &str, container: &str, loaded: &Loaded) -> Result<i32> {
    let name = plugin_name(ref_or_name)?;
    config::save_plugin(
        &loaded.path,
        &name,
        &config::PluginSettings {
            container: container.to_string(),
        },
    )?;
    println!(
        "Plugin {name} will run in {container} by default. \
         Recorded in {}.",
        loaded.path.display()
    );
    Ok(0)
}

/// Implements `dcdc plugin remove <plugin-or-repo> [--sha <sha>]`.
fn remove_plugin(ref_or_name: &str, sha: Option<&str>) -> Result<i32> {
    let root = plugins_root()?;
    if let Ok(repo) = repo::normalize(ref_or_name) {
        return remove_repo(&root, &repo, sha);
    }
    remove_by_name(&root, ref_or_name, sha)
}

/// Removes a repository's downloaded version, by its reference.
fn remove_repo(root: &Path, repo: &repo::Repo, sha: Option<&str>) -> Result<i32> {
    if sha.is_none() && store::installed_dir(root, repo)?.is_none() {
        return Err(Error::BadUsage(format!(
            "no version of {}/{} is installed",
            repo.owner, repo.repo
        )));
    }
    if let Some(sha) = sha {
        let dir = root.join(format!("{}-{sha}", repo.key()));
        if !dir.is_dir() {
            return Err(Error::BadUsage(format!(
                "no version {sha} of {}/{} is installed",
                repo.owner, repo.repo
            )));
        }
        std::fs::remove_dir_all(&dir)?;
        store::clear_marker(root, &repo.key(), sha)?;
        println!("Removed {}/{} at {sha}.", repo.owner, repo.repo);
        return Ok(0);
    }
    store::remove(root, repo)?;
    println!(
        "Removed {}/{} and all of its versions.",
        repo.owner, repo.repo
    );
    Ok(0)
}

/// Removes a plugin found by its directory name, when it is a
/// downloaded version.
///
/// Default plugins, installed from the binary, are managed by the
/// sync and cannot be removed.
fn remove_by_name(root: &Path, name: &str, sha: Option<&str>) -> Result<i32> {
    let plugins = discover()?;
    let matches: Vec<_> = plugins
        .iter()
        .filter(|p| p.name == name)
        .filter(|p| {
            sha.map(|s| {
                p.dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(store::split_dir_name)
                    .map(|(_, d)| d == s)
                    .unwrap_or(false)
            })
            .unwrap_or(true)
        })
        .collect();
    if matches.is_empty() {
        return Err(Error::BadUsage(format!(
            "no plugin named {name} is installed"
        )));
    }
    if matches.len() > 1 {
        return Err(Error::BadUsage(format!(
            "plugin {name} is installed {n} times; pass a repository reference or --sha",
            n = matches.len()
        )));
    }
    let plugin = &matches[0];
    // A project plugin is a plain directory the project owns; it is
    // removed wholesale, with nothing to clean up elsewhere.
    if plugin.source == Source::Project {
        std::fs::remove_dir_all(&plugin.dir)?;
        println!("Removed plugin {name} from {}.", plugin.dir.display());
        return Ok(0);
    }
    let Some(dir_name) = plugin.dir.file_name().and_then(|n| n.to_str()) else {
        return Err(Error::BadUsage(format!(
            "plugin {name} has no directory name"
        )));
    };
    let Some((key, sha)) = store::split_dir_name(dir_name) else {
        // Not SHA-stamped: a default plugin.
        return Err(Error::BadUsage(format!(
            "{name} is a built-in plugin installed by dcdc itself; \
             removing it would only delay the next update"
        )));
    };
    std::fs::remove_dir_all(&plugin.dir)?;
    store::clear_marker(root, &key, &sha)?;
    println!("Removed plugin {name} at {sha}.");
    Ok(0)
}

/// Resolves a `use` argument to a plugin name: a valid repository
/// reference names its plugin by owner-repo, anything else is a name.
fn plugin_name(ref_or_name: &str) -> Result<String> {
    match repo::normalize(ref_or_name) {
        Ok(repo) => Ok(repo.key()),
        Err(_) => Ok(ref_or_name.trim().to_string()),
    }
}

/// Implements `dcdc plugin list`.
///
/// One section per source that holds plugins: the project's, first,
/// then the home's.
fn list(plugins: &[InstalledPlugin], loaded: &Loaded) -> Result<i32> {
    let home_root = plugins_root()?;
    let project_root = project_plugins_root();
    if plugins.is_empty() {
        println!("No plugins installed in {}.", home_root.display());
        println!();
        println!("Install one with `dcdc plugin get <github-repo>`.");
        return Ok(0);
    }
    let mut first = true;
    for source in [Source::Project, Source::Home] {
        let section: Vec<&InstalledPlugin> =
            plugins.iter().filter(|p| p.source == source).collect();
        if section.is_empty() {
            continue;
        }
        if !first {
            println!();
        }
        first = false;
        let root = match source {
            Source::Project => project_root.as_ref().unwrap(),
            Source::Home => &home_root,
        };
        println!("Installed plugins in {}:", root.display());
        for plugin in section {
            let default = loaded
                .config
                .plugins
                .get(&plugin.name)
                .map(|s| s.container.as_str())
                .unwrap_or("none");
            println!("  {}  default: {default}", plugin.name);
            for sub in plugin.subcommands.values() {
                let meta = sub.meta();
                let label = if meta.aliases.is_empty() {
                    meta.name
                } else {
                    format!("{} ({})", meta.name, meta.aliases.join(", "))
                };
                println!("    {label}  {}", meta.version);
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    use crate::testutil;

    /// Builds a plugin directory with one sub-command file,
    /// returning its path. The caller removes the base.
    fn build_plugin(base: &Path, name: &str, file: &str) -> PathBuf {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{file}.ts")), "// sub\n").unwrap();
        dir
    }

    #[test]
    fn discovers_plugins_and_skips_directories_without_sub_commands() {
        let base = testutil::temp_subdir("plugin-discover");
        std::fs::create_dir_all(base.join("empty")).unwrap();
        // A typings-only directory is not a plugin.
        std::fs::create_dir_all(base.join("sdk")).unwrap();
        std::fs::write(
            base.join("sdk").join("dcdc.d.ts"),
            "declare const dcdc: unknown;\n",
        )
        .unwrap();
        // A configuration file alone is not a plugin either.
        std::fs::create_dir_all(base.join("toml-only")).unwrap();
        std::fs::write(
            base.join("toml-only").join("plugin.toml"),
            "name = \"toml-only\"\n",
        )
        .unwrap();
        let _ = build_plugin(&base, "shell", "shell");

        let plugins = discover_at(&base, Source::Home).unwrap();
        let names: Vec<&str> = plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["shell"], "names: {names:?}");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn resolves_names_and_aliases_from_typescript() {
        let base = testutil::temp_subdir("plugin-resolve");
        let dir = base.join("shell");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("shell.ts"),
            r#"
export const version = "1.0.0";
export const aliases = ["bash", "sh"];
export default async (): Promise<number> => 0;
"#,
        )
        .unwrap();
        let plugins = discover_at(&base, Source::Home).unwrap();

        let res = resolve(&plugins, "bash").unwrap();
        assert_eq!(res.plugin.name, "shell");
        assert_eq!(res.subcommand.name, "shell");

        let res = resolve(&plugins, "shell").unwrap();
        assert_eq!(res.subcommand.name, "shell");

        let err = resolve(&plugins, "nope").unwrap_err();
        assert!(
            matches!(err, Error::PluginSubcommandNotFound { .. }),
            "{err:?}"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn an_exported_name_resolves_instead_of_the_file_stem() {
        let base = testutil::temp_subdir("plugin-meta-name");
        let dir = base.join("shell");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("run.ts"),
            r#"
export const name = "execute";
export default async (): Promise<number> => 0;
"#,
        )
        .unwrap();
        let plugins = discover_at(&base, Source::Home).unwrap();

        let res = resolve(&plugins, "execute").unwrap();
        assert_eq!(res.subcommand.name, "run");

        let err = resolve(&plugins, "nope").unwrap_err();
        assert!(
            matches!(err, Error::PluginSubcommandNotFound { .. }),
            "{err:?}"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_name_owned_by_two_plugins_is_ambiguous() {
        let base = testutil::temp_subdir("plugin-ambiguous");
        for name in ["one", "two"] {
            let dir = base.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("dup.ts"), "// dup\n").unwrap();
        }
        let plugins = discover_at(&base, Source::Home).unwrap();
        let err = resolve(&plugins, "dup").unwrap_err();
        assert!(
            matches!(err, Error::AmbiguousPluginSubcommand { .. }),
            "{err:?}"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_project_sub_command_shadows_a_home_one_of_the_same_name() {
        let base = testutil::temp_subdir("plugin-shadow");
        let proj = base.join("proj");
        let home = base.join("home");
        // Both roots own a `dup` sub-command; the project's must win.
        build_plugin(&proj, "one", "dup");
        build_plugin(&home, "two", "dup");
        // A home-only name still resolves through the merge.
        build_plugin(&home, "three", "solo");
        let plugins: Vec<InstalledPlugin> = discover_at(&proj, Source::Project)
            .unwrap()
            .into_iter()
            .chain(discover_at(&home, Source::Home).unwrap())
            .collect();

        let res = resolve(&plugins, "dup").unwrap();
        assert_eq!(res.plugin.name, "one", "the project plugin must win");

        let res = resolve(&plugins, "solo").unwrap();
        assert_eq!(res.plugin.name, "three");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_default_qualifier_reaches_a_home_default_past_a_project_shadow() {
        let base = testutil::temp_subdir("plugin-qual-default");
        let proj = base.join("proj");
        let home = base.join("home");
        // Both roots own a plugin whose sub-command is `bash`; the
        // bare name resolves to the project's copy, the qualifier
        // must not care.
        build_plugin(&proj, "shell", "bash");
        build_plugin(&home, "shell", "bash");
        let plugins: Vec<InstalledPlugin> = discover_at(&proj, Source::Project)
            .unwrap()
            .into_iter()
            .chain(discover_at(&home, Source::Home).unwrap())
            .collect();

        let res = resolve(&plugins, "bash").unwrap();
        assert_eq!(res.plugin.source, Source::Project);

        let res = resolve(&plugins, "default:bash").unwrap();
        assert_eq!(res.plugin.source, Source::Home);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_repository_qualifier_selects_the_active_version() {
        let base = testutil::temp_subdir("plugin-qual-repo");
        let home = base.join("home");
        // Two installed versions of the same repository; the
        // marker names the active one.
        for sha in ["11111111", "22222222"] {
            let dir = home.join(format!("derek-shell-{sha}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("run.ts"), "// run\n").unwrap();
        }
        let refs = home.join(".refs");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("derek-shell"), "22222222\n").unwrap();
        let plugins = discover_at(&home, Source::Home).unwrap();

        let res = resolve(&plugins, "derek/shell:run").unwrap();
        assert_eq!(
            res.plugin.dir,
            home.join("derek-shell-22222222"),
            "the active version must win"
        );

        // A full URL spells the same qualifier.
        let res = resolve(&plugins, "https://github.com/derek/shell:run").unwrap();
        assert_eq!(res.plugin.dir, home.join("derek-shell-22222222"));

        // A qualifier for an uninstalled repository is a usage
        // error, not a script fallback.
        let err = resolve(&plugins, "nobody/nothing:run").unwrap_err();
        assert!(matches!(err, Error::BadUsage(_)), "{err:?}");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_qualified_miss_lists_the_named_plugins_sub_commands() {
        let base = testutil::temp_subdir("plugin-qual-miss");
        let home = base.join("home");
        build_plugin(&home, "shell", "bash");
        let plugins = discover_at(&home, Source::Home).unwrap();

        let err = resolve(&plugins, "default:nope").unwrap_err();
        assert!(
            matches!(
                err,
                Error::PluginSubcommandNotFound { ref available, .. }
                    if available == &["bash [shell]".to_string()]
            ),
            "{err:?}"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_project_root_qualifier_selects_the_projects_plugins() {
        let base = testutil::temp_subdir("plugin-qual-project");
        // The real layout, so the project root the plugins live in
        // carries the folder name the qualifier spells.
        let proj = base.join("myproj");
        let proj_plugins = proj.join(".dcdc").join("plugins");
        let home = base.join("home");
        let home_plugins = home.join(".dcdc").join("plugins");
        // The project's own plugin, and a home plugin with the
        // same sub-command name.
        build_plugin(&proj_plugins, "myplug", "tool");
        build_plugin(&home_plugins, "other", "tool");
        let plugins: Vec<InstalledPlugin> = discover_at(&proj_plugins, Source::Project)
            .unwrap()
            .into_iter()
            .chain(discover_at(&home_plugins, Source::Home).unwrap())
            .collect();

        // The project is named by its root's folder name.
        let res = resolve(&plugins, "myproj:tool").unwrap();
        assert_eq!(res.plugin.source, Source::Project);
        assert_eq!(res.plugin.name, "myplug");

        // A wrong name is a usage error that names the real root.
        let err = resolve(&plugins, "nope:tool").unwrap_err();
        assert!(
            matches!(err, Error::BadUsage(ref msg) if msg.contains("myproj")),
            "{err:?}"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    /// Writes a sub-command file with explicit exports.
    fn build_export(base: &Path, plugin: &str, file: &str, exports: &str) -> PathBuf {
        let dir = base.join(plugin);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{file}.ts"));
        std::fs::write(&path, exports).unwrap();
        path
    }

    #[test]
    fn a_sub_command_claiming_a_base_command_resolves() {
        let base = testutil::temp_subdir("plugin-claim");
        let home = base.join("home");
        // A sub-command named `plugin` claims the base command's
        // name; resolution sees it as any other name.
        build_export(
            &home,
            "plover",
            "plugin",
            "export const name = \"plugin\";\nexport default async (): Promise<number> => 0;\n",
        );
        let plugins = discover_at(&home, Source::Home).unwrap();
        let res = resolve(&plugins, "plugin").unwrap();
        assert_eq!(res.plugin.name, "plover");
        assert_eq!(overridden_base_command(&res.subcommand), Some("plugin"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn overridden_base_command_names_the_claimed_command() {
        let base = testutil::temp_subdir("plugin-override");
        let home = base.join("home");
        // By exported name, by alias, and a claim on none.
        let path = build_export(
            &home,
            "a",
            "x",
            "export const name = \"help\";\nexport default async (): Promise<number> => 0;\n",
        );
        let sub = Subcommand {
            name: "x".into(),
            path: path.clone(),
        };
        assert_eq!(overridden_base_command(&sub), Some("help"));
        let path = build_export(
            &home,
            "b",
            "y",
            "export const name = \"y\";\nexport const aliases = [\"help\"];\nexport default async (): Promise<number> => 0;\n",
        );
        let sub = Subcommand {
            name: "y".into(),
            path: path.clone(),
        };
        assert_eq!(overridden_base_command(&sub), Some("help"));
        let path = build_export(
            &home,
            "c",
            "z",
            "export const name = \"z\";\nexport default async (): Promise<number> => 0;\n",
        );
        let sub = Subcommand {
            name: "z".into(),
            path,
        };
        assert_eq!(overridden_base_command(&sub), None);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_single_wrapper_breaks_a_tie_within_one_scope() {
        let base = testutil::temp_subdir("plugin-wrap-tie");
        let home = base.join("home");
        // Two home plugins claim the name; only one says it means
        // to, and it wins.
        build_export(
            &home,
            "aaa",
            "tool",
            "export const name = \"tool\";\nexport default async (): Promise<number> => 0;\n",
        );
        build_export(
            &home,
            "zzz",
            "tool",
            "export const name = \"tool\";\nexport const wrap_dcdc_command = true;\nexport default async (): Promise<number> => 0;\n",
        );
        let plugins = discover_at(&home, Source::Home).unwrap();
        let res = resolve(&plugins, "tool").unwrap();
        assert_eq!(res.plugin.name, "zzz");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn two_wrappers_leave_the_tie_a_collision() {
        let base = testutil::temp_subdir("plugin-wrap-two");
        let home = base.join("home");
        // Both say they mean to: the bare name stays a collision,
        // and the error spells a qualified name for each.
        build_export(
            &home,
            "aaa",
            "tool",
            "export const name = \"tool\";\nexport const wrap_dcdc_command = true;\nexport default async (): Promise<number> => 0;\n",
        );
        build_export(
            &home,
            "zzz",
            "tool",
            "export const name = \"tool\";\nexport const wrap_dcdc_command = true;\nexport default async (): Promise<number> => 0;\n",
        );
        let plugins = discover_at(&home, Source::Home).unwrap();
        let err = resolve(&plugins, "tool").unwrap_err();
        assert!(
            matches!(
                err,
                Error::AmbiguousPluginSubcommand {
                    ref matches, ..
                } if matches.len() == 2
                    && matches
                        .iter()
                        .all(|m| m.contains("default:tool"))
            ),
            "{err:?}"
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_project_claim_beats_a_home_wrapper() {
        let base = testutil::temp_subdir("plugin-claim-proj");
        let proj = base.join("myproj");
        let proj_plugins = proj.join(".dcdc").join("plugins");
        let home = base.join("home");
        let home_plugins = home.join(".dcdc").join("plugins");
        // The project plugin claims the name without wrapping; the
        // home plugin claims it with wrapping. The project match
        // wins, because project plugins shadow home ones, full
        // stop.
        build_export(
            &proj_plugins,
            "mine",
            "tool",
            "export const name = \"tool\";\nexport default async (): Promise<number> => 0;\n",
        );
        build_export(
            &home_plugins,
            "theirs",
            "tool",
            "export const name = \"tool\";\nexport const wrap_dcdc_command = true;\nexport default async (): Promise<number> => 0;\n",
        );
        let plugins: Vec<InstalledPlugin> = discover_at(&proj_plugins, Source::Project)
            .unwrap()
            .into_iter()
            .chain(discover_at(&home_plugins, Source::Home).unwrap())
            .collect();
        let res = resolve(&plugins, "tool").unwrap();
        assert_eq!(res.plugin.source, Source::Project);
        assert_eq!(res.plugin.name, "mine");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn qualified_call_spells_each_plugin_kind() {
        let base = testutil::temp_subdir("plugin-qualified");
        // Project: the root's folder name.
        let proj = base.join("myproj");
        let proj_plugins = proj.join(".dcdc").join("plugins");
        let home = base.join("home");
        let home_plugins = home.join(".dcdc").join("plugins");
        build_export(
            &proj_plugins,
            "mine",
            "tool",
            "export default async (): Promise<number> => 0;\n",
        );
        // A default: the reserved keyword.
        build_export(
            &home_plugins,
            "plain",
            "tool",
            "export default async (): Promise<number> => 0;\n",
        );
        // Repository versions, one with a recorded reference and
        // one without, in separate directories and markers.
        for (key, sha, marker) in [
            ("derek-shell", "abcd1234", "derek/shell\nabcd1234\n"),
            ("jane-tools", "ef01abcd", "ef01abcd\n"),
        ] {
            let dir = home_plugins.join(format!("{key}-{sha}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("tool.ts"),
                "export default async (): Promise<number> => 0;\n",
            )
            .unwrap();
            let refs = home_plugins.join(".refs");
            std::fs::create_dir_all(&refs).unwrap();
            std::fs::write(refs.join(key), marker).unwrap();
        }
        let plugins: Vec<InstalledPlugin> = discover_at(&proj_plugins, Source::Project)
            .unwrap()
            .into_iter()
            .chain(discover_at(&home_plugins, Source::Home).unwrap())
            .collect();
        for plugin in &plugins {
            let sub = plugin.subcommands.values().next().unwrap();
            let call = qualified_call(plugin, sub);
            match plugin.name.as_str() {
                "mine" => assert_eq!(call.as_deref(), Some("myproj:tool")),
                "plain" => assert_eq!(call.as_deref(), Some("default:tool")),
                // The marker records the reference, so the spelling
                // is the repository the install came from.
                "derek-shell" => assert_eq!(call.as_deref(), Some("derek/shell:tool")),
                // An older install wrote no reference: nothing to
                // spell.
                "jane-tools" => assert_eq!(call.as_deref(), None),
                other => panic!("unexpected plugin {other}"),
            }
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_shadowed_name_is_ambiguous_only_within_its_source() {
        let base = testutil::temp_subdir("plugin-shadow-amb");
        let proj = base.join("proj");
        let home = base.join("home");
        // Two project sub-commands share the name, and a home one
        // does too; the ambiguity is scoped to the project.
        build_plugin(&proj, "one", "dup");
        build_plugin(&proj, "two", "dup");
        build_plugin(&home, "three", "dup");
        let plugins: Vec<InstalledPlugin> = discover_at(&proj, Source::Project)
            .unwrap()
            .into_iter()
            .chain(discover_at(&home, Source::Home).unwrap())
            .collect();

        let err = resolve(&plugins, "dup").unwrap_err();
        assert!(
            matches!(err, Error::AmbiguousPluginSubcommand { .. }),
            "{err:?}"
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}
