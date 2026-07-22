# cozydot

Cozydot is a Linux bootstrapper that provisions packages, development tools,
dotfiles, integrations, desktop settings, and updates from one active YAML
configuration.

Cozydot supports Debian, Ubuntu, Pop!_OS, and Linux Mint on `x86_64` (`amd64`),
`aarch64` (`arm64`), and 32-bit ARMv7 (`arm32`). Other architectures are
rejected.

## Install

On a supported host:

```bash
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/main/install.sh | bash
```

The installer selects the `amd64`, `arm64`, or `arm32` release, verifies its
published SHA-256 file, requires the archive to contain exactly one regular
`cozydot` entry, and atomically installs the binary in the configured install
directory.

These environment variables override the defaults:

| Variable | Purpose |
| --- | --- |
| `COZYDOT_VERSION` | Release version; defaults to `0.0.1` |
| `COZYDOT_RELEASE_BASE_URL` | Release or mirror base URL; defaults to the GitHub releases page |
| `XDG_BIN_HOME` | Install directory; defaults to `~/.local/bin` |

Pass overrides to the shell running the installer, for example:

```bash
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/main/install.sh \
  | COZYDOT_VERSION=0.0.1 COZYDOT_RELEASE_BASE_URL=https://mirror.example/cozydot bash
```

## First run

```bash
cozydot init
$EDITOR "${XDG_CONFIG_HOME:-$HOME/.config}/cozydot/cozydot.yaml"
cozydot apply
# Optional: converge update-enabled configured targets to their latest allowed versions.
cozydot update
```

`init` defaults to the embedded `cozydot` preset. Use
`cozydot init --preset cozydot|full|cli|vm` to select any bundled preset. It
writes the active config and bundled dotfiles under
`${XDG_CONFIG_HOME:-$HOME/.config}/cozydot` without a checkout or network
request.

## Configuration sources

`configs/cozydot.yaml` is the canonical, manually maintained base preset.
`scripts/generate-configs.sh` derives `configs/full.yaml`, `configs/cli.yaml`,
and `configs/vm.yaml`; do not edit those generated files directly. Builds embed
snapshots of all four presets.

The active `cozydot.yaml` created by `init` is user configuration, not a
generated repository file. Edit that active file before running `apply` or
`update`.

## Apply and update behavior

| Command | Configured and missing | Configured and present | Unconfigured |
| --- | --- | --- | --- |
| `cozydot apply` | Installs | Leaves unchanged, even when outdated | Leaves unchanged |
| `cozydot update` | Installs the latest allowed version when its update category is enabled | Checks and updates to the latest allowed version when its category is enabled | Leaves unchanged |

An absent or empty `updates:` section, or one containing only false controls,
makes `cozydot update` a validated silent no-op. `apply` validates update
controls but never executes update operations. Update categories cover
configured Flatpaks, Rust/Node/Go/Python toolchains, Cargo/npm packages, and
Nerd Font families. Explicit font updates redownload configured families; no
release receipt is retained. Managed Deb and AppImage binaries remain
ensure-only and have no update category.

`updates.apt: standard|full` is the documented exception to configured-only
updates: it converges applicable repositories needed by configured APT targets,
installs configured missing APT packages, then intentionally performs a
system-wide APT `upgrade` or `full-upgrade`; `full` also runs purge-autoremove.
These commands run only from `cozydot update`.

## Safety model

- `apply` and `update` validate the complete active config and resolved
  platform, then plan their complete ordered typed-operation workflows before
  starting side effects.
- YAML selects only the documented schema. It cannot provide arbitrary
  commands, shell fragments, managers, lock paths, plugins, or interpolation;
  execution uses a fixed set of typed operations.
- `init` tracks the files it writes. Later runs refresh missing or unchanged
  init-managed files while preserving user-edited, unmanaged, and obsolete
  files.
- Release packaging emits a deterministic one-binary archive and a separate
  checksum. The installer verifies that transport before replacing the binary.
  This does not imply that every upstream package or manager download has a
  checksum.

## Development

Development requires the latest stable Rust toolchain with Rustfmt and Clippy. The config generator also requires `yq` v4.

```bash
scripts/generate-configs.sh --check
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
scripts/package-release.sh
bash -n install.sh scripts/generate-configs.sh scripts/package-release.sh dotfiles/bash/.bashrc
```

`scripts/package-release.sh` performs its release build with `--locked` and
writes the archive and checksum under `target/` by default.
