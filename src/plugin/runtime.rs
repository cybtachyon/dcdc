//! Runs a resolved plugin sub-command inside a rustyscript VM.
//!
//! The module is sandboxed: it has no filesystem or network access of
//! its own. Its only reach into the host is the `dcdc` object, whose
//! functions are registered Rust callbacks. The object is installed
//! after the module loads, so top-level module code runs without
//! host access and only the entry point runs in the full context.

use rustyscript::serde_json::{Value, json};
use rustyscript::{Error as RsError, Module, Runtime, RuntimeOptions};

use super::Resolution;
use crate::error::{Error, Result};

/// The target a sub-command runs against.
#[derive(Debug, Clone)]
pub struct Target {
    /// Container name, or `local` for the host.
    pub name: String,
    pub is_local: bool,
    /// Working directory for runs: the bind-mounted cwd when
    /// available, otherwise the container's working directory.
    pub workdir: String,
    /// Shell preferred in the target: `bash` when present, else
    /// `sh`.
    pub shell: String,
    /// Whether the cwd is bind-mounted into the target container.
    /// Decides whether the cwd's mise.toml applies to tools.
    pub cwd_mounted: bool,
}

/// Project context passed to the plugin.
#[derive(Debug, Clone)]
pub struct Project {
    /// Project root path, or none when running outside a project.
    pub root: Option<String>,
    /// Services defined by the project's compose file.
    pub services: Vec<String>,
}

/// What the tool policy can read on the host side.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The plugin's directory, holding its `mise.toml`.
    pub plugin_dir: std::path::PathBuf,
    /// The project root, or none, holding its `mise.toml`.
    pub root: Option<std::path::PathBuf>,
    /// The directory the command was invoked from.
    pub cwd: std::path::PathBuf,
}

/// Runs the sub-command and returns its exit code.
pub fn execute(
    res: &Resolution,
    args: &[String],
    target: &Target,
    project: &Project,
    ctx: &Ctx,
) -> Result<i32> {
    let module = Module::load(&res.subcommand.path).map_err(|e| Error::PluginRuntime {
        name: res.plugin.name.clone(),
        message: format!("cannot read {}: {e}", res.subcommand.path.display()),
    })?;

    let mut rt = Runtime::new(RuntimeOptions::default()).map_err(|e| Error::PluginRuntime {
        name: res.plugin.name.clone(),
        message: format!("runtime init: {e}"),
    })?;

    // The host side of dcdc.run. The closure must be `Fn`, so the
    // captured state is cloned per call into the async block.
    let run_target = target.clone();
    let run_ctx = ctx.clone();
    rt.register_async_function("dcdc_run", move |cmd_args: Vec<Value>| {
        let target = run_target.clone();
        let ctx = run_ctx.clone();
        Box::pin(async move {
            let code = exec_command(&target, &ctx, &cmd_args).await;
            Ok::<Value, RsError>(Value::from(code))
        })
    })
    .map_err(|e| runtime_error(res, e))?;

    rt.register_async_function("dcdc_confirm", |_args: Vec<Value>| {
        Box::pin(async move {
            let msg = _args.first().and_then(Value::as_str).unwrap_or("");
            Ok::<Value, RsError>(Value::from(confirm(msg)))
        })
    })
    .map_err(|e| runtime_error(res, e))?;

    let handle = rt.load_module(&module).map_err(|e| runtime_error(res, e))?;

    // Sub-command metadata is read from the TypeScript, per the
    // spec; a missing export degrades to an empty value.
    let version: String = rt.get_value(Some(&handle), "version").unwrap_or_default();
    let description: String = rt
        .get_value(Some(&handle), "description")
        .unwrap_or_default();

    let preamble = build_preamble(
        &res.plugin.name,
        &version,
        &description,
        args,
        target,
        project,
    );
    rt.eval::<Value>(&preamble)
        .map_err(|e| runtime_error(res, e))?;

    let call_args = vec![Value::from(args.to_vec())];
    let result: Value = rt
        .tokio_runtime()
        .block_on(rt.call_entrypoint_async(&handle, &call_args))
        .map_err(|e| runtime_error(res, e))?;

    // The contract is an exit code; be forgiving about what the
    // plugin actually returned.
    Ok(match result.as_i64() {
        Some(code) => i32::try_from(code).unwrap_or(1),
        None if result.is_null() => 0,
        None => {
            eprintln!(
                "dcdc: warning: plugin {} sub-command {} returned {result}, \
                 using exit code 1",
                res.plugin.name, res.subcommand.name
            );
            1
        }
    })
}

