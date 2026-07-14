# Cozydot Configuration Version 1.0.0 — Approved Contract

> Approved implementation contract for Cozydot's sole public configuration format.

## 1. Status

- Version `1.0.0` is a clean, breaking replacement contract.
- Superseded configuration formats are not parsed, converted, migrated, or accepted at runtime.
- There are no aliases for superseded field names.
- Superseded and legacy tagged formats are evidence for requirements only.
- This contract defines user-visible configuration, not implementation structure.

## 2. Product goals

1. A beginner can understand the generated configuration without learning Cozydot internals.
2. An advanced user can represent the complete supported workstation state without shell commands.
3. Every concept has one canonical YAML representation.
4. Omission means preserve or do nothing.
5. Potentially destructive behavior is explicit.
6. Configuration is declarative, deterministic, validated before mutation, and idempotent.
7. Package-manager ownership, update scope, platform applicability, and artifact selection are visible.
8. Cozydot chooses safe execution mechanics; users choose desired software and supported settings.

## 3. Explicit non-goals

Version `1.0.0` does not provide:

- arbitrary commands or shell fragments;
- interpolation or environment-variable expansion;
- YAML tags, directives, anchors, aliases, or multiple documents;
- profiles, inheritance, imports, includes, templates, or condition expressions;
- user-selectable package managers or manager flags;
- hooks, plugins, rollback policy, or arbitrary file writes;
- raw APT source lines, configurable keyring paths, or configurable privileged destinations;
- automatic migration or compatibility parsing for superseded formats;
- generic archives or installer scripts in `packages.binaries`.

## 4. Canonical top-level structure

```yaml
version: 1.0.0
system: ...
packages: ...
tools: ...
fonts: ...
dotfiles: ...
integrations: ...
desktop: ...
updates: ...
```

All sections except `version` are optional. A present section must contain at least one effective child. Explicit `null`, empty mappings, and empty sequences are invalid; omit the field instead.

## 5. Global YAML and validation rules

- `version` is required and must be the semantic version `1.0.0`.
- Only exactly `1.0.0` is accepted. Every other value or scalar shape is rejected before typed deserialization with a direct unsupported-version message.
- Unknown fields are rejected recursively.
- Duplicate mapping keys are rejected.
- Scalar, sequence, and mapping shapes are exact; no shorthand alternatives exist.
- YAML booleans must be actual `true` or `false` values.
- All user strings are literal. Substitution-looking text remains literal and is rejected where it violates the field grammar.
- Every identifier, URL, duration, version, package name, repository coordinate, wildcard selector, and cross-field relationship is validated before platform mutation.
- Platform-independent validation runs while loading. Platform-aware validation runs once after detection and before planning.
- Errors contain the complete YAML field path and the invalid value or missing requirement.

### 5.1 Canonical scalar grammars

- **Definition, Nerd Font family, and repository names:** start and end with an ASCII alphanumeric and contain only ASCII alphanumerics, `.`, `_`, or `-`. Names are unique in their scope. Repository names also derive unique lowercase filename stems by replacing interior runs of `.`, `_`, and `-` with one `-`.
- **Executable basenames:** start with an ASCII alphanumeric and contain only ASCII alphanumerics, `.`, `_`, `+`, or `-`; `/`, `\\`, whitespace, and shell metacharacters are invalid.
- **Dotfile package directories:** use the definition-name grammar and denote exactly one child of Cozydot's active dotfiles root; `.` and `..` are invalid.
- **Debian package names:** start with a lowercase ASCII letter or digit and contain only lowercase ASCII letters, digits, `+`, `.`, or `-`.
- **Cargo package names:** start alphanumeric and contain only ASCII alphanumerics, `_`, or `-`. **NPM package names:** unversioned lowercase `name` or `@scope/name`, with lowercase ASCII letters, digits, `.`, `_`, or `-` in each non-empty component.
- **Flatpak IDs:** at least three dot-separated components, each starting with an ASCII letter and containing only ASCII alphanumerics or `_`. **VS Code IDs:** exactly lowercase `publisher.extension`, with each component starting alphanumeric and otherwise containing ASCII alphanumerics or `-`.
- **GNOME extension UUIDs:** exactly two non-empty ASCII identifier components separated by one `@`; each component contains only ASCII alphanumerics, `-`, `_`, or `.`.
- **APT suite/component tokens:** start with a lowercase ASCII letter or digit and otherwise contain lowercase ASCII letters, digits, `.`, `_`, `+`, or `-`; the complete literal `*` is the only exception. `system` is reserved for the suite field.
- **Exact repository paths:** `./` or one or more relative definition-name segments separated by `/` and ending in `/`; absolute paths, empty interior segments, `.`/`..` segments, backslashes, options, and substitutions are invalid.
- **Durations:** a non-negative decimal integer followed by exactly one lowercase unit `s`, `m`, or `h`. **Docker sizes:** a positive decimal integer followed by exactly one lowercase unit `k`, `m`, or `g`.
- **Rust selectors:** `stable`, `beta`, `nightly`, valid `nightly-YYYY-MM-DD`, or two/three numeric components. **Go:** `latest` or two/three numeric components. **Node:** `lts`, `latest`, or one to three numeric components. **Python:** two/three numeric components.
- **GitHub coordinates:** exactly `owner/repository`; owners start/end alphanumeric and otherwise contain alphanumerics or `-`, while repository names contain alphanumerics, `-`, `_`, or `.` and are not solely dots.
- **Asset selectors:** non-empty whole-filename patterns that contain at least one `*` or `?`; those are the only operators. Paths, character classes, braces, control characters, backticks, `$`, and substitutions are invalid.
- **SHA-256:** exactly 64 lowercase hexadecimal characters. HTTPS URLs use canonical parsed URL values with a valid host and no credentials or fragment.

