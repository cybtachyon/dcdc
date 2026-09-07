mod arguments;
mod cli;
mod config;
mod docker;
mod error;
mod local;
mod plugin;
mod root;
mod script;
#[cfg(test)]
mod testutil;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use owo_colors::OwoColorize;

use error::{Error, Result};

/// CLI entry point.
///
/// Keeps the exit code of the executed script or plugin
/// sub-command, so a failing run makes the CLI fail too, and maps
/// every other failure to exit code 1 with a message on stderr.
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
/// No arguments lists the available scripts and plugin sub-commands.
/// A named command, like `dcdc plugin`, is handled structurally, and
/// anything else is a sub-command invocation: a plugin sub-command or
/// alias, or a legacy script.
fn run() -> Result<i32> {
    let raw: Vec<String> = env::args().skip(1).collect();
    // dcdc's own arguments, -c/--container, -v/--verbose,
    // -h/--help, and -V/--version, are read from the front of the
    // line. Clap cannot mix a declared option with an external
    // sub-command, so the split is what lets dcdc and a
    // sub-command use the same flag names: dcdc reads them before
    // the sub-command name, the sub-command the rest.
    let (own, args) = arguments::split(&raw)?;
    // An external sub-command forbids declared global options, so
    // clap would refuse a --version flag; answer it here instead.
    // Only a `--version` in the leading region is dcdc's; one after
    // the sub-command name is the sub-command's own argument.
    if own.is_set("--version") {
        println!("dcdc {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    // The post-update sync brings the embedded default plugins into
    // the dcdc home, in the background: ordinary commands never
    // wait for it. A failure is a warning, not an error: a stale or
    // missing default set still leaves the CLI usable.
    let mut sync = match plugin::sync::start_background() {
        Ok(sync) => sync,
        Err(e) => {
            eprintln!("dcdc: warning: default plugin sync failed: {e}");
            plugin::sync::BackgroundSync::none()
        }
    };
    // A help flag in the leading region is dcdc's own: it answers
    // it, including the plugin sections clap cannot render. The
    // target is the sub-command word, when there is one; a help
    // flag after a command name belongs to that command, so it
    // flows through to it untouched.
    if own.is_set("--help") {
        return handle_dcdc_help(args.first().cloned(), &mut sync, &own);
    }
    let cwd = env::current_dir()?;
    let mut plugins = plugin::discover()?;
    let loaded = config::load_for(&cwd)?;

    // A plugin sub-command can claim the name of a dcdc base
    // command, and the claim overrides the command: the bare word
    // runs the plugin, not dcdc's own command. The claim is checked
    // against the installed set, waiting out a running sync,
    // because a plugin the sync is still installing would claim the
    // name too.
    if let Some(name) = args.first().map(String::as_str)
        && cli::BASE_COMMANDS.contains(&name)
        && (plugin::name_claimed(&plugins, name)
            || sync_pending()
            || sync.is_running()
            || plugins.is_empty())
    {
        if !plugin::name_claimed(&plugins, name) {
            plugins = wait_and_rediscover(&mut sync)?;
        }
        if plugin::name_claimed(&plugins, name) {
            let rest = &args[1..];
            return dispatch_subcommand(name, rest, &own, &plugins, &loaded, &cwd, &mut sync);
        }
    }

    // parse_from skips the first element as the program name, so a
    // placeholder carries it and the real arguments start at one.
    let mut argv = vec!["dcdc".to_string()];
    argv.extend(args);
    let parsed = cli::Cli::parse_from(&argv);

    match &parsed.command {
        Some(cli::Command::Plugin(cmd)) => {
            // The `dcdc plugin` commands act on the installed set,
            // so they must wait for the sync to finish; the wait
            // shows a progress bar while it does.
            let names = sync.wait()?;
            if !names.is_empty() {
                eprintln!(
                    "dcdc: installed {} default plugin(s) to {}",
                    names.len(),
                    plugin::plugins_root()?.display()
                );
            }
            // The sync may have just written plugins, so the set is
            // re-read now that it is guaranteed complete.
            let plugins = plugin::discover()?;
            plugin::cmd(cmd, &plugins, &loaded)
        }
        Some(cli::Command::Sub(args)) => match args.as_slice() {
            [] => {
                // The bare list never waits: the sync keeps running
                // in the background, and the next command re-reads.
                let _ = sync;
                list(&plugins, &loaded)
            }
            [name, rest @ ..] => {
                dispatch_subcommand(name, rest, &own, &plugins, &loaded, &cwd, &mut sync)
            }
        },
        None => list(&plugins, &loaded),
    }
}

/// Answers a dcdc-level help request: the general help, the help of
/// a named command, or the help of a plugin sub-command.
///
/// The plugin sections describe the installed set, so a pending
/// sync is waited out before rendering, like the `plugin` commands
/// do.
fn handle_dcdc_help(
    target: Option<String>,
    sync: &mut plugin::sync::BackgroundSync,
    own: &arguments::OwnArguments,
) -> Result<i32> {
    let mut plugins = plugin::discover()?;
    // The set may predate a sync that is still running or has just
    // finished, so a pending or running sync, or an empty set, makes
    // a re-read warranted; a settled, non-empty one is left alone.
    if sync_pending() || sync.is_running() || plugins.is_empty() {
        plugins = wait_and_rediscover(sync)?;
    } else {
        let _ = sync;
    }
    match target {
        None => {
            print_general_help(own, &plugins);
            Ok(0)
        }
        Some(cmd) => {
            // The named dcdc commands keep the help clap renders for
            // them, unless a plugin has claimed the name.
            if cli::BASE_COMMANDS.contains(&cmd.as_str()) && !plugin::name_claimed(&plugins, &cmd) {
                let argv = vec!["dcdc".to_string(), cmd, "--help".to_string()];
                let _ = cli::Cli::parse_from(&argv);
                unreachable!("clap exits the process when it prints help")
            }
            print_plugin_subcommand_help(&plugins, &cmd)
        }
    }
}

/// Prints the general help.
///
/// Clap renders the fixed part, and this adds the `Plugin Commands`
/// section it cannot render because the sub-commands are not known
/// at compile time. The section lists every sub-command's name and
/// description and leads the regular command list, so the
/// frequently used plugin commands come first.
///
/// A name owned by a project sub-command and a home one is listed
/// once under the shadowing project plugin.
///
/// The `Options:` and `Flags:` sections come from the arguments
/// module, so a new argument documents itself.
fn print_general_help(own: &arguments::OwnArguments, plugins: &[plugin::InstalledPlugin]) {
    println!("dcdc {}  ⎓⎓Dcdc Compose Dev CLI", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Usage: dcdc [OPTIONS] [COMMAND]");

    // The set is project-first, so the first occurrence of a name
    // is its shadowing owner.
    let mut commands: Vec<(String, String)> = Vec::new();
    for plugin in plugins {
        for sub in plugin.subcommands.values() {
            let meta = sub.meta();
            if !commands.iter().any(|(n, _)| *n == meta.name) {
                commands.push((meta.name, meta.description));
            }
        }
    }
    commands.sort();
    if !commands.is_empty() {
        println!();
        println!("Plugin Commands:");
        let width = commands.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        for (name, description) in &commands {
            println!("  {:<width$}  {description}", name);
        }
    }

    println!();
    println!("Commands:");
    println!("  plugin   Manage plugins.");
    println!("  help     Print this message or the help of the given subcommand(s)");
    println!();
    arguments::print_argument_help(own);
}

/// Prints the help of one plugin sub-command: its description, and
/// its `usage` and `example` exports, when the plugin provides
/// them.
///
/// A name matching nothing or several sub-commands is an error that
/// lists the matches, and exits one.
fn print_plugin_subcommand_help(plugins: &[plugin::InstalledPlugin], cmd: &str) -> Result<i32> {
    let res = match plugin::resolve(plugins, cmd) {
        Ok(res) => res,
        Err(Error::AmbiguousPluginSubcommand { matches, .. }) => {
            eprintln!("dcdc: {cmd} matches more than one plugin sub-command:");
            for m in matches {
                eprintln!("  {m}");
            }
            return Ok(1);
        }
        Err(Error::PluginSubcommandNotFound { name, available }) => {
            // The error carries the sub-commands of the plugin the
            // name pointed at, which is the useful list for a
            // qualified miss.
            eprintln!("dcdc: no plugin sub-command named {name}");
            if !available.is_empty() {
                eprintln!("available plugin sub-commands:");
                for l in available {
                    eprintln!("  {l}");
                }
            }
            return Ok(1);
        }
        Err(e) => {
            eprintln!("dcdc: {e}");
            return Ok(1);
        }
    };
    let meta = res.subcommand.meta();
    if meta.description.is_empty() {
        println!("{}", meta.name);
    } else {
        println!("{}  {}", meta.name, meta.description);
    }
    if !meta.usage.is_empty() {
        println!();
        println!("Usage:");
        println!("  {}", meta.usage);
    }
    if !meta.example.is_empty() {
        println!();
        println!("Example:");
        println!("  {}", meta.example);
    }
    Ok(0)
}

/// Whether the default plugin sync is not yet complete, so a
/// plugin-shaped command is worth waiting for it.
fn sync_pending() -> bool {
    !plugin::sync::is_fresh(&plugin::plugins_root().unwrap_or_default())
}

/// Waits for the default plugin sync to settle, then re-reads the
/// installed set.
///
/// A pending sync is waited out with a progress bar; one that has
/// just finished is a no-op. Prints a note when this process
/// installed the default set.
fn wait_and_rediscover(
    sync: &mut plugin::sync::BackgroundSync,
) -> Result<Vec<plugin::InstalledPlugin>> {
    let names = sync.wait()?;
    if !names.is_empty() {
        eprintln!(
            "dcdc: installed {} default plugin(s) to {}",
            names.len(),
            plugin::plugins_root()?.display()
        );
    }
    plugin::discover()
}

/// Runs a sub-command invocation: a plugin sub-command when the name
/// resolves, otherwise a legacy script.
///
/// A name that resolves to nothing may still be a plugin the
/// post-update sync has not finished installing, so a pending sync is
/// waited out once and the set re-read before falling back to a
/// script.
///
/// A resolved sub-command that claims the name of a dcdc base
/// command prints a warning before it runs, until it says it means
/// to, with `wrap_dcdc_command = true`.
///
/// The verbose mode, `-v/--verbose`, names the file the resolved
/// sub-command runs from, and every other sub-command the same name
/// matched, so a plugin developer sees what a bare name hides.
fn dispatch_subcommand(
    name: &str,
    rest: &[String],
    own: &arguments::OwnArguments,
    plugins: &[plugin::InstalledPlugin],
    loaded: &config::Loaded,
    cwd: &Path,
    sync: &mut plugin::sync::BackgroundSync,
) -> Result<i32> {
    // The sub-command's own arguments pass through untouched: a
    // plugin may declare `-c` and `-v` for itself, and dcdc no
    // longer strips them from the sub-command's word list. The
    // container comes only from dcdc's leading arguments.
    let mut container = own.value("--container").map(str::to_string);

    // The set was read while the sync may still be writing, so a
    // miss may be a plugin not yet in the set: a pending or still
    // running sync, or an empty set, each makes a re-read after the
    // sync settles warranted. A miss against a settled, non-empty
    // set is final.
    let reread_worthy = sync_pending() || sync.is_running() || plugins.is_empty();
    // A qualified name names one plugin, so a clean miss is a real
    // error: it never falls through to the script list.
    let qualified = name.contains(':');
    let res = match plugin::resolve(plugins, name) {
        Ok(res) => Some(res),
        // Only a clean miss can be a plugin not yet installed; a
        // qualified miss and an ambiguous name are errors either
        // way, because neither can be a script.
        Err(Error::PluginSubcommandNotFound { .. }) if reread_worthy => {
            // A settled sync makes the wait a no-op; a running one
            // is waited out with a progress bar.
            let plugins = wait_and_rediscover(sync)?;
            match plugin::resolve(&plugins, name) {
                Ok(res) => Some(res),
                // Still missing after the sync: a legacy script,
                // unless the name was qualified.
                Err(Error::PluginSubcommandNotFound { .. }) if !qualified => None,
                Err(e) => return Err(e),
            }
        }
        // A bare miss against a settled set may be a legacy script;
        // a qualified miss is final.
        Err(Error::PluginSubcommandNotFound { .. }) if !qualified => None,
        Err(e) => return Err(e),
    };

    let Some(res) = res else {
        if !rest.is_empty() {
            return Err(Error::TooManyArgs(name.to_string()));
        }
        return run_script(name);
    };
    // The verbose mode accounts for the resolution before anything
    // runs: the file that will execute, and every other sub-command
    // the same name matched, which the selection hides.
    if own.is_set("--verbose") {
        print_verbose_resolution(name, &res, plugins);
    }
    // A sub-command that claims the name of a dcdc base command
    // hides dcdc's own command; the warning says so, and names the
    // export that silences it, on every run until the plugin adds
    // it.
    if let Some(cmd) = plugin::overridden_base_command(&res.subcommand)
        && !res.subcommand.meta().wrap_dcdc_command
    {
        eprintln!(
            "dcdc: warning: plugin {} overrides DCDC command {cmd}. Add \
             `wrap_dcdc_command = true` to the plugin definition to hide this warning.",
            res.plugin.name
        );
    }
    // A sub-command runs in a container scope: the explicit -c
    // value, else the plugin's saved default. With neither, and an
    // interactive terminal, dcdc asks once and records the answer
    // in the dcdc.toml that applies to this directory.
    if container.is_none()
        && !loaded.config.plugins.contains_key(&res.plugin.name)
        && let Some(chosen) = ask_scope(&res.plugin, cwd)
    {
        config::save_plugin(
            &loaded.path,
            &res.plugin.name,
            &config::PluginSettings {
                container: chosen.clone(),
            },
        )?;
        let root = root::find(cwd).ok();
        if docker::scope_found(&chosen, root.as_deref()) {
            eprintln!(
                "Plugin scope set for the {chosen} container. Use \
                 `--container {chosen}` to override to a different container."
            );
            container = Some(chosen);
        } else {
            eprintln!("Container {chosen} not found.");
            return Ok(1);
        }
    }
    let target = build_target(&res.plugin, container.as_deref(), loaded, cwd)?;
    let project = project_ctx(cwd)?;
    let ctx = plugin::runtime::Ctx {
        plugin_dir: res.plugin.dir.clone(),
        root: project.root.as_ref().map(PathBuf::from),
        cwd: cwd.to_path_buf(),
    };
    plugin::runtime::execute(&res, rest, &target, &project, &ctx)
}

/// Prints the verbose account of a resolved sub-command: the file
/// that runs, and the sub-commands of the same name that did not.
///
/// The rivals are matched on the bare command part of the call, so a
/// qualified call also reports the matches its qualifier bypassed.
fn print_verbose_resolution(
    name: &str,
    res: &plugin::Resolution,
    plugins: &[plugin::InstalledPlugin],
) {
    eprintln!(
        "dcdc: verbose: {name} resolves to plugin {} ({}) in {}",
        res.plugin.name,
        source_word(res.plugin.source),
        res.subcommand.path.display(),
    );
    let bare = name.rsplit_once(':').map(|(_, cmd)| cmd).unwrap_or(name);
    let others = plugin::unselected_matches(plugins, bare, res);
    if !others.is_empty() {
        eprintln!(
            "dcdc: verbose: {} other sub-command(s) use the same name and were not selected:",
            others.len()
        );
        for other in &others {
            eprintln!(
                "dcdc: verbose:   {} ({}) in {}",
                plugin::labeled(&other.plugin, &other.subcommand),
                source_word(other.plugin.source),
                other.subcommand.path.display(),
            );
        }
    }
}

/// The word a verbose line uses for a plugin's source.
fn source_word(source: plugin::Source) -> &'static str {
    match source {
        plugin::Source::Project => "project",
        plugin::Source::Home => "home",
    }
}

/// Asks which container a plugin sub-command should run in, when no
/// scope is configured.
///
/// It asks only from an interactive terminal; elsewhere it returns
/// `None`, so a script keeps the existing no-target error. A
/// cancelled or empty prompt counts as no answer, too. The prompt
/// names the project's compose services when it can list them.
fn ask_scope(plugin: &plugin::InstalledPlugin, cwd: &Path) -> Option<String> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return None;
    }
    let services = root::find(cwd)
        .ok()
        .and_then(|r| docker::compose_services(&r).ok())
        .unwrap_or_default();
    let hint = if services.is_empty() {
        "a running container name, or `local` for the host"
    } else {
        &format!("{}, or `local` for the host", services.join(", "))
    };
    inquire::Text::new(&format!(
        "Which container should {} run in? ({hint})",
        plugin.name
    ))
    .prompt()
    .ok()
    .filter(|s| !s.trim().is_empty())
    .map(|s| s.trim().to_string())
}

