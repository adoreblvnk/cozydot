# Cozydot configuration schema v1

This document is the implementation contract for the breaking configuration rewrite. The parser, validator, planner, embedded presets, documentation, and tests must implement this shape exactly. The legacy tagged schema is not part of v1 and receives no compatibility layer.

## Canonical reference

```yaml
schema: 1

system:
  distro: auto
  desktop: auto
  ensure_admin: true
  apt:
    sources: managed
    components:
      - main
    unattended_upgrades: false
  ubuntu:
    snap: false
    codecs: true

packages:
  remove:
    - docker.io
  apt:
    - curl
    - git
    - stow
  repositories:
    - name: github-cli
      key: https://cli.github.com/packages/githubcli-archive-keyring.gpg
      source:
        urls:
          default: https://cli.github.com/packages
        suite: system
        components:
          - main
      packages:
        - gh
  flatpak:
    - com.bitwarden.desktop
  cargo:
    - bat
    - starship
  npm:
    - opencode-ai
  direct:
    - name: obsidian
      format: appimage
      provides:
        - obsidian
      source:
        type: github
        repository: obsidianmd/obsidian-releases
        assets:
          amd64:
            include: "Obsidian-*.AppImage"
            exclude:
              - "Obsidian-*-arm64.AppImage"
          arm64:
            include: "Obsidian-*-arm64.AppImage"
            exclude: []

tools:
  rust: stable
  go: latest
  node: lts
  python: "3.13"

fonts:
  nerd:
    - GeistMono

dotfiles:
  packages:
    - bash
    - starship

integrations:
  docker:
    add_user_to_group: true
    local_log_driver: true
    max_log_size: 10m
  virtualbox:
    add_user_to_group: true
  vscode:
    extensions:
      - rust-lang.rust-analyzer

desktop:
  theme: dark
  terminal: wezterm
  idle:
    timeout: 15m
    dim: false
  gnome:
    extensions:
      - blur-my-shell@aunetx
    dock: true
    rounded_corners: true

updates:
  apt: standard
  flatpak: true
  tools:
    rust: true
    go: true
    node: true
  packages:
    cargo: true
    npm: true
    direct: true
```

The reference demonstrates every field. It is one configuration, not a promise that every listed package or asset is available on every supported distribution or architecture.

## Global rules

- `schema` is required, must be the integer `1`, and is the only required top-level field.
- The only top-level fields are `schema`, `system`, `packages`, `tools`, `fonts`, `dotfiles`, `integrations`, `desktop`, and `updates`. Unknown fields are errors at every level.
- Omission or `null` leaves the corresponding host feature unchanged unless a field documents a detection default. Empty collections schedule no entries. Boolean `false` is meaningful for controls that explicitly manage an on/off host state and otherwise schedules no action; each field below states its behavior. `schema` cannot be omitted or disabled.
- YAML tags, profiles, inheritance, templates, arbitrary shell commands, and public interpolation variables are invalid. Strings are literal values.
- Each concept has only the representation shown here. A scalar cannot replace a sequence or mapping, and a mapping cannot replace a scalar.
- Cozydot detects its native architecture once and translates it internally. Schema v1 supports amd64, arm64, Armv7/armhf, and riscv64 hosts. The Armv7/armhf family accepts machine values `arm32`, `armv7`, `armv7l`, and `armhf`, but not `armv6l`; Go's official `armv6l` archive name remains an internal output translation. Architecture aliases are not configuration fields or interpolation variables.
- Cozydot infers and installs internal prerequisites for enabled features. Users do not configure prerequisite package lists or select package managers.
- Sequences preserve user order. Duplicate entries in a package sequence or duplicate `name` values in definition sequences are validation errors.
- All names, package identifiers, versions, URLs, repository coordinates, extension IDs, and asset patterns must be non-empty strings. URLs must use HTTPS.

## `system`

`system` controls host detection and specific distribution preparation. It is not an opaque preparation switch.