## 6. `system`

Canonical shape:

```yaml
system:
  require:
    distros: [ubuntu, debian]
    desktops: [gnome]
  ensure_admin: true
  apt:
    sources:
      mode: managed
      components:
        ubuntu: [main, restricted, universe, multiverse]
        debian: [main, contrib, non-free, non-free-firmware]
    unattended_upgrades: disabled
  ubuntu:
    snap: disabled
    codecs: installed
```

### 6.1 Platform detection and requirements

- Platform detection is always automatic; version `1.0.0` does not expose redundant `auto` values.
- `require.distros` is an optional non-empty allowlist of canonical distro IDs: `ubuntu`, `linuxmint`, `pop`, `zorin`, `deepin`, `debian`, `kali`, or `tails`.
- `require.desktops` is an optional non-empty allowlist containing `none`, `gnome`, and/or `cinnamon`.
- Omitted requirement lists accept any platform Cozydot supports. A present list that does not contain the detected value fails before mutation.
- Any present backend-neutral `desktop.theme`, `desktop.terminal`, or `desktop.idle` intent requires the resolved desktop to be `gnome` or `cinnamon`. A resolved `none` or unsupported desktop is rejected during platform-aware validation; desktop intent is never silently skipped or deferred to lowering.
- Cozydot retains detected distro, distro family, upstream distro, detected codename, and upstream codename as separate internal facts.
- Repository URL selection checks exact distro, then upstream distro, then `default`.
- Repository `suite: system` resolves to the codename belonging to the selected exact-distro or upstream-distro URL key. It is invalid when URL selection reaches `default`, because `default` carries no repository-family identity. This avoids using a Linux Mint or Pop!_OS codename against an Ubuntu repository.

### 6.2 Administrative membership

- `ensure_admin: true` ensures the invoking user belongs to the platform’s administrative group.
- Omission leaves membership unchanged. Explicit `false` is invalid because it would duplicate omission without removing membership.
- It never removes administrative membership.

### 6.3 Official APT sources

- `system.apt.sources.mode`: `preserve` or `managed`.
- `preserve` does not rewrite official distro sources.
- `managed` reconciles supported official binary sources to Cozydot’s canonical definitions.
- Managed official-source reconciliation is supported only for pure Ubuntu, pure Debian, and Kali. Linux Mint/LMDE, Pop!_OS, Zorin, Deepin, and Tails require `preserve` until a distro-specific migrator exists.
- Managed mode owns only Cozydot's canonical base-source definitions. It preserves unrelated vendor, entitlement, cloud/LAN mirror, source-package, local/removable-media, and third-party repository state.
- `components` is required with `managed` and forbidden with `preserve`.
- `components` is a mapping keyed by canonical distro IDs and/or `default`. Selection checks the exact distro, then its upstream distro, then `default`.
- Every selected component list is non-empty and validated for that resolved distro family. Ubuntu accepts only `main`, `restricted`, `universe`, and `multiverse`; Debian and Kali accept only `main`, `contrib`, `non-free`, and `non-free-firmware`.
- Source reconciliation backs up every replaced file before mutation, preserves local/file/removable-media transports, publishes atomically, and never follows destination symlinks.

