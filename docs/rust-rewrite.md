# Rust rewrite

cozydot is now a Rust binary; the legacy root Bash executable is not restored. The implementation is split into CLI parsing, tagged-YAML validation, platform mappings, command planning, and process execution.

## Compatibility

The public CLI is intentionally small:

- `-V` reports `cozydot 0.0.1`.
- `cozydot init` creates or safely refreshes the active config and bundled dotfiles.
- `cozydot apply` applies `${XDG_CONFIG_HOME:-$HOME/.config}/cozydot/cozydot.yaml`.
- Preset selection, profile inheritance, planning, and multi-command execution are not public interfaces.

`apply` intentionally runs the legacy install and configure phases, including their configured checks. The legacy update phase remains an internal planner path for compatibility testing and is not run by `apply`; recurring upgrades are therefore not an implicit side effect of provisioning.

Tagged config values are validated before planning. Unknown fields, mistyped fields, unsafe paths, unsupported binary suffixes, malformed versions, and unsupported URL lookup forms fail with contextual errors instead of panics. `!enabled` executes a section and `!disabled` skips it while preserving the data.

Repository variables (`UPSTREAM_DISTRO`, `VERSION_CODENAME`, `UNAME_ARCH`, `GO_ARCH`, `LINUX_ARCH`, `X64_ARCH`, `ARM64_SUFFIX`) are expanded from detected platform data. Apt repo entries also resolve the legacy `$(dpkg --print-architecture)` expression to the planned Debian architecture instead of writing it literally.

## Behavior Covered

The planner covers the legacy host-facing flows from `3b98859:cozydot`:

- distro preparation for Ubuntu, Linux Mint, and Debian, including snap cleanup, nosnap pinning, auto-upgrade disabling, Debian sources, and per-package apt guards;
- apt purge/dependency installs, Rustup bootstrap, appimaged cleanup/install, dynamic FUSE package selection, and Nerd Font install;
- third-party apt signing keys, repo files, exact pinning stdin, and package installs with per-package guards;
- Flatpak, Cargo/cargo-binstall, release binaries, Go, FNM/Node, npm, pyenv update/pip, and uv self-update/managed-Python behavior;
- apt/Flatpak/Rustup/Cargo/Go/Node update behavior with command/state guards;
- Stow override/backup, Docker daemon preservation, Docker/VirtualBox groups, VS Code extension idempotency, terminal selection, GNOME settings, extension install/enable, dock keys, and rounded-corner settings.

Config-derived values are passed as process arguments or stdin to fixed command snippets. GitHub, Go, GNOME, and Docker JSON is parsed by internal Rust helpers; `yq` is not required. The remaining shell snippets are static and are used for state checks or tightly scoped workflows that need shell features; YAML values are not interpolated into generated shell source.

## Packaging

`scripts/package-release.sh` builds a deterministic architecture archive containing only:

- `cozydot`

The checksum is published separately. At build time, `configs/default.yaml` and all regular files below `dotfiles/` are sorted, validated, and embedded. Shebang scripts materialize as `0755`; every other asset uses `0644`, so build output does not depend on filesystem-only mode changes. `cozydot init` materializes that immutable snapshot without a checkout, network, archive, or cache. `install.sh` verifies the transport archive, rejects every member except one regular `cozydot`, and atomically replaces only `~/.local/bin/cozydot`; it never provisions user state.

## Safety and Testing

Provisioning side effects cross the existing `Runner` trait. Init uses typed Rust filesystem modules, SHA-256 ownership records, a minimal interruption journal, and symlink-ancestor checks.

Development gates:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
scripts/package-release.sh
bash -n dotfiles/bash/.bashrc
```

Privileged downloads use fail-fast curl options, unique temporary files, and stage destructive replacements only after archive validation succeeds. Go archives are matched against and verified with the official SHA-256 published by `go.dev/dl/?mode=json&include=all`. Other upstream installers and mutable release assets currently provide no checksum in the preset schema; executing those HTTPS-delivered installers/assets remains an explicit trust boundary. The project does not claim end-to-end integrity for them and does not fabricate hashes.
