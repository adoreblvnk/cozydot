# Rust rewrite

cozydot is now a single Rust binary; the legacy root Bash executable has been removed. The implementation is split into CLI entrypoint, tagged-YAML config access, platform mappings, command planning, and process execution.

## Compatibility

All four presets are parsed directly with `serde_yaml`, including `!enabled` and `!disabled` tagged scalars, sequences, and mappings. Repository variables (`UPSTREAM_DISTRO`, `VERSION_CODENAME`, and release architecture variants) are expanded from detected platform data. Existing repository-owned download expressions remain passed to narrowly generated Bash pipeline steps where a GitHub API lookup is required; user input is otherwise passed as process arguments or stdin rather than re-evaluated.

The command planner covers:

- distro preparation, apt purge/dependencies, Rustup, appimaged, and Nerd Fonts;
- apt packages, third-party signing keys/repos/pinning, Flatpak, Cargo via cargo-binstall, release binaries, Go, FNM/Node, global npm packages, pyenv, and uv;
- apt/Flatpak/Rustup/Cargo/yq/Go/Node updates;
- Stow override/backup flows, Docker and VirtualBox groups, VS Code extensions, terminal selection, and GNOME settings/extensions/dock preferences.

Cargo-binstall bootstrap is intentionally infrastructure, not YAML, and always precedes Cargo package operations. FNM replaces NVM. The existing `install.npm` list includes `opencode-ai`.

## Safety and testing

Every side effect crosses the `Runner` trait. Production uses `ProcessRunner`; tests inspect plans or use the recording/dry-run seam and never invoke package managers. `COZYDOT_DRY_RUN=1 cozydot -c full install` is the supported local plan inspection mode.

Development gates:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
bash -n dotfiles/bash/.bashrc
```

Real package-manager and desktop integration is intentionally not exercised in CI because it would mutate the runner host. Plan tests cover ordering, disabled sections, mappings, presets, aliases, and integrated package-manager behavior.
