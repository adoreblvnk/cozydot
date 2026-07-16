# cozydot

cozydot provisions Debian- and Ubuntu-family Linux systems from one active YAML configuration.

## Install

The installer detects Linux amd64/arm64, verifies the published release checksum, requires a one-file release archive, and atomically installs only `~/.local/bin/cozydot`. The binary contains its default configuration and dotfiles:

```bash
curl -fsSL https://raw.githubusercontent.com/adoreblvnk/cozydot/main/install.sh | bash
```

For a local mirror or a pinned release, set `COZYDOT_VERSION` and `COZYDOT_RELEASE_BASE_URL`.

## Use

```bash
cozydot init
$EDITOR "${XDG_CONFIG_HOME:-$HOME/.config}/cozydot/cozydot.yaml"
cozydot apply
```

`init` materializes the version `1.0.0` defaults embedded at build time into the XDG config directory. It needs no repository checkout, network, release archive, or cache. Files unchanged since cozydot last installed them are refreshed; modified, unmanaged, and obsolete files are preserved. `apply` validates the complete file and resolved platform before side effects, then executes the typed plan in dependency order.

The schema and field reference is in [`docs/config-schema-v1.md`](docs/config-schema-v1.md). Managers, lock paths, shell commands, profiles, and plugins are implementation details and cannot be selected from YAML.

Legacy tagged YAML is unsupported and is not converted automatically. See the [0.0.1 release notes](docs/release-notes-v0.0.1.md) before replacing an existing configuration.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
scripts/package-release.sh
```

`configs/default.yaml` and every regular file under `dotfiles/` are the canonical development sources. The build embeds a sorted snapshot in the executable; shebang scripts materialize as `0755` and every other asset as `0644`. Release archives are deterministic transport containing only `cozydot`; their checksum is published separately. The installer verifies transport in a private temporary directory and does not create an XDG cache. The XDG config tree created by `init` is user state.
