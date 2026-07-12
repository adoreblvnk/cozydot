# cozydot

cozydot is a Rust command-line post-install, update, and dotfile manager for Debian- and Ubuntu-family Linux systems. It preserves the repository's tagged YAML presets and manages apt repositories/pinning, Flatpak, release binaries, language toolchains, applications, GNU Stow dotfiles, and GNOME/Cinnamon settings.

## Install

A stable Rust toolchain is required when building from source:

```bash
git clone https://github.com/adoreblvnk/cozydot.git ~/.cozydot
cargo install --locked --path ~/.cozydot
```

Alternatively, download the `cozydot` release binary for your architecture, mark it executable, and place it in `~/.local/bin`:

```bash
install -Dm755 ./cozydot ~/.local/bin/cozydot
```

The binary locates bundled presets in the source/release tree. Keep `configs/` and `dotfiles/` beside the distributed binary, or pass an explicit YAML path with `--config`.

## Usage

```text
cozydot [OPTIONS] <COMMAND>

Commands: check, install (i), update (u), configure (c)
Options:  -c, --config <CONFIG>  -n, --no-color  --list-configs  -h  -V
```

```bash
cozydot --list-configs
cozydot -c vm install
cozydot configure
```

`default`, `cli`, `full`, and `vm` presets remain available. `!enabled` executes a section; `!disabled` preserves its data but skips it. `COZYDOT_DRY_RUN=1` prints the complete command plan without changing the host.

See [docs/rust-rewrite.md](docs/rust-rewrite.md) for architecture, config compatibility, safety, and development details.
