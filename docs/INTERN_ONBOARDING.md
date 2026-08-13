# Cozydot intern onboarding

Cozydot is a Rust CLI that provisions a workstation from one typed YAML file. It supports Debian, Ubuntu, Pop!_OS, Linux Mint, and Apple Silicon macOS.

This guide explains the code you need to change Cozydot safely. Read `README.md` first for the user-facing contract.

## Start here

Run the repository checks before editing:

```bash
scripts/generate-configs.sh --check
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

The main command path is:

```text
CLI command
→ load and validate cozydot.yaml
→ detect the host when required
→ build a complete ordered Vec<Operation>
→ execute each operation in order
```

Configuration is declarative. YAML selects typed behavior; it cannot inject shell commands. Every host mutation has a fixed Rust executor.

## Commands

`src/main.rs` owns the public CLI.

### `init`

Creates or safely refreshes the active configuration and bundled dotfiles under:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/cozydot
```

It embeds four presets:

```text
cozydot
full
cli
vm
```

`init` updates files that are missing or still match the last managed hash. It preserves user-edited files. `.managed-files` stores that ownership state.

### `check`

Loads and validates the active configuration. It does not detect the platform and does not mutate the host. Use it for schema and cross-field validation.

### `apply`

Loads the configuration, detects the host, validates platform constraints, builds the complete apply plan, and executes it sequentially.

Apply means ensure configured state. It should avoid reinstalling software that is already present.

### `dotfiles`

Plans only shared and current-platform dotfiles. GNU Stow applies packages in configured order.

The default is conservative:

```text
unmanaged destination conflict
→ report every conflict
→ change nothing
```

`--replace` moves conflicts into the XDG state directory before applying links.

### `update`

Runs only enabled update policies. It does not replay apply behavior. Empty or all-false update configuration is a valid no-op.

## Repository map

```text
src/main.rs                 CLI and command lifecycles
src/config.rs               typed YAML schema and validation
src/platform.rs             host detection and normalization
src/planner/                configuration → ordered operations
src/operations/mod.rs       Operation enum, labels, dispatch
src/operations/             host-side executors
src/init.rs                 embedded preset/dotfile synchronization
build.rs                    embeds presets and dotfiles
configs/cozydot.yaml        manually maintained base preset
configs/{full,cli,vm}.yaml  generated presets
dotfiles/                   GNU Stow packages
tests/cli.rs                compact external behavior suite
install.sh                  release installer
scripts/                    generation and packaging
```

## Configuration

`Config::load` in `src/config.rs` performs typed deserialization and semantic validation.

The root scopes are:

```text
shared
os.linux
os.macos
```

`shared` describes portable intent such as toolchains, Cargo/npm packages, fonts, VS Code extensions, and dotfiles. Platform scopes describe native package managers, system settings, integrations, desktop behavior, and updates.

The schema uses `deny_unknown_fields`. Removing or renaming a field is therefore an intentional breaking change. Cozydot does not carry compatibility adapters for obsolete schemas.

Empty optional sections and false enable-only flags are no-ops. Values interpreted by native tools should use the native tool's terminology unless Cozydot needs a typed selector for platform planning.

### Presets

Edit only:

```text
configs/cozydot.yaml
```

Then regenerate:

```bash
scripts/generate-configs.sh
```

The script derives `full`, `cli`, and `vm`. CI checks that generated files are current.

## Platform detection

`src/platform.rs` normalizes:

- distro and upstream family;
- distro and base codenames;
- desktop environment;
- architecture.

Linux architecture names inside Cozydot are:

```text
amd64
arm64
arm32
```

APT repository `arch` uses Debian names:

```text
amd64
arm64
armhf
```

Supported Debian releases are Bookworm and Trixie. Unsupported distributions, releases, architectures, and configured desktop requirements fail before a plan executes.

`check` intentionally skips platform detection. `apply`, `dotfiles`, and `update` require it.

## Planning

`src/planner/mod.rs` owns ordering. Leaf modules contribute typed `Operation` values to private execution stages. The stages are implementation-only dependency buckets, not user-visible actions.

Important ordering rules include:

```text
administrator checks
→ platform foundation
→ direct APT metadata and packages
→ derived prerequisites and manager bootstraps
→ third-party repositories
→ repository metadata refresh
→ repository conflicts and packages
→ language managers and toolchains
→ ecosystem packages
→ binaries
→ fonts
→ dotfiles
→ integrations
→ desktop settings
```

Do not rely on function call order alone. `ExecutionStage` controls the final order.

The planner also derives prerequisites. A repository requires curl, CA certificates, and GPG; npm requires FNM; Cargo packages require Rust and cargo-binstall. Add such dependencies in the planner rather than asking users to repeat them in YAML.

The entire plan is created before execution. A planning or platform-validation error must therefore happen before host mutation.

## Operations

`src/operations/mod.rs` is the execution boundary.

Each `Operation` variant contains already planned data. Dispatch is explicit. Executors should:

1. inspect live state;
2. return success when the requested state already exists;
3. mutate through fixed argument vectors;
4. stop on an unexpected command status;
5. provide context that identifies the failed action.