/// The host side of `dcdc.run`.
///
/// The first argument is the command name, the second, when
/// present, its argument list.
async fn exec_command(target: &Target, ctx: &Ctx, cmd_args: &[Value]) -> i32 {
    let Some(cmd) = cmd_args.first().and_then(Value::as_str) else {
        return 127;
    };
    let argv: Vec<String> = cmd_args
        .get(1)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if let Some(code) = ensure_tool(target, ctx, cmd) {
        return code;
    }
    if target.is_local {
        spawn_local(cmd, &argv)
    } else {
        docker_exec(target, cmd, &argv)
    }
}

/// The tool policy for the first word of a command, when a
/// `mise.toml` declares that tool.
fn tool_policy(cmd: &str, target: &Target, ctx: &Ctx) -> super::tools::ToolPolicy {
    let tool = cmd.split_whitespace().next().unwrap_or(cmd);
    let key = tool.rsplit('/').next().unwrap_or(tool);
    // Each source is empty when its file is absent or not part of
    // the resolution order for this target.
    let cwd = super::tools::parse_mise(&ctx.cwd.join("mise.toml")).unwrap_or_default();
    let root = ctx
        .root
        .as_ref()
        .and_then(|r| super::tools::parse_mise(&r.join("mise.toml")).ok())
        .unwrap_or_default();
    let plugin = super::tools::parse_mise(&ctx.plugin_dir.join("mise.toml")).unwrap_or_default();
    super::tools::resolve(key, target, &cwd, &root, &plugin)
}

/// Checks a declared tool before running.
///
/// Returns the exit code when the run must not proceed: a missing
/// tool that was declined, or could not be installed, is a
/// "nothing will happen" 127. A version mismatch only warns.
fn ensure_tool(target: &Target, ctx: &Ctx, cmd: &str) -> Option<i32> {
    let tool = cmd.split_whitespace().next().unwrap_or(cmd);
    let key = tool.rsplit('/').next().unwrap_or(tool);
    let policy = tool_policy(cmd, target, ctx);
    if !policy.declared {
        // Undeclared tools: the spawn itself reports the missing
        // command, with the mise.toml hint.
        return None;
    }
    let Some(version) = probe_version(target, key) else {
        let what = if target.is_local {
            "the host"
        } else {
            &target.name
        };
        let (spec, source) = match (policy.spec.as_ref(), policy.source) {
            (Some(spec), Some(source)) => (spec, source),
            _ => {
                eprintln!(
                    "dcdc: nothing will happen: '{key}' is not installed in {what}. \
                     Did the plugin creator forget to include a mise.toml?"
                );
                return Some(127);
            }
        };
        eprintln!(
            "dcdc: {key} is not installed in {what}; {} asks for {spec}",
            source.label()
        );
        if !confirm(&format!("Install {key} {spec}?")) {
            eprintln!("dcdc: nothing will happen, as the command is missing");
            return Some(127);
        }
        if install_tool(target, ctx, key, spec) != 0 {
            eprintln!("dcdc: the install failed; nothing will happen");
            return Some(127);
        }
        return None;
    };
    if let Some(spec) = &policy.spec
        && !super::tools::matches(spec, &version)
    {
        eprintln!(
            "dcdc: warning: {key} {}.{}.{} does not satisfy {spec}; \
             proceeding anyway",
            version.major, version.minor, version.patch
        );
    }
    None
}

/// The installed version of a tool in the target, or none.
fn probe_version(target: &Target, tool: &str) -> Option<super::tools::Version> {
    let text = if target.is_local {
        let out = std::process::Command::new(tool)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        crate::docker::exec_captured(
            &target.name,
            &target.workdir,
            &[tool.to_string(), "--version".to_string()],
        )
        .ok()?
    };
    parse_version(&text)
}