/// Builds the execution target of a plugin sub-command from its
/// explicit or configured container.
fn build_target(
    plugin: &plugin::InstalledPlugin,
    container: Option<&str>,
    loaded: &config::Loaded,
    cwd: &Path,
) -> Result<plugin::runtime::Target> {
    let name = match container {
        Some(name) => name.to_string(),
        None => match loaded.config.plugins.get(&plugin.name) {
            Some(settings) => settings.container.clone(),
            None => {
                return Err(Error::NoTarget {
                    plugin: plugin.name.clone(),
                });
            }
        },
    };

    if name == "local" {
        return Ok(plugin::runtime::Target {
            name: "local".into(),
            is_local: true,
            workdir: cwd.display().to_string(),
            shell: docker::shell_local(),
            cwd_mounted: false,
        });
    }

    let root = root::find(cwd)?;
    let (container, workdir, mounted) = docker::resolve_container(&name, &root, cwd)?;
    let shell = docker::shell_in(&container, &workdir)?;
    Ok(plugin::runtime::Target {
        name: container,
        is_local: false,
        workdir,
        shell,
        cwd_mounted: mounted,
    })
}

/// The project context a plugin sees: the root and the compose
/// services when inside a project, else just an empty context.
fn project_ctx(cwd: &Path) -> Result<plugin::runtime::Project> {
    let root = root::find(cwd).ok();
    let services = match &root {
        Some(root) => docker::compose_services(root).unwrap_or_default(),
        None => Vec::new(),
    };
    Ok(plugin::runtime::Project {
        root: root.map(|p| p.display().to_string()),
        services,
    })
}