- `distro` is one of `auto`, `ubuntu`, `linuxmint`, `pop`, `zorin`, `deepin`, `debian`, `kali`, or `tails`. These canonical configured and detected IDs are lowercase. `auto` reads `/etc/os-release`. Omission defaults detection to `auto`.
- `desktop` is one of `auto`, `none`, `gnome`, or `cinnamon`. `auto` reads the current desktop environment and resolves to one of the other three values. `none` explicitly selects no desktop. Omission defaults detection to `auto`.
- `ensure_admin` is a boolean. `true` ensures the invoking user belongs to the distribution's administrative group (`sudo` on supported Debian-family systems); `false`, omission, or `null` does not change group membership.
- `apt` is a mapping containing only `sources`, `components`, and `unattended_upgrades`.
- `apt.sources` is either `preserve` or `managed`. `preserve` leaves distribution-owned APT source files untouched. `managed` writes Cozydot's canonical source set for the detected distribution and codename. Omission or `null` is equivalent to `preserve`.
- `apt.components` is a non-empty sequence selected from `main`, `contrib`, `non-free`, `non-free-firmware`, `restricted`, `universe`, and `multiverse`, without duplicates. It supplies the components for `apt.sources: managed`; components unsupported by the detected distribution are validation errors. It is invalid when sources are omitted or `preserve`.
- `apt.unattended_upgrades` is a boolean host-state control. `true` installs and enables the distribution's unattended-upgrade service and periodic configuration. `false` disables its periodic configuration and removes the `unattended-upgrades` package. Omission or `null` preserves the current state.
- `ubuntu` is a mapping containing only `snap` and `codecs`. Its populated controls apply when the normalized upstream family is Ubuntu, including supported Ubuntu derivatives. They are skipped for non-Ubuntu-family distributions and are not emulated there.
- `ubuntu.snap` is a boolean host-state control. `true` ensures Ubuntu's `snapd` package and service are enabled. `false` removes installed snaps and `snapd`, disables its services, removes its data directories, and installs Cozydot's no-Snap APT pin. Omission or `null` preserves the current state.
- `ubuntu.codecs` is an enable-only boolean. `true` installs `ubuntu-restricted-extras`; `false`, omission, or `null` does not install or remove codecs.

Detection is input to planning, not optional host mutation. An unsupported detected or configured distribution fails before execution.

## `packages`

All software installation and removal is declared under `packages`. There is no top-level `apps` section.

- `remove` is a sequence of APT package names to purge. Cozydot may repeat this idempotently on later applies.
- `apt` is a sequence of native APT package names.
- `repositories` is a sequence of third-party APT repository definitions.
- `flatpak` is a sequence of Flatpak application IDs installed from Cozydot's fixed Flathub remote.
- `cargo` is a sequence of Cargo crate names installed with cargo-binstall. Command-line fragments and per-entry manager choices are invalid.
- `npm` is a sequence of NPM package names installed with the Node version managed by FNM.
- `direct` is a sequence of direct package definitions.

APT metadata is refreshed once before the first enabled APT action. Missing internal prerequisites are inferred from enabled behavior and are not added to `packages.apt` in the effective user configuration.

### APT repositories

Every `packages.repositories` item has exactly these fields:

- `name`: required stable identifier. Cozydot derives keyring and source-list filenames from it.
- `key`: required HTTPS URL for the repository signing key.
- `source`: required mapping containing exactly `urls`, `suite`, and `components`.
- `source.urls`: required map of HTTPS base URLs. Keys are `default` and/or supported distro IDs. Cozydot selects the detected distro key first, then `default`, and errors when neither exists.
- `source.suite`: required scalar. The semantic literal `system` resolves internally to the detected distribution codename. Any other non-empty value is a fixed literal suite, such as `stable` or `squeeze`; it is never interpolated.
- `source.components`: required non-empty sequence of literal APT components.
- `packages`: required non-empty sequence of package names installed from the repository.

Cozydot writes the `signed-by` path and detected native Debian architecture into the source entry. Architecture fields, raw `deb` lines, key paths, pinning blocks, and variable substitution are not accepted.

### Direct packages

Every `packages.direct` item has exactly these fields:

- `name`: required stable identifier used for state and update tracking.
- `format`: required scalar, either `deb` or `appimage` in schema v1.
- `provides`: required non-empty sequence of unique executable names used together to determine whether the package is present. A package may expose more than one executable, so a scalar form is invalid.
- `source`: required source mapping. Schema v1 supports only the GitHub source below.

A GitHub `source` has exactly these fields:

- `type`: required literal `github`.
- `repository`: required `owner/repository` coordinate.
- `assets`: required map from canonical architecture keys to asset selector mappings. Allowed keys are `amd64`, `arm64`, `arm32`, and `riscv64`; these keys accommodate upstream naming and are not interpolation variables.

Each architecture value is one mapping with exactly two required children: `include`, one anchored wildcard pattern, and `exclude`, a sequence of zero or more anchored wildcard patterns. Every pattern must contain `*` or `?`, may use only those two wildcard operators, and matches an entire asset filename (`*` matches zero or more characters and `?` matches exactly one). Paths, character classes, malformed wildcard syntax, interpolation, and substitutions are invalid. Scalar selectors and selectors missing either canonical child are invalid.

At plan time Cozydot selects the mapping for the native canonical architecture and fails clearly if that key is absent. Cozydot resolves the latest GitHub release, matches asset filenames against `include`, removes every asset matching any `exclude` pattern, and requires exactly one remaining asset. Zero or multiple remaining assets fail with the package name, architecture, selector, and match count. Cozydot downloads the sole match and installs it with the fixed handler for `format`.

## `tools`

Each tool has one scalar representation. A present non-null scalar installs or selects the tool through its fixed manager.

