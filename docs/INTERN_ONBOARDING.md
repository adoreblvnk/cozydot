# Cozydot intern onboarding

Cozydot is a Rust CLI that provisions Debian-family Linux and Apple Silicon macOS from one typed YAML configuration. Read `README.md` for the user contract; use this guide to preserve lifecycle order and safety boundaries while changing the implementation.

## Development setup

Install the latest stable Rust toolchain with Rustfmt and Clippy, plus `yq` v4. Run:

```bash
scripts/generate-configs.sh --check
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
```

## Runtime architecture

The binary exposes `init`, `check`, `apply`, `dotfiles`, and `update`.

Host-changing commands follow this boundary:

```text
resolve the active Cozydot root
-> deserialize and validate the complete configuration
-> detect and normalize the current platform
-> validate the complete configuration against that platform
-> derive cross-cutting prerequisites and perform static preflight
-> enter the explicit Linux or macOS workflow
-> construct and execute each typed operation in dependency order
-> stop immediately on failure
```

There is no stage map or complete operation vector. The root workflows in `src/workflow/mod.rs` make lifecycle order visible top-to-bottom. They may perform pure derivation before their first executor call when later ordering depends on aggregate facts, but each operation is otherwise built immediately before execution.

The implementation layers are:

```text
src/main.rs            command entry points and active-host loading
src/init.rs            config root, presets, bundled files, managed hashes
src/config.rs          typed YAML schema and config/platform checking
src/platform.rs        host detection and normalized platform facts
src/workflow/mod.rs    explicit Linux/macOS command workflows
src/operations/mod.rs  operation types, labels, and typed dispatch
src/operations/        live-state checks and host mutations
tests/cli.rs           external lifecycle and safety contracts
```

## Validation boundary

`Config::load` performs typed deserialization and platform-independent semantic validation. The schema uses `deny_unknown_fields`; unknown keys fail rather than being ignored. Empty optional sections, empty lists, absent values, and false enable-only flags are no-ops unless a field has a stricter contract.

`ActiveHost::load` then detects the platform and calls `Config::validate_for_platform`. No host-changing workflow starts before that complete config/platform check succeeds. This rejects unsupported distribution, architecture, desktop, and platform-specific intent before mutation.

Repository declarations receive static validation before applicability filtering. Linux apply also constructs every applicable repository payload during its pure preflight, before its first executor call. This catches malformed source facts before an earlier operation can mutate the host without recreating a hidden complete operation plan. Network key retrieval, public-key validation, and publication remain live executor checks.

`cozydot check` deliberately preserves its public semantics: it validates the active YAML without platform detection or mutation.

## Platform facts

`src/platform.rs` detects the operating system, Linux distribution and upstream family, distro and base codenames, desktop environment, and architecture. Cozydot Linux architecture names are `amd64`, `arm64`, and `arm32`; APT uses `amd64`, `arm64`, and `armhf`. macOS supports Apple Silicon only.

Linux repository applicability is resolved from `PlatformIdentity`, distro/upstream URL selection, and optional APT architecture filters. An inapplicable repository contributes no prerequisite, source, conflict, package, or metadata refresh.

## Apply workflow

`apply` is ensure-only. It accepts update controls as part of the complete configuration but never executes them. Existing executors inspect live state and avoid reinstalling state that is already correct.

Before Linux apply mutates the host, `linux_apply_facts` derives the normalized identity, applicable repository facts, aggregate conflicts and packages, direct and repository refresh requirements, manager bootstraps, architecture-specific binary applicability, and the deduplicated APT prerequisite set.

Linux apply executes the following order, skipping absent configuration:

1. On Debian:
   1. If administrator verification is enabled, verify administrator access.
   2. Converge official Debian APT components.
2. On Ubuntu:
   1. If unattended upgrades are configured, apply their state.
   2. If Snap is configured, apply its state.
   3. If codecs are enabled, install Ubuntu restricted codecs.
3. If distro APT metadata is required:
   1. Refresh APT metadata.
4. If derived APT prerequisites are required:
   1. Install the prerequisites.
5. If direct APT packages are configured:
   1. Install the packages.