/// Prints every discovered script with where it would run, followed
/// by the installed plugin sub-commands.
///
/// Listing is how the discovery is visible without running anything,
/// which keeps the load step inspectable on its own.
fn list(plugins: &[plugin::InstalledPlugin], loaded: &config::Loaded) -> Result<i32> {
    let cwd = env::current_dir()?;
    let root = root::find(&cwd)?;
    let cmd_dir = cmd_dir(&root);
    let scripts = script::load(&cmd_dir)?;

    if scripts.is_empty() {
        println!("No scripts found in {}.", cmd_dir.display());
        println!();
        println!("Place bash scripts under .dcdc/cmd to run them with dcdc.");
    } else {
        println!("Scripts in {}:", cmd_dir.display());
        let width = scripts.iter().map(|s| s.name.len()).max().unwrap_or(0);
        for s in &scripts {
            let mode = match &s.subdir {
                None => "local".green().to_string(),
                Some(service) => format!("docker: {service}").yellow().to_string(),
            };
            println!("  {:<width$}  {mode}", s.name);
        }
    }

    if !plugins.is_empty() {
        println!();
        println!("Plugins:");
        for plugin in plugins {
            let default = loaded
                .config
                .plugins
                .get(&plugin.name)
                .map(|s| s.container.as_str())
                .unwrap_or("none");
            let width = plugin
                .subcommands
                .values()
                .map(|s| plugin::label(s).len())
                .max()
                .unwrap_or(0);
            for sub in plugin.subcommands.values() {
                let tag = if default == "local" {
                    "local".green().to_string()
                } else {
                    format!("docker: {default}").yellow().to_string()
                };
                println!("  {:<width$}  {tag}", plugin::label(sub));
            }
        }
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
