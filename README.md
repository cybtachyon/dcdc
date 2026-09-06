# dcdc
Dcdc Compose Dev CLI (DCDC) - The Useful Local Dev Solution.

## Features
- **Plugin-based Architecture** - For unlimited OOTB support
- **Simple Setup** - Only requires docker to be installed
- **Unlimited Projects** - Develop to your heart’s content
- **Multi-platform** - For macOS, Linux, and Windows
- **Custom Domains and SSL** - Uncomplicated local development
- **Knows Your Stack** - Includes presets for the most common apps, from Next.js, to Laravel, Wordpress, PostCMS,
 Drupal, Craft CMS, and more
- **Insanely Fast** - Written in Rust for maximum speed
- **Bring Your Own Anything** - Dockerfiles, scripts, everything’s a plugin
- **No-risk No-lock-in** - Easy to add or remove from any repo without losing a thing

## Installation

If you have [Mise](https://mise.jdx.dev/), it's easy:

```
mise install dcdc
```

### Native Installers

Linux
```bash
curl -fsSL https://raw.githubusercontent.com/cybtachyon/dcdc/refs/heads/main/install.sh | bash
```

Mac
```zsh
brew install orbstack docker
brew install dcdc/dcdc/dcdc
```

Windows
```powershell
wsl --install --no-distribution
Restart-Computer
wsl --update
choco install wsl-ubuntu-2604
choco install dcdc
```

Packages are also available on the [GitHub Releases](https://github.com/cybtachyon/dcdc/releases) page.

## Quick-Start

```bash
dcdc --help

# Using plugins:
dcdc --help bash
# Override the default container for a plugin:
dcdc -c web default:bash ls -la
```

## Plugins

Plugins are TypeScript sub-commands. The defaults are synced into your `~/.dcdc` home when dcdc installs or updates:
```bash
dcdc plugin list          # 1. List plugins
dcdc plugin use jq api    # 2. Pick a default container for a plugin
dcdc bash echo hi         # 3. Use plugin!
```

A project can carry its own plugins in `.dcdc/plugins`. They can
override or wrap defaults, so you can always call a specific
plugin:
```bash
dcdc default:bash ls -la                         # Default plugin
dcdc myproject:plugin --argument                 # A project plugin
dcdc some-user/some-repository:plugin --argument # Installed plugin
```

Add more plugins from online repositories:
```bash
dcdc plugin get some-user/some-plugin
dcdc plugin remove some-user/some-plugin
```

Create your own plugins with:
```bash
dcdc plugin new my-plugin
```

  > ![Info](https://raw.githubusercontent.com/primer/octicons-v2/master/icons/16/info.svg) Plugins can override or wrap other commands. While this can be very useful, because of unintended side-effects.

## Docs

@todo Finish this stub with links to the GitHub wiki.

## Contributing.

The test suite is `cargo test`; run cargo with `CARGO_HOME` set to the in-repo `.cargo` so the registry stays inside the
sandbox. Release packages build with `cargo packager --release`.

See [CONTRIBUTING](CONTRIBUTING.md) for more details.

### Build Release Packages

`cargo packager --release`

If packaging in WSL2, prefix with `PATH=$(echo "$PATH" | tr ':' '\n' | grep -v "WindowsApps" | tr '\n' ':')`.

Then you can `sudo apt install ./dist/dcdc_0.1.0_amd64.deb`.

## Acknowledgements

@todo Finish this stub.

## License
MIT. See [LICENSE](LICENSE).