### 6.4 Unattended upgrades

- `unattended_upgrades`: `enabled` or `disabled`.
- Omission preserves current state.
- No boolean form is accepted because `false` would obscure an active disable operation.

### 6.5 Ubuntu controls

- The built-in `ubuntu` section is applied only on Ubuntu-family hosts and is skipped on other allowed distros. This is fixed platform applicability, not a general condition language.
- `snap: enabled` installs/enables Snap support.
- `snap: disabled` removes installed snaps and Snap support, disables associated services, removes managed Snap data, and applies Cozydot’s no-Snap APT policy.
- Omission preserves Snap state.
- `codecs: installed` ensures the supported Ubuntu codec package set is installed.
- Omission preserves codec state. Version `1.0.0` does not claim reliable codec removal.

## 7. `packages`

Canonical hierarchy:

```yaml
packages:
  apt:
    remove: [...]
    install: [...]
    repositories: [...]
  flatpak: [...]
  cargo: [...]
  npm: [...]
  binaries: [...]
```

APT concerns are grouped under `packages.apt`; `remove` is therefore never mistaken for a manager-independent removal mechanism.

### 7.1 APT removal and installation

- `packages.apt.remove`: Debian package names to purge before configured APT installation.
- `packages.apt.install`: package names installed from official or already configured sources.
- Entries are package names only. Versions, manager flags, wildcards, paths, and command fragments are invalid.
- Duplicates within and across APT installation groups are rejected when they create ambiguous ownership.
- Cozydot infers and owns internal prerequisites such as HTTPS certificates, GnuPG, Flatpak, Stow, archive tools, and desktop helpers. Users do not list prerequisites unless they independently want those packages.

### 7.2 Custom APT repositories

Canonical suite/components form:

```yaml
- name: wezterm
  key: https://apt.fury.io/wez/gpg.key
  urls:
    default: https://apt.fury.io/wez/
  suite: "*"
  components: ["*"]
  packages: [wezterm-nightly]
```

Canonical exact-path form:

```yaml
- name: example-exact-path
  key: https://packages.example.com/signing-key.asc
  urls:
    default: https://packages.example.com/debian/
  path: "./"
  packages: [example]
```

Repository fields:

- `name`: stable safe identifier used to derive Cozydot-owned keyring/source filenames and update state.
- `key`: required HTTPS signing-key URL. Destination paths are derived and not configurable.
- `urls`: required mapping keyed by supported distro IDs and/or `default`.
- A repository uses exactly one canonical source form: `suite` with non-empty `components`, or `path`.
- The suite/components form requires both fields and rejects `path`.
- The path form requires a safe relative path ending in `/` and rejects `suite` and `components`.
- Suite/component token grammar accepts normal APT tokens plus the complete literal `*`. The asterisk is data, not wildcard expansion, and is invalid when embedded in another token.
- `suite: system` is a reserved semantic value resolved from an exact-distro or upstream-distro URL key; it is invalid with a selected `default` URL.
- `packages`: non-empty package list installed after all configured repositories are published and one shared APT metadata refresh succeeds.
- Raw `deb` lines, architecture interpolation, key paths, source filenames, pinning snippets, and arbitrary options remain invalid.
- Cozydot always emits the detected native architecture and its derived `signed-by` path.

### 7.3 Flatpak

- `packages.flatpak` is a non-empty ordered sequence of canonical Flatpak application IDs.
- Cozydot owns one fixed per-user Flathub remote.
- Required runtimes and related refs are inferred.
- Remote selection, arbitrary refs, system-wide installation, and extra Flatpak flags are not configurable in version `1.0.0`.

### 7.4 Cargo and NPM

- `packages.cargo` and `packages.npm` are non-empty ordered package-name sequences.
- Manager flags and command fragments are invalid.
- Non-empty Cargo requires a declared `tools.rust` toolchain and uses Cozydot’s fixed Rust/Cargo installation policy.
- Non-empty NPM requires a declared `tools.node` toolchain and uses Cozydot’s fixed managed Node environment.
- Version `1.0.0` intentionally does not add package-version maps until exact package-version semantics are designed consistently.

### 7.5 Binary packages

`packages.binaries` is the sole typed mechanism for artifacts installed outside package repositories. The unclear `direct` name belongs only to a superseded format and is not accepted.

Canonical GitHub form:

```yaml
- name: obsidian
  format: appimage
  commands: [obsidian]
  source:
    provider: github
    repository: obsidianmd/obsidian-releases
    assets:
      amd64:
        include: "Obsidian-*.AppImage"
        exclude: ["Obsidian-*-arm64.AppImage"]
```

Fields:

- `name`: safe stable identity used for managed state and updates.
- `format`: `deb` or `appimage`.
- `commands`: non-empty unique executable basenames used for presence and postcondition checks.
- `source.provider`: `github` or `url`.

GitHub provider:

- `repository`: validated `owner/repository` coordinate.
- Every GitHub source resolves the repository’s latest non-prerelease release. The YAML has no release field, tag pin, or alternate release-selection mode.
- `assets`: non-empty architecture map using canonical `amd64`, `arm64`, `arm32`, and `riscv64` keys.
- Every selected architecture contains required `include` and optional `exclude` and `sha256` fields.
- Omitted `exclude` means no exclusions; an empty exclusion sequence is invalid.
- Include/exclude patterns are anchored whole-filename patterns with only `*` and `?` operators.
- Selection requires exactly one remaining asset.
- `sha256`, when supplied, must be exactly 64 lowercase hexadecimal characters and must match before installation.
- If GitHub publishes a SHA-256 digest for the selected asset, Cozydot verifies it even when the YAML omits `sha256`.

Fixed-URL provider:

```yaml
source:
  provider: url
  urls:
    amd64: https://downloads.example.com/app-amd64.deb
  sha256:
    amd64: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

- `urls` and `sha256` must contain exactly the same non-empty canonical architecture key set.
- Fixed URLs require configured SHA-256 values.
- URL sources have no release lookup and are not selected by automatic binary updates.
- Changing the source identity or checksum causes Cozydot to reinstall the declared binary even if its command already exists.

Installation behavior:

- Debian artifacts are validated with `dpkg-deb`, installed through one fixed noninteractive APT-local-package operation, and verified through every command.
- AppImages are ELF-validated, atomically stored below Cozydot’s user data directory, mode `0755`, and linked under `~/.local/bin` using every declared command.
- Existing unrelated executables never authorize Cozydot to overwrite files it does not own. Managed source identity is recorded so declaration changes are not hidden by command-presence checks.
- Generic archives and installer scripts are out of scope.

## 8. `tools`

```yaml
tools:
  rust: stable
  go: latest
  node: lts
  python: "3.13"
```

- Each present scalar declares one toolchain through a fixed manager.
- Rust accepts moving channels and grammar-conforming numeric Rust selectors. Cozydot resolves availability against its fixed official Rust release metadata before that toolchain operation mutates state; an unavailable selector fails directly.
- Go accepts `latest` or an exact Go version.
- Node accepts `lts`, `latest`, or an exact Node version.
- Python accepts grammar-conforming numeric selectors. Cozydot resolves availability against its fixed Python-backend release index before that toolchain operation mutates state; an unavailable selector fails directly. Python has no automatic update leaf in version `1.0.0`.
- Canonical host target and architecture aliases are detected internally.
- A numeric selector with omitted trailing components resolves once to a concrete available release during the first managed installation; Cozydot records that concrete release and keeps it pinned on later applies. It is not a moving selector.
- Exact and resolved numeric selectors remain pinned. Only named moving selectors are refreshed when their matching update leaf is enabled.

## 9. `fonts`

```yaml
fonts:
  nerd:
    - GeistMono
```

- `nerd` is a non-empty unique sequence of canonical Nerd Fonts family names.
- Cozydot resolves the fixed official Nerd Fonts release source, validates archives and paths, installs atomically, and refreshes the user font cache.

## 10. `dotfiles`

```yaml
dotfiles:
  packages: [bash, bin, bat, starship]
```

- `packages` selects non-empty safe directory names under the active Cozydot dotfiles root.
- Stow is the fixed backend.
- Cozydot owns one fixed backup-before-adoption policy.
- Conflict policies, Stow flags, arbitrary source paths, and deletion modes are not configurable.

## 11. `integrations`

```yaml
integrations:
  docker:
    add_user_to_group: true
    logging:
      driver: local
      max_size: 10m
  virtualbox:
    add_user_to_group: true
  vscode:
    extensions: [...]