/// Parses the version from a `--version` output.
///
/// A line that says `version` is preferred, because tools like
/// bash also print a year on later lines; otherwise the first
/// version-looking token of the whole output is taken.
fn parse_version(text: &str) -> Option<super::tools::Version> {
    let mut fallback = None;
    for line in text.lines() {
        let version_line = line
            .split_whitespace()
            .any(|t| t.eq_ignore_ascii_case("version"));
        for v in line_version_tokens(line) {
            if version_line {
                return Some(v);
            }
            if fallback.is_none() {
                fallback = Some(v);
            }
        }
    }
    fallback
}

/// The version tokens of a line, in order: each run of digits and
/// dots that parses as a version, so a trailing parenthesized part
/// like `5.2.21(1)` stops the run cleanly.
fn line_version_tokens(line: &str) -> Vec<super::tools::Version> {
    let mut out = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let candidate: String = chars[start..i].iter().collect();
            if let Some(v) = super::tools::parse_version(&candidate) {
                out.push(v);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Runs `mise install` for a tool, in the directory the constraint
/// came from on the host, or inside the target container.
///
/// Returns the exit code; nonzero means the install failed.
fn install_tool(target: &Target, ctx: &Ctx, tool: &str, spec: &str) -> i32 {
    let ref_ = format!("{tool}@{spec}");
    if target.is_local {
        let dir = install_dir(ctx);
        match std::process::Command::new("mise")
            .args(["install", &ref_])
            .current_dir(&dir)
            .status()
        {
            Ok(status) => status.code().unwrap_or(1),
            // A host without mise cannot install on the host.
            Err(_) => {
                eprintln!(
                    "dcdc: mise is not installed on the host, \
                     so {tool} cannot be installed"
                );
                1
            }
        }
    } else {
        crate::docker::exec_in(
            &target.name,
            &target.workdir,
            &["mise".into(), "install".into(), ref_],
        )
        .unwrap_or(1)
    }
}

/// The directory to install a host tool into, following the policy
/// source: the working directory or the project root when either
/// holds a `mise.toml`, else the plugin's own directory.
fn install_dir(ctx: &Ctx) -> std::path::PathBuf {
    if ctx.cwd.join("mise.toml").is_file() {
        return ctx.cwd.clone();
    }
    if let Some(root) = &ctx.root
        && root.join("mise.toml").is_file()
    {
        return root.clone();
    }
    ctx.plugin_dir.clone()
}

/// Spawns a command on the host in the current directory, with
/// stdio inherited, and returns its exit code.
fn spawn_local(cmd: &str, argv: &[String]) -> i32 {
    match std::process::Command::new(cmd).args(argv).status() {
        Ok(status) => status.code().unwrap_or(1),
        // A missing command surfaces as NotFound; under a sandboxed
        // environment the execve denial surfaces as PermissionDenied
        // instead. Both mean the tool cannot run here.
        Err(e)
            if e.kind() == std::io::ErrorKind::NotFound
                || e.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            // The plugin asked for a tool that is not installed.
            // Per the spec, warn that nothing happens and point at
            // a likely cause.
            eprintln!(
                "dcdc: nothing will happen: '{cmd}' is not installed. \
                 Did the plugin creator forget to include a mise.toml?"
            );
            127
        }
        Err(e) => {
            eprintln!("dcdc: {e}");
            1
        }
    }
}

/// Runs a command inside the target container, with stdio
/// inherited.
fn docker_exec(target: &Target, cmd: &str, argv: &[String]) -> i32 {
    let mut full = vec![cmd.to_string()];
    full.extend(argv.iter().cloned());
    match crate::docker::exec_in(&target.name, &target.workdir, &full) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("dcdc: {e}");
            1
        }
    }
}

/// The host side of `dcdc.confirm`: a y/n prompt in the terminal.
///
/// A failed prompt (closed terminal) behaves as a decline, the
/// default of `false`.
fn confirm(msg: &str) -> bool {
    inquire::Confirm::new(msg)
        .with_default(false)
        .prompt()
        .unwrap_or_default()
}

