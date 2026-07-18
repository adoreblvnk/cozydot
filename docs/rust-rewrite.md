# Rust rewrite

cozydot is a Rust binary with one strict version `1.0.0` configuration. The implementation is split into CLI/config/platform planning and a small set of fixed-operation modules.

## Compatibility

The public CLI is intentionally small:

- `-V` reports `cozydot 0.0.1`.
- `cozydot init [--preset cozydot|full|cli|vm]` creates or safely refreshes the active config and bundled dotfiles.
- `cozydot apply` applies `${XDG_CONFIG_HOME:-$HOME/.config}/cozydot/cozydot.yaml`.
- Profile inheritance, planning, and multi-command execution are not public interfaces.

`apply` parses and validates the complete file, resolves the host platform, plans typed operations directly, and executes them in fixed phases. Unknown fields, wrong types, YAML extensions, interpolation, unsafe paths, malformed identifiers, and unsupported platform combinations fail with contextual field paths.

Managers and repository paths are fixed implementation choices. YAML cannot select commands, shell source, managers, lock paths, plugins, profiles, or interpolation variables.

## Behavior Covered

The typed planner and operations cover:

- explicit managed or preserved APT sources, administrative membership, unattended upgrades, Ubuntu Snap/codecs, package removal/install, repositories, and scoped updates;
- per-user Flathub applications, Rustup/Rust, official Go archives, FNM/Node, UV/Python, Cargo, NPM, managed Debian/AppImage binaries, and Nerd Fonts;
- one backup-before-Stow policy, existing-product-only Docker/VirtualBox/VS Code integrations, GNOME/Cinnamon settings, GNOME extensions, dock, and rounded corners.

Config-derived values are passed as arguments to typed fixed operations. GitHub, Go, GNOME, NPM, and Docker state is parsed by internal Rust helpers; `yq` and generated shell source are not used at runtime.

## Packaging

`scripts/package-release.sh` builds a deterministic architecture archive containing only:

- `cozydot`

The checksum is published separately. At build time, all four files below `configs/` and all regular files below `dotfiles/` are sorted and embedded. `configs/cozydot.yaml` is the canonical base; `scripts/generate-configs.sh` derives the other three presets and checks generated-file drift. Shebang scripts materialize as `0755`; every other asset uses `0644`, so build output does not depend on filesystem-only mode changes. `cozydot init` materializes the selected immutable snapshot without a checkout, network, archive, or cache. `install.sh` verifies the transport archive, rejects every member except one regular `cozydot`, and atomically replaces only `~/.local/bin/cozydot`; it never provisions user state.

## Safety and Testing

Provisioning side effects are dispatched by matching each planned typed `Operation` and invoking its fixed Rust executor. Cozydot is binary-only, so raw operations and executable steps are not an external API. Init uses typed Rust filesystem modules, SHA-256 ownership records, a minimal interruption journal, and symlink-ancestor checks.

Development gates:

```bash
scripts/generate-configs.sh --check
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
scripts/package-release.sh
bash -n scripts/generate-configs.sh dotfiles/bash/.bashrc
```

Privileged downloads use fail-fast curl options, unique temporary files, and stage destructive replacements only after archive validation succeeds. Go archives are matched against and verified with the official SHA-256 published by `go.dev/dl/?mode=json&include=all`. Fixed-URL binary sources require configured SHA-256 values, and GitHub binaries verify configured and API-published digests when available. Upstream manager installers and GitHub assets without an available digest remain explicit HTTPS trust boundaries; the project does not fabricate hashes for them.
