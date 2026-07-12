# cozydot

cozydot is a Rust command-line post-install, update, and dotfile manager for Debian- and Ubuntu-family Linux systems. It preserves the repository's tagged YAML presets and manages distro preparation, apt repositories/pinning, Flatpak, release binaries, language toolchains, applications, GNU Stow dotfiles, and GNOME/Cinnamon settings.

## Install

A stable Rust toolchain is required when building from source:

```bash
git clone https://github.com/adoreblvnk/cozydot.git ~/.cozydot
cargo install --locked --path ~/.cozydot
```

Alternatively, build or download the release archive and keep the extracted layout together:

```bash
scripts/package-release.sh
tar -C ~/.local/share -xzf target/cozydot-0.0.1.tar.gz
ln -sf ~/.local/share/cozydot-0.0.1/cozydot ~/.local/bin/cozydot
```

The binary locates bundled presets and dotfiles beside itself, or from `COZYDOT_ROOT`. `--config` selects a named YAML preset under the bundled `configs/` directory; arbitrary config paths are intentionally rejected.

## Usage

```text
cozydot [OPTIONS] [COMMAND...]

Commands: check, install (i), update (u), configure (c)
Options:  -c, --config <CONFIG>  -n, --no-color  --list-configs  -h  -V
```

```bash
cozydot --list-configs
cozydot -c vm install
cozydot check update
cozydot configure
```

`default`, `cli`, `full`, and `vm` presets remain available. `!enabled` executes a section; `!disabled` preserves its data but skips it. `COZYDOT_DRY_RUN=1` prints the complete command plan without changing the host.

See [docs/rust-rewrite.md](docs/rust-rewrite.md) for architecture, config compatibility, safety, and development details.
