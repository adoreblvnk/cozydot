# cozydot

cozydot provisions Debian- and Ubuntu-family Linux systems from one active YAML configuration.

## Install

The installer detects Linux amd64/arm64, verifies the published release checksum, requires a one-file release archive, and atomically installs only `~/.local/bin/cozydot`. The binary contains its bundled configurations and dotfiles:

```bash
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/main/install.sh | bash
```

For a local mirror or a pinned release, set `COZYDOT_VERSION` and `COZYDOT_RELEASE_BASE_URL`.

## Use

```bash
cozydot init
# Or: cozydot init --preset full|cli|vm
$EDITOR "${XDG_CONFIG_HOME:-$HOME/.config}/cozydot/cozydot.yaml"
cozydot apply
```

`init` materializes the embedded version `1.0.0` `cozydot` preset by default; `--preset` selects `cozydot`, `full`, `cli`, or `vm`. It needs no repository checkout, network, release archive, or cache. Files unchanged since cozydot last installed them are refreshed; modified, unmanaged, and obsolete files are preserved. `apply` validates the complete active file and resolved platform before side effects, then executes the typed plan in dependency order.

The schema and field reference is in [`docs/config-schema-v1.md`](docs/config-schema-v1.md). Managers, lock paths, shell commands, profiles, and plugins are implementation details and cannot be selected from YAML.

## Development

```bash
scripts/generate-configs.sh --check
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
scripts/package-release.sh
```

`configs/cozydot.yaml` is the canonical base. `scripts/generate-configs.sh` deterministically derives `full.yaml`, `cli.yaml`, and `vm.yaml`; edit the base or generator rather than those outputs. The build embeds all four configurations and every regular file under `dotfiles/`. Shebang scripts materialize as `0755` and every other asset as `0644`. Release archives are deterministic transport containing only `cozydot`; their checksum is published separately. The installer verifies transport in a private temporary directory and does not create an XDG cache. The XDG config tree created by `init` is user state.