/// Builds the JS that installs the ambient `dcdc` object.
///
/// The function members are bridges to the registered host
/// functions; everything else is a JSON literal, which is valid JS.
fn build_preamble(
    plugin: &str,
    version: &str,
    description: &str,
    args: &[String],
    target: &Target,
    project: &Project,
) -> String {
    let plugin_json = json!({
        "name": plugin,
        "version": version,
        "description": description,
    });
    let container_json = json!({
        "name": target.name,
        "isLocal": target.is_local,
        "workdir": target.workdir,
        "cwdMounted": target.cwd_mounted,
    });
    let project_json = json!({
        "root": project.root,
        "services": project.services,
    });
    let args_json = Value::from(args.to_vec());
    format!(
        r#"
        globalThis.dcdc = {{
          run: (...a) => rustyscript.async_functions.dcdc_run(...a),
          confirm: (...a) => rustyscript.async_functions.dcdc_confirm(...a),
          plugin: {plugin},
          args: {args},
          shell: {shell},
          container: {container},
          project: {project},
        }};
        "#,
        plugin = plugin_json,
        args = args_json,
        shell = json!(target.shell),
        container = container_json,
        project = project_json,
    )
}

/// Maps a rustyscript error onto the plugin runtime error, naming
/// the plugin and sub-command.
fn runtime_error(res: &Resolution, e: RsError) -> Error {
    Error::PluginRuntime {
        name: res.plugin.name.clone(),
        message: format!("{} (sub-command {})", e, res.subcommand.name),
    }
}

/// The metadata a sub-command exports from its TypeScript file.
///
/// This is the plugin's configuration: there is no manifest file,
/// so the name, description, version, and aliases all live in the
/// sub-command itself, and so do its optional usage and example
/// lines, which the CLI shows for `dcdc --help <command>`.
#[derive(Debug, Clone, Default)]
pub struct SubcommandMeta {
    /// The command name; the file's stem when not exported.
    pub name: String,
    /// One-line description, for display.
    pub description: String,
    /// Version string, as exported.
    pub version: String,
    /// Alternate names that resolve to this sub-command.
    pub aliases: Vec<String>,
    /// An invocation line, shown under `dcdc --help <command>`.
    pub usage: String,
    /// A sample invocation, shown under `dcdc --help <command>`.
    pub example: String,
    /// That the sub-command means to own its name: it wins a
    /// tie with other sub-commands of the same name in one
    /// scope, and it silences the warning a sub-command that
    /// claims a dcdc base command name prints on every run.
    pub wrap_dcdc_command: bool,
}

