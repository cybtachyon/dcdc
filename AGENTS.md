# Agent Guidelines
This is a monorepo of a Rust CLI for managing local Docker Compose dev environments.

## Technical Overview
- The back-end is a revisionable CMS that uses the Bob ORM for working with a MariaDB database.
- The front-end uses the Go-App package with Pongo 2 templates and CSS from Bulma via GoDartSass.
- The front-end communicates with the back-end via incremental Websocket streams using the gob encoding format.
- The build process compresses the WASM output using Brotli.
- The CI / CD pipeline is in `.github/workflows`.
- Infrastructure code is in `infra` and uses Terraform to deploy to Digital Ocean.

# Dev Environment Tips
- Read `go.mod` for the list of configured packages and dependencies.
- Web Assembly has a 2GB memory limit, so prioritize memory efficiency for the front-end.

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
