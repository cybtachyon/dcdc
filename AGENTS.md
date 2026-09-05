# Agent Guidelines
This is a monorepo of a Rust CLI for managing local Docker Compose dev environments.

## Technical Overview
- The focus is on speed and ease of use, followed by developer experience for creating plugins and extending functionality.
- This is a plugin-based architecture where they are written in TypeScript and provide CLI sub-commands.
- Plugins can be stored in ~/.dcdc/plugins or added via CLI command.

# Dev Environment Tips
- Read `Cargo.toml` for the list of configured packages and dependencies.
- Run `cargo` commands with `CARGO_HOME` set to `.cargo` to avoid sandbox restrictions.
  - e.g. `CARGO_HOME=.cargo cargo build`
- Use `DCDC_HOME` to set the dcdc home directory to a local path for sandboxed testing.
  - e.g. `DCDC_HOME=.dcdc ../../target/debug/dcdc hello-world` 

## Code Style
- Follow Rust conventions: [Effective Rust]([https://go.dev/doc/effective_go](https://effective-rust.com/title-page.html)). Be concise, declarative, and factual.
- Never use grammatical shortcuts like emdash. Use commas, semicolons, or separate sentences.
- **Doc comments** start with the name and state what it does. Structure: `// [Name] [verb]s [what]. [Optional: when/why to use it].` Put useful information where users make decisions (usually the constructor, not methods).
- **Inline comments** are terse. Prefer end-of-line when short enough.
- **Explain why, not what.** The code shows what it does; comments should explain reasoning, non-obvious decisions, or edge cases.
- **Wrap comments** at ~80 characters, continuing naturally at word boundaries.
- **Naming matters.** Before proposing a name, stop and review existing names in the file. Ask: what would someone assume from this name? Does it fit with how similar things are named? A good name is accurate on its own and consistent in context.
- **Comment content:**
  - Add information beyond what the code shows (not tautologies)
  - State directly: "Returns X" (not "Note that this returns X")
  - Drop filler: "basically", "actually", "really" add nothing

## Docs
- [Rustyscript](https://docs.rs/rustyscript/latest/rustyscript/)
- [indiciatif](https://docs.rs/indicatif/latest/indicatif/)
- [inquire](https://docs.rs/inquire/latest/inquire/)
- [owo_colors](https://docs.rs/owo-colors/latest/owo_colors/)
- [Packager](https://docs.crabnebula.dev/packager/)
- [Packager Updater](https://github.com/crabnebula-dev/cargo-packager/tree/main/crates/updater)

## Testing Instructions
`cargo test`