- `rust`: Rustup toolchain name or version, for example `stable`.
- `go`: `latest` or an exact Go version, installed from official Go archives.
- `node`: `lts`, `latest`, or an exact Node version, managed by FNM.
- `python`: an exact Python major/minor or patch version string, managed by UV. Quote values such as `"3.13"` so YAML cannot coerce them to numbers.

The managers are not configurable. Rustup, cargo-binstall, official Go archives, FNM, NPM, and UV are implementation choices.

## `fonts`

- `nerd` is a sequence of Nerd Font family names. Each listed family is installed using Cozydot's fixed Nerd Fonts source and destination.

## `dotfiles`

- `packages` is a required non-empty sequence of directory names below the bundled or active `dotfiles` root.

Dotfiles are applied with Stow. Cozydot owns one fixed conflict policy: before adoption, every conflicting target is moved to a timestamped backup under Cozydot's state directory, preserving its relative path; Cozydot never silently overwrites or deletes it. Failure to complete the backup aborts that package before Stow runs. Users select packages only; conflict-policy fields are invalid. Stow and other internal prerequisites are inferred.

## `integrations`

- `docker` is a mapping containing only `add_user_to_group`, `local_log_driver`, and `max_log_size`. Boolean shorthand is invalid.
- `docker.add_user_to_group` is a boolean. `true` ensures the invoking user belongs to the `docker` group; `false`, omission, or `null` leaves membership unchanged.
- `docker.local_log_driver` is a boolean. `true` sets Docker's daemon-wide log driver to `local`; `false`, omission, or `null` leaves the configured driver unchanged.
- `docker.max_log_size` is a Docker size string matching a positive integer followed by `k`, `m`, or `g`, for example `10m`. It sets the `local` driver's `max-size` option and is valid only with `local_log_driver: true`; omission leaves that option unchanged.
- `virtualbox` is a mapping containing only `add_user_to_group`. Boolean shorthand is invalid. `virtualbox.add_user_to_group: true` ensures the invoking user belongs to `vboxusers`; `false`, omission, or `null` leaves membership unchanged.
- `vscode` is a mapping containing only `extensions`. `vscode.extensions` is a sequence of unique extension IDs installed through an existing VS Code command; scalar and top-level integration shorthand forms are invalid.

Integrations configure installed software; they do not implicitly add the associated product package.

## `desktop`

- `theme` is either `light` or `dark` and applies the corresponding supported desktop color preference. Omission or `null` preserves the current preference.
- `terminal` is a non-empty executable name configured as the default terminal on supported desktops. It is not restricted to a package catalogue. Omission or `null` preserves the current default.
- `idle` is a mapping containing only `timeout` and `dim`.
- `idle.timeout` is a scalar duration string consisting of a non-negative integer followed by exactly one unit: `s`, `m`, or `h`. For example, `15m` sets a fifteen-minute timeout and `0s` disables it. Numeric YAML values, negative values, multiple units, and other duration forms are invalid. Omission or `null` preserves the current timeout.
- `idle.dim` is a boolean host-state control. `true` enables dimming when idle and `false` disables it. Omission or `null` preserves the current setting.
- `gnome` is a mapping applied only when the detected desktop is GNOME.
- `gnome.extensions` is a sequence of unique exact GNOME extension UUIDs to install and enable. Omission or an empty sequence manages no extensions.
- `gnome.dock` is an enable-only boolean. `true` applies Cozydot's fixed dock layout; `false`, omission, or `null` leaves dock settings unchanged.
- `gnome.rounded_corners` is an enable-only boolean. `true` applies Cozydot's fixed rounded-corner settings; `false`, omission, or `null` leaves those settings unchanged.

Desktop behavior that does not match the detected desktop is skipped, not emulated.

## `updates`

`updates` controls update actions independently of initial installation. The leaves remain granular; enabling one does not enable its siblings.

- `apt` is one scalar string policy: `off`, `standard`, or `full`. `off` is the explicit disabled value; omission or `null` is equivalent to `off` and schedules no APT update. `standard` runs metadata refresh followed by the normal upgrade. `full` runs those steps followed by full upgrade and purge-autoremove. YAML booleans, mappings, and other scalar values are invalid.
- `flatpak` is a boolean enabling Flatpak updates.
- `tools.rust`, `tools.go`, and `tools.node` are booleans enabling updates through Rustup, official Go archives, and FNM respectively.
- `packages.cargo`, `packages.npm`, and `packages.direct` are booleans enabling updates of the corresponding configured package sets.

An update flag only acts on its corresponding configured or installed set. It never enables installation declarations and never selects a different manager.

## Validation boundary

The complete configuration is parsed and validated before host-changing operations begin. Errors include the field path and reject unknown keys, wrong YAML types, unsupported enum values, malformed identifiers, non-HTTPS URLs, duplicate definitions, unsupported architectures, missing native architecture selectors, and missing selector children. Cozydot does not silently reinterpret invalid input.