```

General rule: integrations configure existing products; they never install them. A canonical full configuration must declare the corresponding packages before integrations.

Docker:

- `add_user_to_group: true` ensures invoking-user membership in Docker’s group. Omission preserves membership; explicit `false` is invalid.
- `logging.driver` currently accepts only `local`; omission preserves Docker logging configuration.
- `max_size` is optional with `driver: local`, uses a positive Docker size, and preserves the value when omitted.
- Cozydot merges only owned Docker daemon keys, preserves unrelated JSON, publishes atomically under a fixed lock, and does not silently restart active containers.

VirtualBox:

- `add_user_to_group: true` ensures invoking-user membership in `vboxusers`.
- Omission preserves membership; explicit `false` is invalid.

VS Code:

- `extensions` is a non-empty ordered sequence of canonical lowercase extension IDs.
- Cozydot ensures every configured extension is installed and does not remove unrelated extensions.

## 12. `desktop`

```yaml
desktop:
  theme: dark
  terminal: wezterm
  idle:
    timeout: 15m
    dim: false
  gnome:
    extensions: [...]
    dock: true
    rounded_corners: true
```

- `theme`: `light` or `dark`.
- `terminal`: safe executable basename that must exist after package/tool installation.
- `idle.timeout`: strict duration. `0s` disables idle timeout.
- `idle.dim`: boolean desired state.
- `theme`, `terminal`, and `idle` are backend-neutral desired-state intents owned by the resolved desktop backend. GNOME and Cinnamon each have a separate typed lowerer/operation implementation; these fields are never translated by constructing schema names dynamically from unvalidated desktop text.
- The `gnome` mapping owns only GNOME-specific extensions, dock, and rounded-corner behavior. No `cinnamon` mapping exists in version `1.0.0` because there are no Cinnamon-only public fields.
- Each backend preflights its required schema/CLI before mutation and fails rather than claiming an unsupported setting.
- A present `gnome` section requires resolved GNOME; it is not silently emulated on another desktop.
- `gnome.extensions`: non-empty unique extension UUID sequence.
- Newly installed GNOME extensions that require shell re-registration produce an explicit login-required result; Cozydot never reports them as already enabled. After one logout/login, the next apply enables and verifies them.
- `gnome.dock: true` ensures Cozydot’s fixed supported dock provider for the resolved GNOME platform before applying and verifying dock behavior. Omission preserves dock state; explicit `false` is invalid until a reversible disabled state is defined.
- `gnome.rounded_corners: true` ensures Cozydot’s fixed supported rounded-corner provider before applying and verifying its settings. Omission preserves corner state; explicit `false` is invalid until a reversible disabled state is defined.
- Omitted desktop leaves desktop state unchanged.

## 13. `updates`

```yaml
updates:
  apt: full
  flatpak: true
  tools:
    rust: true
    go: true
    node: true
  packages:
    cargo: true
    npm: true
    binaries: true
  fonts: true
