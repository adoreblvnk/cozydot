<div align="center"> <!-- use align as CSS is not allowed on GitHub markdown https://github.com/orgs/community/discussions/22728 -->
  <h1>cozydot</h1> <!-- Project Name -->
  <p> <!-- Description -->
    Declarative system setup & dotfile manager for Linux & macOS
  </p>
  <p> <!-- Built With -->
    Built With: Rust &bull; <a href="https://www.gnu.org/software/stow">GNU Stow</a>
  </p>
</div>

---

<details>
<summary>Table of Contents</summary>

- [About](#about)
  - [Why cozydot](#why-cozydot)
  - [Supported platforms](#supported-platforms)
- [Demo](#demo)
- [Getting Started](#getting-started)
  - [Prerequisites](#prerequisites)
  - [Installation](#installation)
  - [Execution](#execution)
  - [Uninstall](#uninstall)
- [Usage](#usage)
  - [Configuration](#configuration)
  - [Development](#development)
- [Roadmap](#roadmap)
</details>

## About

Setting up a new computer on Linux or macOS typically requires running many commands: adding GPG keys, configuring 3rd-party APT repositories, downloading binaries, managing dotfiles, & tweaking desktop preferences.

cozydot is built around 2 core principles:
- **Declarative by design:** The entire machine state lives in 1 config file (`cozydot.yaml`) applied idempotently, rather than fragile setup scripts.
- **Native with zero lock-in:** cozydot provisions through official OS tools & standard upstream paths. If cozydot is uninstalled, the system remains manageable from official docs.

### Why cozydot

- **Sane defaults out of the box:** `cozydot init` provisions ready-to-use presets (`cozydot`, `cli`, `vm`) with terminal tools, Nerd Fonts, themes, & shell configurations.
- **Idempotent apply:** `cozydot apply` ensures the desired setup without duplication. Can be run repeatedly with no side effects.
- **Full lifecycle updates:** `cozydot update` keeps system packages, Flatpaks, Homebrew, language toolchains, & fonts up to date.
- **Dry-run safety:** `cozydot check` validates active config against host platform constraints before any changes are made.

### Supported platforms

- **Linux (`x86_64`, `aarch64`)**: Debian 12 (Bookworm), Debian 13 (Trixie), Ubuntu, Pop!_OS, & Linux Mint.
- **macOS (`aarch64`)**: Apple Silicon (`aarch64-apple-darwin`).

## Demo

## Getting Started

### Prerequisites

- Standard utilities: `curl`, `bash`

### Installation

Install the latest pre-compiled binary:

```bash
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/master/install.sh | bash
```

To build & install from source:

```bash
git clone https://github.com/adoreblvnk/cozydot.git
cd cozydot
cargo install --path . --locked
```

### Execution

```bash
# initialize config & bundled dotfiles
cozydot init
# edit active config
$EDITOR "${XDG_CONFIG_HOME:-$HOME/.config}/cozydot/cozydot.yaml"
# optional: validate config for host
cozydot check
# apply config to host
cozydot apply
# optional: run enabled ecosystem updates
cozydot update
```

### Uninstall

```bash
# remove cozydot binary
rm ~/.local/bin/cozydot
```

## Usage

- `cozydot init [--preset <preset>]` \
  Writes active config & bundled dotfiles under `~/.config/cozydot`. Bundled presets:
  - `cozydot` (default): full workstation (GUI apps, Flatpaks, fonts, desktop settings, dev tools)
  - `cli`: headless / server profile (terminal utilities, language toolchains, shell dotfiles)
  - `vm`: lightweight profile for virtual machines & test environments
- `cozydot check` \
  Validates `cozydot.yaml` against schema constraints & detected platform capabilities without changing system state.
- `cozydot apply` \
  Applies active config to the host. Installs missing packages, configures toolchains, links dotfiles, installs extensions, & sets desktop preferences. Installed software & unmanaged packages remain untouched.
- `cozydot dotfiles [-r | --replace]` \
  Symlinks configured dotfile packages with GNU Stow. Simulates transactions to detect conflicts before making changes. Use `-r` / `--replace` to back up conflicting files to `${XDG_STATE_HOME:-$HOME/.local/state}/cozydot/dotfile-backups` before linking.
- `cozydot update` \
  Executes enabled update policies: APT (`upgrade` / `full-upgrade`), Flatpak, Homebrew formulae & casks, Rustup toolchains, `fnm` Node.js versions, `uv` Python versions, Go toolchains, Cargo crates, global npm packages, & Nerd Fonts.

### Configuration

`~/.config/cozydot/cozydot.yaml` layout:

```yaml
system:       # OS settings (sudo group, unattended upgrades)
packages:     # APT repos, Flatpaks, Homebrew formulae & casks, GitHub binaries
tools:        # Rust, Node.js (fnm), Python (uv), Go, Cargo crates, npm
fonts:        # Nerd Font families
dotfiles:     # Stow packages (all, linux, macos)
integrations: # VS Code extensions, agent skills, Docker / VirtualBox
desktop:      # Theme (dark/light), GNOME extensions, macOS defaults
updates:      # Upgrade policies for each ecosystem
```

### Development

```bash
# validate generated preset configurations
scripts/generate-configs.sh --check
# format check & lints
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
# run test suite
cargo test --locked --all-targets --all-features
# documentation
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
# build release archive & checksum
scripts/package-release.sh
```

## Roadmap

- Update managed Deb & AppImage binaries from configured release sources.
- Complete 1st-run Xcode Command Line Tools installation before continuing a macOS apply.
- Add a dedicated command to list bundled presets.

## License <!-- omit in toc -->

Distributed under the MIT License.

## Credits <!-- omit in toc -->

- [adore_blvnk](https://x.com/adore_blvnk)

<!-- Inspired by Best-README-Template (https://github.com/othneildrew/Best-README-Template) -->
<!-- Table of Contents generated by Markdown All in One (https://github.com/yzhang-gh/vscode-markdown) -->

<!-- Helpful notes:
- insert a centred image:
  <div align=center><img src="" alt="" width=750></div>
-->
