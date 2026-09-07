
Written plugins live in `~/.dcdc/plugins`, and downloaded
repositories are stored per commit SHA. A plugin is a directory of
TypeScript files, one per sub-command; the directory's name is the
plugin name, and each sub-command's name, description, version, and
aliases are exports of its file, so no configuration file is
required. A `mise.toml` may declare the tools the plugin needs,
with version constraints. The TypeScript talks to the host only
through the sandboxed `dcdc` object, whose `dcdc.run` and
`dcdc.confirm` are the only escapes; the full type declarations and
an authoring guide live in `plugins/sdk/README.md` of this
repository. Start a new plugin from the built-in template:

```bash
dcdc plugin new my-plugin
```

## The dcdc home

The per-user state of dcdc, the `dcdc.toml` configuration and the
installed plugins, lives in the `~/.dcdc` directory by default. Set
the `DCDC_HOME` environment variable to another directory to
relocate it; dcdc then keeps its user state in `<DCDC_HOME>/.dcdc`
instead.

## Smoke test

`cargo run --bin smoke` builds dcdc and runs it against the
`examples/next-js-example` project in a throwaway dcdc home: the
build, the listing, the default plugin sync, and the example's
local scripts. Run it to smoke test a change to the CLI. Pass
`--example <name>` to exercise another project from `examples/`:

```bash
cargo run --bin smoke
cargo run --bin smoke -- --example <name>
```