```

Rules:

- Omission disables that update target; it never implies a global sweep.
- `apt`: `standard` or `full`. `standard` performs the fixed safe upgrade policy; `full` allows dependency-changing full upgrade. There is no `off`; omission is the sole disabled form.
- `flatpak: true` updates only configured application IDs while permitting required runtimes, related refs, and declared end-of-life replacements.
- Tool updates require configured moving selectors. Exact Rust, Go, Node, and Python versions remain pinned, and an update leaf targeting an exact selector is rejected as ineffective. Python has no update leaf because version `1.0.0` exposes no moving Python selector.
- Cargo and NPM updates target only configured package names.
- Binary updates target every configured GitHub definition by resolving its latest non-prerelease release. Fixed URLs remain fixed and are not selected by automatic binary updates.
- `fonts: true` updates only configured Nerd Font families through the fixed official release source.
- Explicit `false` update leaves are invalid; omit the leaf instead. This preserves one canonical disabled representation.
- Every update leaf requires at least one matching configured target; ineffective update leaves are validation errors rather than silent no-ops.

## 14. Planning and execution order

After complete loading and platform validation, Cozydot plans in this fixed order:

1. Infer and install internal prerequisites.
2. Bootstrap fixed language/package managers required by declared state.
3. Verify privilege and administrative requirements.
4. Reconcile managed official APT sources.
5. Publish every third-party repository key and source.
6. Perform one shared APT metadata refresh when any APT consumer requires it.
7. Apply unattended-upgrade, Ubuntu Snap, and codec state.
8. Purge configured APT conflicts.
9. Install repository package groups.
10. Install ordinary APT packages.
11. Ensure Flatpak applications.
12. Ensure language toolchains.
13. Ensure Cargo and NPM packages.
14. Ensure binary packages.
15. Ensure fonts.
16. Apply dotfiles.
17. Apply existing-product integrations.
18. Apply desktop state.
19. Apply enabled update targets in declared fixed order.
20. Verify operation postconditions and emit a concise result summary.

No planner or lowerer may reorder operations across these dependency boundaries based on map iteration order.

## 15. Platform and architecture guarantees

- Supported canonical architectures are `amd64`, `arm64`, `arm32`, and `riscv64`.
- Runtime aliases are normalized internally and never appear in user selectors.
- Every configured binary must contain the native architecture key before mutation.
- Repositories and APT package availability remain upstream responsibilities, but URL/suite resolution is validated before publication.
- Ubuntu amd64 GNOME and Debian 13 amd64 GNOME are mandatory acceptance-test reference targets for the comprehensive configuration, not exclusive `system.require` constraints. The configuration may also validate elsewhere when every repository and binary supports the detected platform.
- Platform-requirement mismatch, missing architecture selectors, unavailable repository URL mappings, and unsupported managed-source components fail before mutation.

## 16. State, idempotence, and failure behavior

- Cozydot records only the minimum managed identity needed to distinguish owned artifacts and changed declarations.
- A second apply with unchanged configuration performs no unnecessary mutation.
- Every download, conversion, archive extraction, and privileged publication is staged and verified before atomic replacement.
- Existing unmanaged files are never silently adopted or overwritten.
- Multi-operation workflows are failure-preserving but not advertised as transactional rollback.
- Partial success is reported precisely; Cozydot never prints global success after a failed or skipped required operation.

## 17. Canonical artifacts

Configuration version `1.0.0` has three different documentation artifacts:

1. **Generated beginner config** — concise, conservative, and safe for a broadly supported host.
2. **Comprehensive real config** — a realistic Ubuntu/Debian 13 GNOME workstation containing the complete intended software set.
3. **Exhaustive parser fixture** — synthetic coverage of mutually exclusive and unusual forms such as fixed URLs and exact-path repositories.

The real full config is never distorted merely to exercise every parser field. The exhaustive fixture is never presented as a recommended workstation.

## 18. Coverage disposition

Version `1.0.0` coverage is deliberate rather than a mechanical copy of superseded fields.

Retained and redesigned:

- automatic platform detection with optional distro/desktop allowlists;
- official APT preservation or management;
- unattended upgrades, Ubuntu Snap, and codecs;
- APT purge/install, third-party repositories, Flatpak, Cargo, NPM, and release binaries;
- Rust, Go, Node, Python, Nerd Fonts, and Stow dotfiles;
- Docker, VirtualBox, and VS Code integration;
- theme, terminal, idle, GNOME extension, dock, and rounded-corner state;
- granular updates without unrelated package sweeps.

Improved in version `1.0.0`:

- APT fields are grouped by manager;
- WezTerm's official `* *` repository is represented natively;
- true exact-path repositories use the mutually exclusive typed `path` form;
- upstream distro/codename resolution is explicit;
- `binaries` replaces the unclear `direct` term;
- binary sources support latest GitHub releases and checksummed fixed URLs;
- changed binary declarations cannot be hidden by an unrelated command in `PATH`;
- destructive service/package states use explicit words rather than overloaded booleans;
- Nerd Font updates become an explicit configured target;
- `null`, redundant no-op structures, and explicit false update leaves are rejected.

Removed intentionally:

- `appimaged`: Cozydot owns AppImage placement, links, state, and updates directly;
- `pyenv`: Python uses one fixed Cozydot-managed backend;
- `yq`: configuration and release metadata are parsed internally;
- package-manager flags embedded in package strings;
- user-listed bootstrap dependencies that Cozydot can infer;
- raw binary URLs without checksums;
- legacy tags, shell substitutions, raw repository lines, and post-apply YAML mutation.

The comprehensive full YAML restores the legacy intended applications: Docker CE, eza/fzf/yazi/zoxide, GitHub CLI, Helium, OnlyOffice, VirtualBox, VS Code, WezTerm Nightly, the Flatpak set, Cargo/NPM tools, draw.io, Fastfetch, Git Credential Manager, Obsidian, and Zen Browser. Dependencies omitted from that YAML are omitted because version `1.0.0` assigns them to Cozydot's inferred prerequisites, not because the applications were dropped.

## 19. Approval record

The user approved implementation of:

- this field hierarchy and naming;
- the beginner config;
- the comprehensive config;
- destructive and omission semantics;
- repository layouts;
- binary providers and latest-release update semantics;
- update scopes;
- platform claims and non-goals.
