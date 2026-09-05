# dcdc
Dcdc Compose Dev CLI (DCDC) - The Useful Local Dev Solution.

## Features
- **Plugin-based Architecture** - For unlimited OOTB support
- **Simple Setup** - Only requires docker to be installed
- **Unlimited Projects** - Develop to your heart’s content
- **Multi-platform** - For macOS, Linux, and Windows
- **Custom Domains and SSL** - Uncomplicated local development
- **Knows Your Stack** - Includes presets for the most common apps, from Next.js, to Laravel, Wordpress, PostCMS, Drupal, Craft CMS, and more
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

@todo Finish this stub.

## Docs

@todo Finish this stub with links to the GitHub wiki.

## Contributing.

@todo Finish this stub.

### Build Release Packages

`cargo packager --release`

If packaging in WSL2, prefix with `PATH=$(echo "$PATH" | tr ':' '\n' | grep -v "WindowsApps" | tr '\n' ':')`.

Then you can `sudo apt install ./dist/dcdc_0.1.0_amd64.deb`.

## Acknowledgements

@todo Finish this stub.

## License
MIT. See [LICENSE](LICENSE).
