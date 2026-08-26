# Cozydot

Cozydot is an idempotent post-install and dotfile manager for Linux and macOS.
It provisions packages, development tools, dotfiles, integrations, desktop
settings, and updates from one declarative YAML file.

## Why Cozydot?

### Carry one configuration between machines

A new laptop should not require a new setup process. Cozydot keeps the software
and configuration you want in a readable YAML file that can be reused across
machines. The file shows what Cozydot will install and configure without hiding
the setup inside a shell script.

Running Cozydot again converges the host on that configuration. State that is
already correct is left alone, and software you did not configure is not
removed.

### Keep the machine native

Cozydot uses the host's established package managers, configuration locations,
and upstream conventions. APT packages remain APT packages, Homebrew formulae
remain Homebrew formulae, and dotfiles remain ordinary files and links in their
standard locations.

This keeps Cozydot non-intrusive. Removing the Cozydot binary does not leave the
machine dependent on a custom runtime, package store, or configuration layout.
The installed software and configuration remain usable and can still be managed
with their official documentation.

Cozydot supports Debian, Ubuntu, Pop!_OS, and Linux Mint on `x86_64` (`amd64`)
and `aarch64` (`arm64`), plus macOS on Apple Silicon (`aarch64-apple-darwin`).
Other architectures are rejected.

Supported Debian releases are Bookworm and Trixie. On pure Debian, every
`apply` appends `contrib`, `non-free`, and `non-free-firmware` to the selected
conventional source file entries that already contain `main`. Official sources
on Ubuntu and supported derivatives are left unchanged.

## Install

On a supported host:

```bash
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/master/install.sh | bash
```

The installer selects the `amd64` or `arm64` release, verifies its
published SHA-256 file, requires the archive to contain exactly one regular
`cozydot` entry, and atomically installs the binary in `~/.local/bin`.

You can override the release base URL using the `COZYDOT_RELEASE_BASE_URL` environment variable, or pass flags directly to `install.sh` using `bash -s`:

```bash
# Install a specific version
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/master/install.sh | bash -s -- -v 1.0.0

# Install from a mirror
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/master/install.sh | COZYDOT_RELEASE_BASE_URL=https://mirror.example/cozydot bash
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

> The Microsoft core fonts (`ttf-mscorefonts-installer`) are not provisioned
> because their EULA must be accepted interactively. Install manually:
> `sudo apt-get install -y ttf-mscorefonts-installer`

## Configuration sources

`configs/cozydot.yaml` is the manually maintained base preset.
`scripts/generate-configs.sh` derives `configs/cli.yaml` and `configs/vm.yaml`;
do not edit those generated files directly. Builds embed snapshots of all three
presets.

The active `cozydot.yaml` created by `init` is user configuration, not a
generated repository file. Edit that active file and run `cozydot check` to
validate it against the current platform without making changes. `apply`,
`dotfiles`, and `update` load the same active file.

## Apply, dotfiles, and update behavior

`cozydot apply` ensures configured software is present, applies configured
state, and leaves unconfigured software unchanged. It does not upgrade present
software merely because a newer release exists.

`cozydot dotfiles` applies only shared dotfile packages and those configured for
the current platform. It uses Stow's simulation mode to reject destination
conflicts without changing dotfiles. `cozydot dotfiles --replace` (or `-r`) first backs
conflicts up under
`${XDG_STATE_HOME:-$HOME/.local/state}/cozydot/dotfile-backups`, then applies
Cozydot's links. The command requires GNU Stow to be installed and never adopts
destination files into Cozydot's source. `apply` uses the same conservative
conflict behavior.

`cozydot update` runs each enabled update category independently from apply
intent. Flatpak updates installed user applications and runtimes; Cargo updates installed
registry crates; npm updates global packages. Rust ensures the configured or stable
toolchain, then updates all installed rustup toolchains. Selectorless Go, Node, and
Python updates use `latest`, `latest`, and `3` respectively. Font updates still
redownload configured Nerd Font families because fonts have no native manager;
an absent family list is a no-op.

On Linux, `cozydot update` always ensures the base prerequisite packages before
running enabled update categories; on macOS, it always ensures Homebrew. With
an absent, empty, or all-false `updates:` section, that baseline operation is
its only work. `apply` accepts update controls but never executes them. Managed
Deb and AppImage binaries remain ensure-only and have no update category.

`updates.apt: upgrade|full-upgrade` runs `apt-get update`, then performs a system-wide
APT `upgrade` or `full-upgrade`; `full-upgrade` also runs purge-autoremove. This updates
existing APT-managed state only. Run `cozydot apply` first after changing APT
packages or repositories.

Direct APT packages are ensured before third-party repositories. Cozydot publishes
every repository applicable to the detected distribution and optional APT-native
`arch` list, runs `apt-get update` once, purges all installed repository conflicts,
then ensures all repository packages without upgrading installed versions. An omitted `arch`
supports every Cozydot Linux architecture; supported values are `amd64` and
`arm64`.

## Roadmap

- Update managed Deb and AppImage binaries from their configured release sources.
- Complete first-run Xcode Command Line Tools installation before continuing a
  macOS apply.
- Add a dedicated command for listing bundled presets.

## Safety model

- `check`, `apply`, `dotfiles`, and `update` validate the complete active
  configuration against the detected platform. Explicit Linux and macOS
  workflows then execute host operations sequentially in dependency order and
  stop on the first failure.
- YAML selects only the documented schema. It cannot provide arbitrary
  commands, shell fragments, managers, lock paths, plugins, or interpolation;
  execution uses a fixed set of host-operation functions.
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