There is no general shell-script operation and no YAML-to-shell interpolation.

`Host` in `src/operations/host.rs` centralizes command execution and environment/path access. Keep command arguments separate. Do not build shell strings for ordinary execution.

## APT lifecycle

Direct APT packages run before third-party repository packages:

```text
refresh distro metadata
→ install missing direct packages
→ publish applicable third-party repositories
→ refresh metadata once for those repositories
→ purge installed declared conflicts once
→ install all missing repository packages once
```

The two refresh points serve different source sets. Do not collapse them unless direct packages are deliberately moved after repository publication.

### Third-party repositories

Every declaration is validated before platform filtering or mutation:

- safe `sources.list.d` name;
- non-empty distro URL map;
- key path directly under `/etc/apt/keyrings` or `/usr/share/keyrings`;
- safe `.asc` or `.gpg` key filename;
- required non-empty suite and components;
- supported APT architecture names;
- no control characters in source values.

Applicability is:

```text
matching exact distro, upstream family, or default URL
AND
current architecture included by optional arch
```

An inapplicable repository contributes no source, conflict, package, prerequisite, or metadata refresh.

Applicable repositories follow this path:

```text
download key to an unprivileged temporary file
→ reject empty content
→ dearmor and require at least one public key
→ keep armored bytes for .asc or binary bytes for .gpg
→ atomically publish the key and source file
```

All applicable repository conflicts and packages are aggregated. Purging happens only after repository publication and a successful metadata refresh.

### Debian official sources

Pure Debian converges official source components separately. It supports conventional Bookworm and Trixie `.list` / deb822 `.sources` files while preserving unrelated entries and options. This is not the third-party repository path.

## Privileged publication

`src/operations/privileged_file.rs` publishes root-owned files through staged writes:

```text
write and fsync an unprivileged temporary file
→ sudo install into a temporary file beside the destination
→ sync the staged file
→ reject directory destinations
→ atomic mv over the destination
→ sync the parent directory
```

Do not replace this with streamed writes into `/etc` or `/usr`. Download and parsing failures must leave privileged destinations untouched.

## Init ownership

`src/init.rs` embeds presets and dotfiles at build time and tracks managed hashes.

The update policy is:

```text
missing file              → install
known and unchanged file  → refresh
modified or unmanaged     → preserve
```

Paths must remain relative, normal, UTF-8-compatible, and free of tabs/newlines. The configuration root and managed directory chain must not traverse symlinks.

The manifest is published only after all selected files synchronize successfully.

## Dotfiles

A dotfile package is a real directory below the embedded `dotfiles/` root. The source tree is authoritative. Cozydot never copies a destination file back into the repository.

Before replacement, Cozydot validates every configured package and checks GNU Stow. This prevents a later bad package or missing executable from moving earlier conflicts and leaving the operation half-complete.

GnuPG dotfiles receive special handling because `~/.gnupg` must remain a real directory with mode `0700`.

## Apply and update semantics

Keep ensure and update behavior separate.

Examples:

- `apply` installs a missing configured Cargo package;
- `update` runs cargo-update only when enabled;
- `apply` ensures the selected toolchain;
- `update` asks the native manager to update according to policy;
- managed Deb/AppImage binaries are ensure-only.

A native manager owns native value parsing where possible. Cozydot should not duplicate upstream grammars without a planning or safety reason.

## Integration tests

`tests/cli.rs` is intentionally small and external. It tests command contracts and high-risk boundaries through the compiled CLI:

- help, version, missing configuration, and installer rejection;
- init ownership and preset materialization;
- validation before platform detection or mutation;
- silent no-op apply/update;
- dotfile conflict and replacement behavior;
- repository schema, applicability, key validation, publication, ordering, and idempotence;
- APT update policy.

Do not add a test for every enum field, command spelling, or upstream manager output. Add integration coverage when a failure would violate a public command contract, mutation ordering, idempotence, or a security boundary.

Prefer one end-to-end case that proves a lifecycle over several narrow tests that repeat the same setup. Fake executables should record fixed argv and model only the state needed by the contract.

## Making a change

1. Read the relevant config type, planner leaf, operation variant, and executor.
2. Write the intended lifecycle and identify prerequisites and ordering.
3. Update the base preset if the schema or defaults change.
4. Regenerate derived presets.
5. Update the external behavior test only when the public contract changes.
6. Run all checks.
7. Search for obsolete names and documentation.
8. Review the complete diff for unrelated changes.

## Required checks

```bash
scripts/generate-configs.sh --check
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
scripts/package-release.sh
bash -n install.sh scripts/generate-configs.sh scripts/package-release.sh dotfiles/bash/.bashrc
git diff --check
```

The release script builds with `--locked` and writes the archive and checksum under `target/`.

## Review checklist

Before committing, answer these directly:

- Does the complete configuration fail before mutation?
- Is platform applicability resolved before adding operations?
- Is dependency order encoded by the correct execution stage?
- Does the executor inspect state before changing it?
- Are privileged destinations constrained and atomically published?
- Is repeated apply a no-op where the native state is already correct?
- Did generated presets and public docs change with the schema?
- Does the compact integration suite still cover the changed public boundary?
