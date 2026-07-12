# cozydot

cozydot provisions Debian- and Ubuntu-family Linux systems from one active YAML configuration.

## Install

The installer detects Linux amd64/arm64, verifies the published release checksum, validates the archive, and atomically installs only `~/.local/bin/cozydot`:

```bash
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/main/install.sh | bash
```

For an offline mirror or a pinned release, set `COZYDOT_VERSION` and `COZYDOT_RELEASE_BASE_URL`.

## Use

```bash
cozydot init
$EDITOR "${XDG_CONFIG_HOME:-$HOME/.config}/cozydot/cozydot.yaml"
cozydot apply
```

`init` safely copies the default configuration and dotfiles into the XDG config directory. Files unchanged since cozydot last installed them are refreshed; modified, unmanaged, and obsolete files are preserved. `apply` runs the existing provisioning engine using that active configuration.

`apply` provisions configured software and settings; it does not run the legacy recurring-update phase.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
scripts/package-release.sh
```

Release archives contain `cozydot`, `configs/default.yaml`, and `dotfiles/` at their root. No extracted application directory is required at runtime.