/// Loads a sub-command module and returns its exported metadata,
/// without calling the entry point. Used by listings and name
/// resolution.
///
/// A missing or broken export degrades to the defaults: the file's
/// stem for the name, empties otherwise.
pub fn read_meta(path: &std::path::Path) -> SubcommandMeta {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let mut meta = SubcommandMeta {
        name: stem,
        ..Default::default()
    };
    let Ok(module) = Module::load(path) else {
        return meta;
    };
    let Ok(mut rt) = Runtime::new(RuntimeOptions::default()) else {
        return meta;
    };
    let Ok(handle) = rt.load_module(&module) else {
        return meta;
    };
    for key in ["name", "description", "version", "usage", "example"] {
        if let Some(s) = rt
            .get_value::<Value>(Some(&handle), key)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
        {
            match key {
                "name" => meta.name = s,
                "description" => meta.description = s,
                "usage" => meta.usage = s,
                "example" => meta.example = s,
                _ => meta.version = s,
            }
        }
    }
    if let Some(arr) = rt
        .get_value::<Value>(Some(&handle), "aliases")
        .ok()
        .and_then(|v| v.as_array().cloned())
    {
        meta.aliases = arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
    }
    // A strict boolean: a wrong type degrades to the default, like
    // every other missing export.
    if let Some(flag) = rt
        .get_value::<Value>(Some(&handle), "wrap_dcdc_command")
        .ok()
        .and_then(|v| v.as_bool())
    {
        meta.wrap_dcdc_command = flag;
    }
    meta
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    use crate::plugin::{InstalledPlugin, Source};
    use crate::testutil;

    /// A minimal plugin tree: one plugin with one sub-command that
    /// echoes its arguments through dcdc.run on the host.
    fn build_plugin(base: &Path) -> (PathBuf, Resolution) {
        let dir = base.join("echo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("echo.ts"),
            r#"
            export const version = "9.9.9";
            export const description = "echo";
            export default async (args: string[]): Promise<number> => {
              return dcdc.run("echo", args);
            };
            "#,
        )
        .unwrap();
        let plugin = InstalledPlugin {
            name: "echo".into(),
            dir: dir.clone(),
            source: Source::Home,
            subcommands: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "echo".to_string(),
                    crate::plugin::Subcommand {
                        name: "echo".into(),
                        path: dir.join("echo.ts"),
                    },
                );
                m
            },
        };
        let subcommand = crate::plugin::Subcommand {
            name: "echo".into(),
            path: dir.join("echo.ts"),
        };
        (dir, Resolution { plugin, subcommand })
    }

    #[test]
    fn read_meta_reads_the_exports_and_defaults_the_rest() {
        let base = testutil::temp_subdir("rt-meta");
        let (_dir, res) = build_plugin(&base);
        let meta = read_meta(&res.subcommand.path);
        // The file exports version and description only, so the name
        // is the file's stem and the alias list is empty.
        assert_eq!(meta.name, "echo");
        assert_eq!(meta.version, "9.9.9");
        assert_eq!(meta.description, "echo");
        assert!(meta.aliases.is_empty());

        // An explicit export of every field is read as-is.
        let dir = base.join("full");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("run.ts"),
            r#"
            export const name = "do";
            export const version = "1.2.3";
            export const description = "does things";
            export const aliases = ["do2", "do3"];
            export const usage = "dcdc do <thing>";
            export const example = "dcdc do the-thing";
            export default async (): Promise<number> => 0;
            "#,
        )
        .unwrap();
        let meta = read_meta(&dir.join("run.ts"));
        assert_eq!(meta.name, "do");
        assert_eq!(meta.version, "1.2.3");
        assert_eq!(meta.description, "does things");
        assert_eq!(meta.aliases, vec!["do2", "do3"]);
        assert_eq!(meta.usage, "dcdc do <thing>");
        assert_eq!(meta.example, "dcdc do the-thing");
        assert!(!meta.wrap_dcdc_command);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn read_meta_reads_the_wrap_flag_as_a_strict_boolean() {
        let base = testutil::temp_subdir("rt-wrap");
        // A true boolean is read; a string is not a boolean, so it
        // degrades to the default, like a missing export.
        let dir = base.join("wrap");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("yes.ts"),
            "export const wrap_dcdc_command = true;\nexport default async (): Promise<number> => 0;\n",
        )
        .unwrap();
        assert!(read_meta(&dir.join("yes.ts")).wrap_dcdc_command);
        std::fs::write(
            dir.join("no.ts"),
            "export const wrap_dcdc_command = \"true\";\nexport default async (): Promise<number> => 0;\n",
        )
        .unwrap();
        assert!(!read_meta(&dir.join("no.ts")).wrap_dcdc_command);
        std::fs::write(
            dir.join("none.ts"),
            "export default async (): Promise<number> => 0;\n",
        )
        .unwrap();
        assert!(!read_meta(&dir.join("none.ts")).wrap_dcdc_command);
        std::fs::remove_dir_all(base).unwrap();
    }

    fn host_ctx(dir: &Path) -> Ctx {
        Ctx {
            plugin_dir: dir.to_path_buf(),
            root: None,
            cwd: std::env::current_dir().unwrap_or_default(),
        }
    }

    #[test]
    fn execute_runs_the_entry_point_on_the_host() {
        let base = testutil::temp_subdir("rt-exec");
        let (dir, res) = build_plugin(&base);
        let target = Target {
            name: "local".into(),
            is_local: true,
            workdir: "/".into(),
            shell: "sh".into(),
            cwd_mounted: false,
        };
        let project = Project {
            root: None,
            services: vec![],
        };
        // The host echo prints its arguments on stdout, which the
        // test process inherits; the exit code must be 0.
        let code = execute(
            &res,
            &["hello".to_string()],
            &target,
            &project,
            &host_ctx(&dir),
        )
        .unwrap();
        assert_eq!(code, 0);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn execute_returns_127_for_a_missing_host_command() {
        let base = testutil::temp_subdir("rt-missing");
        let dir = base.join("missing");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("missing.ts"),
            r#"
            export const version = "1.0.0";
            export default async (): Promise<number> => {
              return dcdc.run("definitely-not-a-real-command-xyz");
            };
            "#,
        )
        .unwrap();
        let sub = crate::plugin::Subcommand {
            name: "missing".into(),
            path: dir.join("missing.ts"),
        };
        let res = Resolution {
            plugin: InstalledPlugin {
                name: "missing".into(),
                dir: dir.clone(),
                source: Source::Home,
                subcommands: {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("missing".to_string(), sub.clone());
                    m
                },
            },
            subcommand: sub,
        };
        let target = Target {
            name: "local".into(),
            is_local: true,
            workdir: "/".into(),
            shell: "sh".into(),
            cwd_mounted: false,
        };
        let project = Project {
            root: None,
            services: vec![],
        };
        let code = execute(&res, &[], &target, &project, &host_ctx(&dir)).unwrap();
        assert_eq!(code, 127);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_declared_missing_tool_is_declined_and_stops_at_127() {
        let base = testutil::temp_subdir("rt-decl-missing");
        let dir = base.join("decl");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mise.toml"),
            "[tools]\nsurely-not-a-real-tool-xyz = \"~9.9\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("run.ts"),
            r#"
            export const version = "1.0.0";
            export default async (): Promise<number> => {
              return dcdc.run("surely-not-a-real-tool-xyz");
            };
            "#,
        )
        .unwrap();
        let sub = crate::plugin::Subcommand {
            name: "run".into(),
            path: dir.join("run.ts"),
        };
        let res = Resolution {
            plugin: InstalledPlugin {
                name: "decl".into(),
                dir: dir.clone(),
                source: Source::Home,
                subcommands: {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("run".to_string(), sub.clone());
                    m
                },
            },
            subcommand: sub,
        };
        let target = Target {
            name: "local".into(),
            is_local: true,
            workdir: "/".into(),
            shell: "sh".into(),
            cwd_mounted: false,
        };
        let project = Project {
            root: None,
            services: vec![],
        };
        // No TTY in the test, so the install prompt fails and is a
        // decline; the run must stop at 127, never attempt the tool.
        let code = execute(&res, &[], &target, &project, &host_ctx(&dir)).unwrap();
        assert_eq!(code, 127);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_declared_present_tool_proceeds() {
        let base = testutil::temp_subdir("rt-decl-present");
        let dir = base.join("decl2");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mise.toml"), "[tools]\nbash = \"*\"\n").unwrap();
        std::fs::write(
            dir.join("run.ts"),
            r#"
            export const version = "1.0.0";
            export default async (): Promise<number> => {
              return dcdc.run("bash", ["--version"]);
            };
            "#,
        )
        .unwrap();
        let sub = crate::plugin::Subcommand {
            name: "run".into(),
            path: dir.join("run.ts"),
        };
        let res = Resolution {
            plugin: InstalledPlugin {
                name: "decl2".into(),
                dir: dir.clone(),
                source: Source::Home,
                subcommands: {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("run".to_string(), sub.clone());
                    m
                },
            },
            subcommand: sub,
        };
        let target = Target {
            name: "local".into(),
            is_local: true,
            workdir: "/".into(),
            shell: "sh".into(),
            cwd_mounted: false,
        };
        let project = Project {
            root: None,
            services: vec![],
        };
        // bash is present and the constraint matches, so the run
        // proceeds and the version banner prints on stdout.
        let code = execute(&res, &[], &target, &project, &host_ctx(&dir)).unwrap();
        assert_eq!(code, 0);
        std::fs::remove_dir_all(base).unwrap();
    }
}

#[cfg(test)]
mod parse_version_tests {
    use super::parse_version;

    #[test]
    fn prefers_the_version_line_over_later_years() {
        let text = "GNU bash, version 5.2.21(1)-release (x86_64-pc-linux-gnu)\nCopyright (C) 2022 Free Software Foundation, Inc.\n";
        let v = parse_version(text).unwrap();
        assert_eq!((v.major, v.minor, v.patch), (5, 2, 21));
    }

    #[test]
    fn falls_back_to_the_first_token_when_no_version_word() {
        let text = "jq-1.7.1\n";
        let v = parse_version(text).unwrap();
        assert_eq!((v.major, v.minor, v.patch), (1, 7, 1));
    }
}
