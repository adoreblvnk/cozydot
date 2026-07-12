# cozydot

cozydot provisions Debian- and Ubuntu-family systems from a named YAML preset. It manages system packages, repositories, Flatpak apps, language toolchains, dotfiles, and supported desktop settings.

## Install in a VM

### From a release archive

Download the archive for the release, then keep the binary, configs, and dotfiles together:

```bash
mkdir -p ~/.local/share ~/.local/bin
tar -C ~/.local/share -xzf cozydot-<version>.tar.gz
ln -sf ~/.local/share/cozydot-<version>/cozydot ~/.local/bin/cozydot
```

Ensure `~/.local/bin` is in `PATH`, then confirm the installation:

```bash
cozydot --version
cozydot --list-configs
```

### Build the release archive from source

Use this while the Rust rewrite is experimental or when no release archive is available:

```bash
sudo apt-get update
sudo apt-get install -y build-essential curl git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
git clone https://github.com/adoreblvnk/cozydot.git ~/.cozydot
cd ~/.cozydot
archive=$(scripts/package-release.sh)
bundle=$(basename "$archive" .tar.gz)
mkdir -p ~/.local/share ~/.local/bin
tar -C ~/.local/share -xzf "$archive"
ln -sf "$HOME/.local/share/$bundle/cozydot" ~/.local/bin/cozydot
```

The extracted directory is required at runtime because it contains the presets and dotfiles. To update a source-built installation, pull the repository and repeat the package, extract, and symlink steps.

## Run cozydot in a VM

Start with the `vm` preset and inspect the command plan before changing the machine:

```bash
COZYDOT_DRY_RUN=1 cozydot -c vm check install configure
cozydot -c vm check install configure
```

Run updates later with the same preset:

```bash
cozydot -c vm update
```

Commands run in the order supplied. Available commands are `check`, `install` (`i`), `update` (`u`), and `configure` (`c`).

## Choose a config

| Config | Use it for |
| --- | --- |
| `vm` | Lightweight virtual machines with minimal apps and utilities |
| `cli` | CLI-only systems and WSL2 |
| `default` | A regular desktop with the standard cozydot setup |
| `full` | The complete desktop and application set |

Use `-c` to select a preset:

```bash
cozydot -c cli check install configure
cozydot -c default check install configure
cozydot -c full check install configure
```

List the presets bundled with the installed version:

```bash
cozydot --list-configs
```

Only named presets under the bundled `configs/` directory are accepted. In a preset, `!enabled` runs a section and `!disabled` keeps its values without running it.

## CLI

```text
cozydot [OPTIONS] [COMMAND...]

Commands: check, install (i), update (u), configure (c)
Options:  -c, --config <CONFIG>  -n, --no-color  --list-configs  -h  -V
```

See [docs/rust-rewrite.md](docs/rust-rewrite.md) for architecture, configuration compatibility, safety boundaries, and development details.
