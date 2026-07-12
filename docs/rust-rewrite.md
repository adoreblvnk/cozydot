# Rust rewrite

cozydot is now a Rust binary; the legacy root Bash executable is not restored. The implementation is split into CLI parsing, tagged-YAML validation, platform mappings, command planning, and process execution.

## Compatibility

The CLI keeps the original command-loop shape:

- `-V` reports `cozydot 0.0.1`.
- `--config <name>` selects `<root>/configs/<name>.yaml`; path-style config arguments are rejected.
- `--list-configs` can be combined with other options.
- Multiple commands can run sequentially, such as `cozydot check update`.
- `--no-color` is accepted for compatibility; Rust output is plain by default.

Tagged config values are validated before planning. Unknown fields, mistyped fields, unsafe paths, unsupported binary suffixes, malformed versions, and unsupported URL lookup forms fail with contextual errors instead of panics. `!enabled` executes a section and `!disabled` skips it while preserving the data.

Repository variables (`UPSTREAM_DISTRO`, `VERSION_CODENAME`, `UNAME_ARCH`, `GO_ARCH`, `LINUX_ARCH`, `X64_ARCH`, `ARM64_SUFFIX`) are expanded from detected platform data. Apt repo entries also resolve the legacy `$(dpkg --print-architecture)` expression to the planned Debian architecture instead of writing it literally.

## Behavior Covered

The planner covers the legacy host-facing flows from `3b98859:cozydot`:

- distro preparation for Ubuntu, Linux Mint, and Debian, including snap cleanup, nosnap pinning, auto-upgrade disabling, Debian sources, and per-package apt guards;
- apt purge/dependency installs, Rustup bootstrap, appimaged cleanup/install, dynamic FUSE package selection, and Nerd Font install;
- third-party apt signing keys, repo files, exact pinning stdin, and package installs with per-package guards;
- Flatpak, Cargo/cargo-binstall, release binaries, Go, FNM/Node, npm, pyenv update/pip, and uv self-update/managed-Python behavior;
- apt/Flatpak/Rustup/Cargo/yq/Go/Node update behavior with command/state guards;
- Stow override/backup, Docker daemon preservation, Docker/VirtualBox groups, VS Code extension idempotency, terminal selection, GNOME settings, extension install/enable, dock keys, and rounded-corner settings.

Config-derived values are passed as process arguments or stdin to fixed command snippets. The remaining shell snippets are static and are used for state checks or tightly scoped workflows that need shell features; YAML values are not interpolated into generated shell source.

## Packaging

`scripts/package-release.sh` builds `target/cozydot-0.0.1.tar.gz` with:

- `cozydot`
- `configs/`
- `dotfiles/`

At runtime the binary uses `COZYDOT_ROOT` when set, otherwise an adjacent `configs/` and `dotfiles/` directory, otherwise the source checkout path for development. CI and tests smoke the extracted layout.

## Safety and Testing

Every side effect crosses the `Runner` trait. Production uses `ProcessRunner`; tests inspect plans, dry-run CLI output, validation errors, and packaged-layout behavior without invoking package managers.

Development gates:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
scripts/package-release.sh
bash -n dotfiles/bash/.bashrc
```

Privileged downloads now use unique temporary files and stage destructive replacements after archive extraction succeeds. The rewrite does not invent checksums for upstream assets that do not have pinned hashes in the current config; adding explicit checksum fields would be the next schema extension if those upstream artifacts are to be cryptographically pinned.
