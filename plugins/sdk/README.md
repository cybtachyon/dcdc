# Writing a dcdc plugin

A plugin is a directory of TypeScript files that dcdc loads in a
sandboxed JavaScript runtime. It adds sub-commands to the CLI:

    dcdc bash echo "Hello world!"

This directory, `sdk/`, holds `dcdc.d.ts`, the type declarations of
the host API. Add it to your TypeScript project's `include` while
writing a plugin:

```ts
// tsconfig.json
{ "include": ["*.ts", "sdk/dcdc.d.ts"] }
```

## Layout

- One or more `.ts` files: the sub-commands.
- `mise.toml` (optional): the tools the plugin needs installed.

```
⊢— shell/
  ∣
  ⊢— mise.toml
  ∣
  ⊢— shell.ts
```

The plugin name is the repository/directory name. Everything else is configured in TypeScript:

- `name`: the command name; the file's stem when omitted,
- `description`: a one-line summary,
- `version`: shown by `dcdc plugin list`,
- `aliases`: an array of alternate names,
- `usage` (optional): an invocation line,
- `example` (optional): a sample invocation,

```ts
export const name = "shell";
export const version = "0.1.0";
export const description = "Run a command in the container shell";
export const aliases = ["bash", "sh"];
export const usage = "dcdc bash <command> [args...]";
export const example = 'dcdc bash echo "Hello world!"';

export default async (args: string[]) => {
  const quoted = args.map((a) => `'${a.replaceAll("'", "'\\''")}'`).join(" ");
  return dcdc.run(dcdc.shell, ["-c", quoted]);
};
```

The default export receives the typed arguments. Return an exit
code, or nothing for zero.

## Where plugins live

dcdc loads plugins from two places, in order:

- the nearest project's `.dcdc/plugins`, then
- the dcdc home's `plugins`.

A sub-command owned by a project plugin shadows a home sub-command
of the same name, so a project can override a default for its
members without touching the home. Project plugins are plain
directories, usually committed with the project. `dcdc plugin
remove <name>` deletes a project plugin's directory, and `dcdc
plugin list` shows one section per place.

## Qualified calls

A sub-command can be called through its plugin's name, with a
colon between the qualifier and the command:

    dcdc default:bash echo "Hello world!"
    dcdc derek/shell:bash echo "Hello world!"
    dcdc myproject:setup run

`default` names the plugins dcdc itself syncs into the home, a
GitHub repository reference (`owner/repo`, or a full URL) names
the installed version of that repository, and any other
qualifier names the current project, by the folder name of its
root. A qualified call always reaches the named plugin, so it is
how you reach a sub-command another plugin has shadowed. A colon
in a name is reserved for this syntax.

## Overriding commands

A sub-command can take the name of a dcdc base command (`plugin`
or `help`) or of any other sub-command. A sub-command named after
a base command overrides it: the bare word runs the plugin instead
of dcdc's own command, so dcdc prints a warning on every run until
the sub-command says it means to:

    dcdc plugin
    dcdc: warning: plugin plover overrides DCDC command plugin.
    Add `wrap_dcdc_command = true` to the plugin definition to hide this warning.

Add the export to silence the warning:

    export const wrap_dcdc_command = true;

Among several sub-commands of one place that claim the same name,
a single one that wraps wins. Project plugins always shadow home
ones, so a project claim beats a home one, wrapped or not. When
two or more of them wrap, or none does, the bare name is a
collision: it is refused, and each sub-command stays reachable
only by its fully namespaced name, like `derek/shell:tool` or
`myproject:tool`.

## Container scope

A sub-command runs in a container scope: the `-c/--container`
flag, or the plugin's default recorded in `dcdc.toml`. With
neither, dcdc asks which container the plugin should run in and
records the answer in the `dcdc.toml` that applies to the
directory:

    [plugin.myplugin]
    container = "api"

The answer can be `local` for the host, a service of the
project's compose file, or a running container's name. A name
that matches nothing is still recorded, but the command does not
run until a container with that name exists. In a non-interactive
terminal dcdc does not ask; pass `-c` there, or set the scope
with `dcdc plugin use`.

## Help

The description always appears in the CLI's help. `dcdc --help`
lists every sub-command with its description under a `Plugin
Commands` section, and `dcdc --help <command>` shows one
sub-command's description along with its `usage` and `example`
exports, when it has them.

The help flag is dcdc's own only before the command name: `dcdc
--help bash` prints the help, while `dcdc bash --help` passes the
flag to the plugin, which is free to answer it however it likes.

## The host API

The `dcdc` global is the only way a plugin talks to the host.
Everything else, file access and network included, is unavailable to
plugin code.

- `dcdc.run(command, args)` runs a program in the target container,
  or on the host when the target is `local`. Returns the exit code.
  When the program is not installed, the CLI tells the user that
  nothing will happen and returns 127.
- `dcdc.confirm(message)` asks a yes/no question in the terminal and
  returns the answer. A failed prompt is a no.
- `dcdc.plugin`, `dcdc.args`, `dcdc.shell`, `dcdc.container`, and
  `dcdc.project` carry the invocation context; see `dcdc.d.ts` for
  the full shapes.

## Tools

Declare the tools a plugin needs in its `mise.toml`, with semantic
version constraints:

```toml
[tools]
jq = "~1.7"
```

Version resolution depends on where the command runs:

- on the host: the `mise.toml` of the working directory wins, then
  the project root's, then the plugin's,
- in a container with the working directory bind-mounted: the
  container's own tools,
- in a container without a mount: the project root's constraint, or
  the version the container already has.

If the working directory and project root declare no version and the
plugin declares one, the CLI warns and asks before installing. If
nothing declares a version and the tool is missing, the CLI warns
that nothing will happen, because the command is missing.

## Publishing

Publish a plugin in a GitHub repository and install it with:

    dcdc plugin get owner/repo
    dcdc plugin use owner/repo [container]

`get` stores the repository under its commit SHA, so several
versions can coexist, and `use` records the default container in
`dcdc.toml`. Omit the container, or use `local`, to run on the host.