6. If applicable third-party repositories are configured:
   1. Publish each repository.
   2. Refresh repository metadata once.
   3. Purge aggregate conflicts and install aggregate repository packages.
7. If Flatpak applications are configured:
   1. Ensure Flathub availability.
   2. Install the configured applications.
8. Apply required tools:
   1. Bootstrap rustup.
   2. Ensure the Rust toolchain.
   3. Bootstrap FNM.
   4. Ensure the Node.js toolchain.
   5. Bootstrap uv.
   6. Ensure the Python toolchain.
   7. Ensure the Go toolchain.
   8. Bootstrap cargo-binstall.
   9. Bootstrap cargo-update.
9. Apply configured language packages:
   1. Converge Cargo packages.
   2. Converge npm packages.
10. Converge each applicable Deb binary.
11. If applicable AppImages are configured:
    1. Converge appimaged.
    2. Converge each AppImage.
12. If Nerd Fonts are configured:
    1. Converge the configured font families.
13. If dotfiles are configured:
    1. Apply shared and Linux dotfiles.
14. Apply configured integrations:
    1. Apply Docker group membership and logging.
    2. Apply VirtualBox group membership.
    3. Converge VS Code extensions.
15. If Linux desktop settings are configured:
    1. Apply the settings.

`AptBootstrapPackages` performs its own metadata refresh immediately before installing missing prerequisites. A transcript can therefore contain that refresh in addition to the explicit distro or repository refreshes.

macOS apply derives Homebrew need from configured formulae, casks, dotfiles, FNM, and cargo-binstall. It adds `stow` to formulae for apply dotfile intent.

macOS apply executes the following order, skipping absent configuration:

1. If administrator verification is enabled:
   1. Verify administrator access.
2. If Xcode Command Line Tools are enabled:
   1. Install Xcode Command Line Tools.
3. If Rosetta is enabled:
   1. Install Rosetta.
4. If Homebrew is required:
   1. Bootstrap Homebrew.
5. If Homebrew packages are required:
   1. Install configured formulae and casks.
6. Apply required tools:
   1. Bootstrap rustup.
   2. Ensure the Rust toolchain.
   3. Bootstrap FNM.
   4. Ensure the Node.js toolchain.
   5. Bootstrap uv.
   6. Ensure the Python toolchain.
   7. Ensure the Go toolchain.
   8. Bootstrap cargo-binstall.
   9. Bootstrap cargo-update.
7. Apply configured language packages:
   1. Converge Cargo packages.
   2. Converge npm packages.
8. If Nerd Fonts are configured:
   1. Install the configured user font families.
9. If dotfiles are configured:
   1. Apply shared and macOS dotfiles.
10. If VS Code extensions are configured:
    1. Converge the extensions.
11. If macOS defaults are configured:
    1. Apply the defaults.

Homebrew must precede FNM and cargo-binstall because those bootstraps use Homebrew on macOS.

## Dotfiles workflow

Standalone `dotfiles` combines shared packages with packages for the detected platform, preserves declaration order, and executes one typed dotfiles operation when the list is non-empty. It does not bootstrap Stow; apply owns platform-specific Stow prerequisites.

The dotfiles executor performs complete preflight before changing destinations:

1. Verify the dotfiles root and every selected package directory.
2. Verify Stow availability.
3. Resolve every intended link and destination.
4. Collect all unmanaged destination conflicts.
5. Report all conflicts and change nothing when `--replace` is absent.
6. Prepare the backup destination when replacement is enabled.
7. Move conflicts under `${XDG_STATE_HOME:-$HOME/.local/state}/cozydot/dotfile-backups`.
8. Invoke Stow and stop on the first failed mutation.

Cozydot never adopts destination files into its dotfile source.

## Update workflow

`update` executes only explicitly enabled controls and does not replay apply intent. An absent, empty, or all-false update section is a validated silent no-op. Managed Deb and AppImage declarations are ensure-only and have no update category.

Linux update executes:

1. Refresh APT metadata when an APT policy is enabled.
2. Run the selected standard or full APT upgrade.
3. Install deduplicated prerequisites for enabled updates.
4. Update installed Flatpak applications.
5. Bootstrap rustup.
6. Update Rust toolchains.
7. Update the Go toolchain.
8. Bootstrap FNM.
9. Update the Node.js toolchain.
10. Bootstrap uv.
11. Update the Python toolchain.
12. Update installed Cargo packages.
13. Update global npm packages.
14. Redownload configured Nerd Fonts.

Linux npm package update remains independent from Node toolchain update: npm-only update does not add FNM or APT prerequisites. Flatpak update installs the native `flatpak` prerequisite when needed but does not configure Flathub.

macOS update executes:

1. Bootstrap Homebrew when required by FNM.
2. Update selected Homebrew formulae and casks.
3. Bootstrap rustup.
4. Update Rust toolchains.
5. Update the Go toolchain.
6. Bootstrap FNM.
7. Update the Node.js toolchain.
8. Bootstrap uv.
9. Update the Python toolchain.
10. Update installed Cargo packages.
11. Update global npm packages.
12. Redownload configured user Nerd Fonts.

npm update on macOS derives FNM and therefore Homebrew. Selectorless Go and Node updates use `latest`; Python updates use major line `3`; Rust updates all installed rustup toolchains.

## Operation execution

The small workflow `execute` helper obtains the operation label, prints `Applying <label>` or `Updating <label>`, dispatches through `operations::execute`, adds action context to failures, reports `LoginRequired`, and returns immediately on failure. A workflow with no enabled operations prints nothing.

`Operation` remains Cozydot's safe typed execution boundary. There is no YAML-to-shell path. Executors invoke fixed programs with separate argument vectors, inspect live state, reject unexpected status, and return success when requested state is already correct.

## Repository safety

Direct distro packages intentionally precede third-party repository publication. Applicable repositories are published sequentially, followed by one metadata refresh and one aggregate conflict/package operation.

Repository config validation covers safe names, bounded keyring destinations, `.asc` or `.gpg` suffixes, non-empty URL maps, suite and component requirements, supported APT architectures, collisions, and control characters.

The executor downloads each key to an unprivileged temporary file, rejects empty content, validates that GPG finds a public key, prepares armored or binary bytes, and only then publishes through `src/operations/privileged_file.rs`. Privileged publication stages and fsyncs content beside the bounded destination, rejects directory destinations, atomically renames, and syncs the parent. Do not replace this with streamed writes into `/etc` or `/usr`.

## Other lifecycles

Architecture-specific binary declarations are derived immediately before their execution section. Missing mappings contribute no operation. Appimaged is converged before individual AppImages.

Linux desktop intent is checked against the detected desktop before mutation. The workflow derives `dconf-cli` and `libglib2.0-bin`, plus `gnome-shell` for GNOME extension capabilities. Desktop settings execute last. macOS defaults are collected into one typed operation and also execute last.

`init` owns its separate managed-file lifecycle: validate managed paths and symlink boundaries, preserve user-edited or unmanaged files, atomically synchronize selected files, and publish the managed hash manifest only after successful synchronization.

## Tests and changes

`tests/cli.rs` is the authoritative compact integration suite. It covers validation before platform detection or mutation, init ownership, silent no-ops, dotfile preflight and replacement, repository applicability and key safety, repository/package ordering, APT policy, and installer safety. Do not add unit tests under `src/`.

When changing orchestration:

1. Read the relevant config type, root workflow, typed operation, and executor.
2. State the required dependency order and no-op behavior.
3. Keep complete config/platform checking before mutation.
4. Keep Linux and macOS policy differences explicit.
5. Keep lifecycle order visible in the root workflow.
6. Derive prerequisites once and preserve manager-before-consumer order.
7. Preserve repository filtering, aggregation, and post-publication refresh.
8. Preserve fixed arguments, bounded paths, public-key validation, and atomic publication.
9. Add integration coverage only for a meaningful public or safety boundary.
10. Search for obsolete architecture terms, inspect the complete diff, and run all required checks.

Required checks:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --release
scripts/generate-configs.sh --check
git diff --check
```
