# Cozydot configuration schema v1

This document is the implementation contract for the breaking configuration rewrite. The parser, validator, planner, embedded presets, documentation, and tests must implement this shape exactly. The legacy tagged schema is not part of v1 and receives no compatibility layer.

## Canonical reference

```yaml
schema: 1

system:
  distro: auto
  desktop: auto
  prepare: true

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
        suite: stable
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
          amd64: Obsidian-1.8.10.AppImage
          arm64: Obsidian-1.8.10-arm64.AppImage

tools:
  rust: stable
  go: latest
  node: lts
  python: "3.13"

fonts:
  nerd:
    - GeistMono

dotfiles:
  conflict: overwrite
  packages:
    - bash
    - starship

integrations:
  docker: true
  virtualbox: true
  vscode_extensions:
    - rust-lang.rust-analyzer

desktop:
  terminal: wezterm
  gnome:
    settings: true
    extensions:
      - blur-my-shell@aunetx
    macos_dock: true
    rounded_corners: true

updates:
  apt:
    full: false
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
- Optional behavior is disabled when its field is omitted, `null`, `false`, or an empty collection. A populated mapping enables only its populated children. `schema` cannot be disabled.
- YAML tags, profiles, inheritance, templates, arbitrary shell commands, and public interpolation variables are invalid. Strings are literal values.
- Each concept has only the representation shown here. A scalar cannot replace a sequence or mapping, and a mapping cannot replace a scalar.
- Cozydot detects its native architecture once and translates it internally. Architecture aliases are not configuration fields or interpolation variables.
- Cozydot infers and installs internal prerequisites for enabled features. Users do not configure prerequisite package lists or select package managers.
- Sequences preserve user order. Duplicate entries in a package sequence or duplicate `name` values in definition sequences are validation errors.
- All names, package identifiers, versions, URLs, repository coordinates, extension IDs, and asset filenames must be non-empty strings. URLs must use HTTPS.

## `system`

`system` controls host detection and baseline distribution preparation.

- `distro` is a scalar. `auto` reads `/etc/os-release`; otherwise it is a supported Debian-family distribution ID. Omission defaults detection to `auto` but does not enable preparation.
- `desktop` is a scalar. `auto` reads the current desktop environment; `none` explicitly selects no desktop; otherwise it is a supported desktop ID. Omission defaults detection to `auto`.
- `prepare` is a boolean. `true` enables Cozydot's fixed distribution preparation, such as repository baseline and conflicting default cleanup. Omission, `null`, or `false` disables it.

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
- `source.suite`: required literal APT suite string, such as `stable` or `noble`.
- `source.components`: required non-empty sequence of literal APT components.
- `packages`: required non-empty sequence of package names installed from the repository.

Cozydot writes the `signed-by` path and detected native Debian architecture into the source entry. Architecture fields, raw `deb` lines, key paths, pinning blocks, and variable substitution are not accepted.

### Direct packages

Every `packages.direct` item has exactly these fields:

- `name`: required stable identifier used for state and update tracking.
- `format`: required scalar, either `deb` or `appimage` in schema v1.
- `provides`: required non-empty sequence of executable names used to determine whether the package is present.
- `source`: required source mapping. Schema v1 supports only the GitHub source below.

A GitHub `source` has exactly these fields:

- `type`: required literal `github`.
- `repository`: required `owner/repository` coordinate.
- `assets`: required map from canonical architecture keys to exact release asset filenames. Allowed keys are `amd64`, `arm64`, `arm32`, and `riscv64`.

At plan time Cozydot selects the asset for the native canonical architecture and fails clearly if that key is absent. Asset values are literal filenames, not globs or templates. Cozydot resolves the latest GitHub release, downloads that exact asset, and installs it with the fixed handler for `format`.

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

- `conflict` is required when `dotfiles` is enabled and is either `overwrite` or `backup`. `overwrite` replaces conflicting target content; `backup` preserves conflicting content before Stow adopts the package.
- `packages` is a required non-empty sequence of directory names below the bundled or active `dotfiles` root.

Dotfiles are applied with Stow. Stow and other internal prerequisites are inferred.

## `integrations`

- `docker` is a boolean enabling Cozydot's fixed post-install Docker user and daemon configuration. Docker software itself must be declared under `packages`.
- `virtualbox` is a boolean enabling fixed VirtualBox user-group configuration. VirtualBox software itself must be declared under `packages`.
- `vscode_extensions` is a sequence of extension IDs installed through an existing VS Code command. VS Code software itself must be declared under `packages`.

Integrations configure installed software; they do not implicitly add the associated product package.

## `desktop`

- `terminal` is a scalar executable name configured as the default terminal on supported desktops.
- `gnome` is a mapping applied only when the detected desktop is GNOME.
- `gnome.settings` is a boolean enabling Cozydot's fixed baseline GNOME settings.
- `gnome.extensions` is a sequence of exact GNOME extension UUIDs.
- `gnome.macos_dock` is a boolean enabling Cozydot's fixed dock layout.
- `gnome.rounded_corners` is a boolean enabling Cozydot's fixed rounded-corner settings.

Desktop behavior that does not match the detected desktop is skipped, not emulated.

## `updates`

`updates` controls update actions independently of initial installation.

- `apt` is a mapping. Its presence enables `apt update` and `apt upgrade`; `apt.full: true` additionally enables full upgrade and purge-autoremove. `full` defaults to `false` inside an enabled `apt` mapping.
- `flatpak` is a boolean enabling Flatpak updates.
- `tools.rust`, `tools.go`, and `tools.node` are booleans enabling updates through Rustup, official Go archives, and FNM respectively.
- `packages.cargo`, `packages.npm`, and `packages.direct` are booleans enabling updates of the corresponding configured package sets.

An update flag only acts on its corresponding configured or installed set. It never enables installation declarations and never selects a different manager.

## Validation boundary

The complete configuration is parsed and validated before host-changing operations begin. Errors include the field path and reject unknown keys, wrong YAML types, unsupported enum values, malformed identifiers, non-HTTPS URLs, duplicate definitions, unsupported architectures, and missing architecture assets. Cozydot does not silently reinterpret invalid input.
