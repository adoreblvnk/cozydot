# cozydot

Cozydot provisions packages, development tools, dotfiles, integrations, desktop
settings, and updates on Linux and macOS from one active configuration file.

Cozydot supports Debian, Ubuntu, Pop!_OS, and Linux Mint on `x86_64` (`amd64`),
`aarch64` (`arm64`), and 32-bit ARMv7 (`arm32`), plus macOS on Apple Silicon
(`arm64`). Other architectures are rejected.

Supported Debian releases are Bookworm and Trixie. On pure Debian, every
`apply` appends `contrib`, `non-free`, and `non-free-firmware` to the selected
conventional source file entries that already contain `main`. Official sources
on Ubuntu and supported derivatives are left unchanged.

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
| `COZYDOT_VERSION` | Release version; defaults to `1.0.0` |
| `COZYDOT_RELEASE_BASE_URL` | Release or mirror base URL; defaults to the GitHub releases page |
| `XDG_BIN_HOME` | Install directory; defaults to `~/.local/bin` |

Pass overrides to the shell running the installer, for example:

```bash
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/main/install.sh \
  | COZYDOT_VERSION=1.0.0 COZYDOT_RELEASE_BASE_URL=https://mirror.example/cozydot bash
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
`cozydot init --preset cozydot|cli|vm` to select any bundled preset. It
writes the active configuration and bundled dotfiles under
`${XDG_CONFIG_HOME:-$HOME/.config}/cozydot` without a checkout or network
request.

## Configuration sources

`configs/cozydot.yaml` is the manually maintained base preset.
`scripts/generate-configs.sh` derives `configs/cli.yaml` and `configs/vm.yaml`;
do not edit those generated files directly. Builds embed snapshots of all three
presets.

The active `cozydot.yaml` created by `init` is user configuration, not a
generated repository file. Edit that active file and run `cozydot check` to
validate it without detecting the platform or making changes. `apply` and
`update` load the same active file.

## Apply, dotfiles, and update behavior

`cozydot apply` ensures configured software is present, applies configured
state, and leaves unconfigured software unchanged. It does not upgrade present
software merely because a newer release exists.

`cozydot dotfiles` applies only shared dotfile packages and those configured for
the current platform. It reports every unmanaged destination conflict and exits
without changing dotfiles. `cozydot dotfiles --replace` (or `-r`) first backs
conflicts up under
`${XDG_STATE_HOME:-$HOME/.local/state}/cozydot/dotfile-backups`, then applies
Cozydot's links. The command requires GNU Stow to be installed and never adopts
destination files into Cozydot's source. `apply` uses the same conservative
conflict behavior.

`cozydot update` runs each enabled update category independently from apply
intent. Flatpak updates installed user applications; Cargo updates installed
registry crates; npm updates global packages. Rust updates all installed
rustup toolchains when no selector is configured. Selectorless Go, Node, and
Python updates use `latest`, `latest`, and `3` respectively. Font updates still
redownload configured Nerd Font families because fonts have no native manager;
an absent family list is a no-op.

On Linux, `cozydot update` always ensures the base prerequisite packages before
running enabled update categories. With an absent, empty, or all-false
`updates:` section, that baseline operation is its only work. The same
configuration is a validated silent no-op on macOS. `apply` accepts update
controls but never executes them. Managed Deb and AppImage binaries remain
ensure-only and have no update category.

`updates.apt: upgrade|full-upgrade` runs `apt-get update`, then performs a system-wide
APT `upgrade` or `full-upgrade`; `full-upgrade` also runs purge-autoremove. This updates
existing APT-managed state only. Run `cozydot apply` first after changing APT
packages or repositories.

Direct APT packages are ensured before third-party repositories. Cozydot publishes
every repository applicable to the detected distribution and optional APT-native
`arch` list, runs `apt-get update` once, purges all installed repository conflicts,
then installs all missing repository packages in one operation. An omitted `arch`
supports every Cozydot Linux architecture; supported values are `amd64`, `arm64`,
and `armhf`.

## Safety model

- `apply`, `dotfiles`, and `update` validate the complete active configuration
  against the detected platform before starting side effects. Explicit Linux
  and macOS workflows then execute typed operations sequentially in dependency
  order and stop on the first failure.
- YAML selects only the documented schema. It cannot provide arbitrary
  commands, shell fragments, managers, lock paths, plugins, or interpolation;
  execution uses a fixed set of typed `Operation` variants and executors.
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
