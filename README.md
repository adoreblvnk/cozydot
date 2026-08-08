# cozydot

Cozydot provisions packages, development tools, dotfiles, integrations, desktop
settings, and updates on Linux and macOS from one active YAML configuration.

Cozydot supports Debian, Ubuntu, Pop!_OS, and Linux Mint on `x86_64` (`amd64`),
`aarch64` (`arm64`), and 32-bit ARMv7 (`arm32`), plus macOS on Intel and Apple
silicon. Other architectures are rejected.

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
cozydot check
cozydot apply
# Optional: run the enabled ecosystem updates.
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
generated repository file. Edit that active file and run `cozydot check` to
validate it without detecting the platform or making changes. `apply` and
`update` load the same active file.

## Apply, dotfiles, and update behavior

`cozydot apply` installs configured missing targets and leaves present or
unconfigured software unchanged, even when it is outdated.

`cozydot dotfiles` applies only the configured shared and current-platform
dotfile packages. It reports every unmanaged destination conflict and exits
without changing dotfiles. `cozydot dotfiles --replace` (or `-r`) first backs
conflicts up under
`${XDG_STATE_HOME:-$HOME/.local/state}/cozydot/dotfile-backups`, then applies
Cozydot's links. The command requires GNU Stow to be installed and never adopts
destination files into Cozydot's source. `apply` uses the same conservative
conflict behavior.

`cozydot update` runs each enabled update category independently from apply
targets. Flatpak updates installed user applications; Cargo updates installed
registry crates; npm updates global packages. Rust updates all installed
rustup toolchains when no selector is configured. Selectorless Go, Node, and
Python updates use `latest`, `latest`, and `3` respectively. Font updates still
redownload configured Nerd Font families because fonts have no native manager;
an absent family list is a no-op.

An absent or empty `updates:` section, or one containing only false controls,
makes `cozydot update` a validated silent no-op. `apply` accepts update controls
but never executes them. Managed Deb and AppImage binaries remain ensure-only
and have no update category.

`updates.apt: standard|full` converges applicable repositories, installs
configured missing APT packages, then performs a system-wide APT `upgrade` or
`full-upgrade`; `full` also runs purge-autoremove. These commands run only from
`cozydot update`.

Direct APT packages are ensured before packages owned by third-party
repositories. A repository may declare distro-selected `conflicts`; after that
repository is published and APT metadata is refreshed, Cozydot purges only
installed conflicts before ensuring the repository's packages. Conflict names
must differ from repository target names. This makes repository migrations such
as `docker.io` to `docker-ce` no-ops on a second `apply` while keeping all
destructive removal tied to its replacement repository. There is no global APT
remove list.

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
