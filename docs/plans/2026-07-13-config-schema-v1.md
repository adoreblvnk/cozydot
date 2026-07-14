# Config schema v1 implementation plan

## Goal

Replace the tagged legacy configuration with the single canonical contract in `docs/config-schema-v1.md` while preserving the `curl -> init -> edit cozydot.yaml -> apply` product flow. This is a breaking migration: remove legacy parsing and planning paths when their v1 replacements land; do not add adapters, aliases, or dual-schema support.

## Milestones

### 1. Freeze contract and architecture foundation

Affected paths:

- `docs/config-schema-v1.md`
- `docs/plans/2026-07-13-config-schema-v1.md`
- `src/platform.rs`
- `src/planner.rs`

Work:

- Freeze all v1 field names, types, disable semantics, managers, repository shape, direct-package shape, and update controls.
- Introduce one typed canonical architecture value.
- Detect the production machine label deliberately with successful, non-empty UTF-8 `uname -m` output, then trim and normalize it. Keep `Platform::from_parts` as the deterministic injected path for tests.
- Define `Arm32` as Armv7/armhf, normalize only its supported machine aliases, and translate Debian, Go, Rust, and release-asset names internally. Host aliases are source-specific: reject ambiguous Rust `arm`, release-only `x64`/`riscv64gc`, and `armv6l`; keep Go's official `armv6l` archive name and release aliases as output translations.
- Migrate current architecture callers without changing embedded YAML.

TDD gate:

- Table tests cover every required host-normalization alias and separately cover canonical, Debian, Go, Rust, and release-asset output translations. Release-alias tests do not require host-normalization round trips. Tests prove `arm`, `armv6l`, `x64`, and `riscv64gc` are rejected as host inputs rather than assigned a host architecture.
- Focused helper tests cover trimmed `uname -m` output plus command failure, empty output, and non-UTF-8 output without invoking the real host.
- Unknown architecture tests assert a clear deterministic error.
- Existing preset, planner, behavior, CLI, init, and distribution tests remain green.

Exit criteria:

- Contract and plan agree on every field.
- No architecture alias is introduced as a v1 YAML field or interpolation variable.
- No production preset uses schema v1 yet.

### 2. Add typed v1 parser and validation

Affected paths:

- `src/config.rs` or a replacement `src/config/` module
- `src/lib.rs`
- `tests/fixtures/`
- new focused configuration tests under `tests/`

Work:

- Define Serde-backed structs and enums matching only the authoritative contract.
- Require integer `schema: 1`; deny unknown fields recursively.
- Preserve the contract's distinction between omitted/`null` host-state controls and an explicit `false`; do not create alternate public forms while normalizing enable-only controls and empty collections.
- Validate literal repository names, safe direct-package definition names, manager-specific package identifiers, parsed HTTPS URLs, versions, duration strings, duplicate entries, managed APT components, repository URL selection, `system` suite resolution, direct-asset selector mappings, and cross-field requirements.
- Restrict Rust declarations to `stable`, `beta`, `nightly`, dated nightly channels, or numeric two- or three-component versions. Keep host-target selection internal and reject target-qualified Rustup toolchain names.
- Expose one unified platform-validation method that checks all distribution/upstream and architecture-dependent constraints before planning or mutation.
- Derive repository filename stems by the contract's fixed sanitization algorithm; reject empty stems and collisions before planning.
- Return field-path errors before platform mutation or command execution.
- Reviewed parser findings may tighten malformed-input grammars without adding alternate public forms.
- Retain the legacy production parser unchanged for its current `main`, planner, and embedded-preset callers. The v1 parser is exercised directly by fixtures and tests in this milestone; this is temporary development sequencing, not runtime dual-schema detection, fallback, conversion, or a public compatibility adapter.
- Delete tag handling, string-path lookup, purge-tag mutation, and legacy validation only in the atomic integration milestone after all production callers and embedded presets move to typed values.

TDD gate:

- Start with one minimal valid fixture and one complete valid fixture.
- Add one negative test for each unknown field, wrong type, enum, URL, duplicate, missing required child, schema version, invalid component, invalid duration string, invalid wildcard pattern, invalid field dependency, and native-asset failure class. Prove numeric YAML values are invalid for `desktop.idle.timeout`.
- Validate the canonical full reference on amd64 and arm64 fixtures. Assert clear missing-native-selector failures on arm32 and riscv64; do not add unsupported Obsidian selectors. Planner coverage begins after its typed-input migration.
- Assert omission and `null` preservation, meaningful explicit `false` host-state controls, enable-only `false`, and empty-collection behavior for their respective fields.
- Reject every removed system, dotfile, integration, and desktop form, plus boolean or nested `updates.apt` values, scalar `provides`, scalar asset selectors, selectors missing `include` or `exclude`, and architecture interpolation or substitution.
- `cargo test --all-targets --all-features` passes; v1 parser tests use only v1 fixtures, while unchanged production tests and presets remain legacy until the atomic integration switch.

Exit criteria:

- A complete typed v1 parser and validator exists alongside the temporarily retained legacy production parser.
- V1 fixtures are rejected before any planner or operation call, with field-path errors and no generic `serde_yaml::Value` in the v1 model or validation path.
- `main`, `apply`, the planner, and embedded presets remain legacy-only; there is no runtime v1 auto-detection, fallback, or conversion.

### 3. Build the typed intent planner

Affected paths:

- `src/planner.rs`
- `src/planner/v1.rs`
- focused typed planner tests under `tests/`

Work:

- Build a dedicated equality-testable `PlanV1`/`PlannedAction` model from `ConfigV1` and resolved `Platform`; do not lower it to executable `Operation` or `Step` values in this milestone.
- Plan in dependency-safe phases: fixed internal bootstrap prerequisites; enable-only non-APT preparation; managed distribution-source and third-party key/source publication; one explicit post-source APT metadata refresh when an APT consumer exists; APT-backed controls, removals, repository package groups, and native packages; then Flatpak, tools, Cargo/NPM/direct packages, fonts, dotfiles, integrations, desktop, and enabled granular updates.
- Keep bootstrap separate from the post-publication refresh. Milestone 4 may perform only the minimum old-source metadata setup needed to install fixed internal prerequisites during bootstrap; it must not treat that setup as, replace, or duplicate the one post-source refresh consumed by declared APT work.
- Generate managed distribution sources from detected platform data and configured components. Generate third-party APT source entries from typed fields, the selected distro `HttpsUrl`, typed `system` codename resolution or fixed literal suite, derived key path, and canonical `Architecture`; translate architecture only during milestone-4 lowering. Publish repository sources separately from their typed package-consumer groups.
- Derive every repository `signed-by` path from its validated sanitized name. Retain the canonical key URL and sole derived root-owned mode-`0644` keyring destination needed for milestone 4's fixed HTTPS-download, batch-GPG/dearmor validation, and privileged atomic publication; never expose key handling fields.
- Select and retain one direct-package asset selector by canonical architecture without resolving live release metadata. Retain its required anchored `include`, ordered `exclude` sequence, format, source coordinate, `provides`, and definition name for milestone 4 lowering.
- Plan fixed backup-before-adoption dotfile behavior and the canonical Docker, VirtualBox, VS Code, theme, terminal, idle, and GNOME controls independently.
- Omit desktop actions that do not apply to the resolved desktop; do not emit another desktop's controls or emulate them.
- Translate `updates.apt` over the system APT package set according to its scalar policy; it consumes the earlier explicit refresh and lowering must not refresh again. Target only configured Flatpak application IDs while allowing their required runtimes, related refs, and upstream-declared end-of-life replacements; update only configured Cargo/NPM names and configured direct definitions; never sweep unrelated installed packages. Require declared Node for every non-empty NPM declaration and each corresponding `tools.*` declaration for tool updates, producing no update step when required non-empty lists or tool declarations are absent.
- Mark moving Rust channels, Go `latest`, and Node `latest`/`lts` selectors as moving typed values. Retain exact Rust/Go/Node selectors as pinned typed values; target resolution, verification, and installation belong to milestone 4. Treat unversioned Cargo and NPM schema entries as manager-current intents when enabled.
- Keep interpolation expansion and every raw repository/binary representation out of the v1 planner. The temporarily retained legacy planner remains unchanged until the atomic integration milestone.
- Emit domain values only, not process arguments, command lines, operations, or arbitrary shell source.

TDD gate:

- Snapshot or exact-step tests cover minimal, complete, disabled, and mixed configurations.
- Ordering tests prove prerequisites precede consumers and each shared refresh/bootstrap occurs once.
- System intent tests cover preserve/managed APT sources, component compatibility, explicit admin and unattended-upgrade states, every supported lowercase distro ID, and Ubuntu Snap/codecs controls on the Ubuntu upstream family including supported derivatives and their non-Ubuntu-family skip behavior, without an opaque preparation path.
- Repository planner tests cover distro URL precedence, default fallback, codename-appropriate `system` resolution, the GitHub CLI fixed `stable` suite, derived signed-by output, sanitized-name rejection/collision, and native architecture.
- Direct-package planner tests cover canonical architecture selection, absent-native-selector failure, retained required `include` and ordered `exclude` children, paths, substitutions, and invalid wildcard syntax. Include-then-exclude matching and zero/one/multiple-result behavior belong to milestone 4 operation tests.
- Dotfile planner tests prove every package carries the sole backup-before-Stow policy. Backup execution and failure behavior belong to milestone 4 operation tests.
- Integration and desktop intent tests cover each child independently, including existing-product preflight semantics, exact typed GNOME/Cinnamon targets, `desktop.idle.timeout: 0s`, and the documented desktop-mismatch omission rule; parser tests retain wrong-type and shorthand rejection coverage.
- Exact planner tests prove APT targets the system package set for `off`/`standard`/`full`; Flatpak targets only configured application IDs while allowing required runtimes, related refs, and upstream-declared end-of-life replacements; Cargo and NPM target only their configured names; direct updates target only configured definitions and latest-release selectors; unrelated installed entries never appear. They prove every empty or missing update target produces no update step, including Flatpak without refs, NPM without packages, and tool leaves without matching declarations. A non-empty NPM declaration without Node is rejected earlier at configuration validation.
- Typed planner tests distinguish moving Rust channels, Go `latest`, and Node `latest`/`lts` selectors from exact pinned versions. Target resolution, verification/reinstallation, and manager-current Cargo/NPM behavior belong to milestone 4 behavior tests. Update tests reject boolean APT policies and cover every granular sibling without cross-enablement; omitted and `null` APT behave as `off`.
- Existing legacy operation behavior tests remain unchanged until typed lowering replaces them in milestone 4.

Exit criteria:

- V1 planner has no dotted string lookups, YAML tags, raw repository lines, public interpolation, command representation, or user-selectable conflict/manager algorithms. It retains direct-package wildcards as typed selectors but performs no release lookup or matching.
- Every installation is sourced from `packages` or inferred as an internal prerequisite.

### 4. Lower typed intents into complete fixed-manager operations

Implementation progress:

- Pass 004b adds the typed executable boundary for direct-package ensure-present and update modes without switching production lowering. It validates schema-v1 direct intents again at execution, resolves one latest GitHub release asset with exact anchored `*`/`?` include-then-exclude semantics, validates canonical HTTPS asset URLs and optional GitHub SHA-256 digests, and installs Debian packages or atomically managed AppImages through fixed handlers.
- Direct Debian execution now skips only when every declared provide is executable, validates downloads with `dpkg-deb` before one fixed noninteractive APT install, and verifies every provide afterward. Direct AppImages now use `~/.local/share/cozydot/direct/<name>.AppImage` with mode `0755` and conflict-safe managed links in `~/.local/bin`; failed downloads, checksums, or ELF validation do not replace the managed artifact.
- Pass 004c adds a separate dependency-safe bootstrap APT operation. It validates fixed canonical package names, reuses the strict single-query dpkg state parser, skips all APT mutation when complete, and otherwise performs exactly one old-source metadata refresh followed by one ordered missing-package batch. Normal package, purge, and upgrade operations use fixed noninteractive APT argv and never add metadata refreshes.
- Pass 004c also adds the canonical per-user Flathub operation set: strict fixed-identity remote inspection and convergent ensure, one strict installed-app inspection followed by one ordered missing-ref install batch, and one update targeting configured application IDs while retaining required runtime/related-ref updates and accepting upstream-declared end-of-life replacements. The ensure queries fixed machine-readable per-user state, adds an absent Flathub without race-masking flags, immediately validates its publication, explicitly enables dependency use on every apply, and finally revalidates its exact URL, enabled/GPG/enumeration state, and canonical no-filter state. Remote identity, URL, user scope, and flags are not operation inputs.
- Pass 004d adds operation-owned typed ensure-present and update-current foundations for schema-v1 Cargo and NPM package sets. Cargo resolves fixed Cargo state through `cargo install --list`, ignores valid source-qualified display records for crates.io registry presence, bootstraps and verifies cargo-binstall only when selected mutation requires it, applies one ordered configured package batch through fixed `--no-confirm` and update-only `--force`, and verifies configured postconditions. NPM resolves the FNM default version established by the Node tool operation, accepts only one canonical exact Node version, treats npm's dependency-less empty global root as empty state, runs every JSON state query and configured mutation through `fnm exec --using <version> -- npm`, and verifies configured postconditions without ambient npm. Ensure-present uses one fixed `npm install --global -- <missing...>` batch; update-current uses one fixed unversioned `npm install --global -- <all-configured...>` batch because npm resolves unversioned names through its configured tag (default `latest`), converging both existing and absent configured names without an unscoped global update.
- Both Pass 004d package operations defensively enforce the frozen unversioned package grammars, non-empty unique ordered inputs, strict UTF-8 and state parsing, no shell interpretation, retry-safe ensure selection, and configured-only updates. Their fake-host coverage includes exact argv, scoped NPM names, hostile state, absent managers/default Node, bootstrap and mutation failures, missing postconditions, and ambient npm traps.
- Pass 004e adds operation-owned typed existing-product integration foundations for Docker group membership, VirtualBox group membership, Docker's daemon-wide local log driver, and ordered VS Code extension sets. Every operation defensively validates its own inputs and fixed-environment username before mutation, probes the canonical existing product CLI, strictly parses UTF-8 state, applies only missing state through fixed argv without a shell or package manager, and verifies mutations. Group operations use exact `getent`/`id` records, create only their fixed system group when absent, never run `newgrp`, and leave new supplementary membership to logout/login. Docker preserves unrelated JSON, publishes `/etc/docker/daemon.json` through the accepted root-owned mode-`0644` atomic helper, and performs no daemon restart or reload; active behavior changes at Docker's normal reload/restart boundary. VS Code deduplicates valid repeated installed records, installs exact missing IDs in configuration order, and requeries once.
- Pass 004e fake-host coverage includes missing and malformed products, hostile inputs, exact and similar group/user records, absent groups, membership no-op, mutation and postcondition failures, Docker destination and JSON types, omitted and supplied max size, unrelated-data preservation, every publication failure phase, exact argv, no shell, ordered VS Code selection, malformed/non-UTF-8/fatal extension state, and retry-safe second apply.
- The legacy `DownloadBinary` operation, legacy Flatpak steps, legacy `CargoPackages { force }`, standalone `NpmPackages`, `DockerConfig`, `VirtualBoxConfig`, and `VsCodeExtension` variants, and production planner remain unchanged until the atomic integration pass. The new APT bootstrap, Flatpak, Cargo package-set, NPM package-set, and existing-product integration operations are executable foundations only; no schema-v1 integration intent is lowered yet. Remaining milestone-4 work includes atomically switching those intents to the typed operations while lowering all other `PlannedAction` variants, including `DirectPackageIntent` and `UpdateAction::Direct`, plus the other fixed-manager, dotfile, and desktop handlers listed below.

Affected paths:

- `src/operations/`
- `src/json_helpers.rs`
- `src/runner.rs`
- focused operation tests in `tests/behavior.rs`

Work:

- Lower every executable `PlannedAction` into fixed `Operation`/`Step` values, then ensure Rustup, cargo-binstall, official Go archives, FNM, NPM, UV, APT, Flatpak, and Stow are the only manager implementations.
- Resolve moving tool targets and direct-package latest-release metadata during lowering/execution, never in the planner.
- Add Rust toolchain selection and canonical Rust target use where host targeting is required.
- Implement deterministic latest-GitHub-release selector matching in include-then-exclude order, with exactly one remaining asset and fixed `deb`/`appimage` installation.
- Implement repository key handling exactly once: download to temporary storage; validate and normalize armored or binary OpenPGP through batch GPG/dearmor; require non-empty output; then use a privileged fixed operation to atomically replace the derived root-owned mode-`0644` keyring. Conversion or publication failure must preserve the previous keyring.
- Make state checks and updates deterministic for `provides`, tools, and configured package sets.
- Make direct-package presence checks require the complete non-empty `provides` sequence.
- Implement the one fixed dotfile backup policy and parameterized integration/desktop operations without adding alternate schema forms. Docker, VirtualBox, and VS Code integration lowering must preflight the existing product and fail clearly when absent; it must never install that product.
- Lower the explicit post-source APT refresh exactly once before all declared APT consumers. APT update policies consume it rather than adding another refresh; any old-source bootstrap metadata setup remains internal to bootstrap and semantically separate.
- Preserve checksum verification where upstream metadata provides it and document remaining trust boundaries.

TDD gate:

- Fake-host tests prove first install, idempotent second apply, requested granular update, include/exclude overlap, zero/one/multiple remaining asset matches, malformed metadata, checksum failure, backup failure, and interrupted download behavior.
- Repository-key behavior tests cover armored input, binary input, malformed input, sanitization collisions, interrupted conversion, first publication, and successful replacement. They assert failures leave the previous keyring byte-for-byte intact and successful files are canonical binary, root-owned mode `0644` at the sole derived signed-by path.
- No test requires network, root, or the developer's real home.

Exit criteria:

- Each schema field has an exercised operation path or an explicit planner-only skip for desktop mismatch.
- Manager selection cannot be influenced by YAML.

### 5. Migrate presets, init, and user documentation atomically

Affected paths:

- `configs/default.yaml`
- `configs/cli.yaml`
- `configs/full.yaml`
- `configs/vm.yaml`
- `build.rs`
- `src/init.rs`
- `README.md`
- `docs/rust-rewrite.md`
- CLI/init tests and bundle fixtures

Work:

- Convert every preset directly to schema v1 with no generated compatibility form.
- Keep all software under `packages`; remove exposed prerequisite lists and unsupported legacy controls while retaining the canonical customizable system, integration, desktop, and update fields.
- Make the embedded default the beginner-focused canonical starting point.
- Update init refresh/ownership behavior for the new embedded bytes.
- Replace legacy documentation and examples with links to the authoritative contract.

TDD gate:

- Parse and plan every preset on its supported canonical architectures using deterministic platform fixtures and latest-release asset manifests. Separately, validate and plan the canonical full reference on amd64 and arm64 and require its expected native-selector failure on arm32 and riscv64 without fake Obsidian assets.
- Init tests compare emitted `cozydot.yaml` with the embedded v1 default and preserve user-owned edits.
- A repository search finds no `!enabled`, `!disabled`, legacy top-level sections, or interpolation placeholders outside historical migration notes.

Exit criteria:

- `cozydot init` emits only schema v1.
- All shipped examples validate and plan.

### 6. Release hardening and cleanup

Affected paths:

- `install.sh`
- `scripts/package-release.sh`
- `.github/workflows/`
- `tests/distribution.rs`
- release and VM documentation

Work:

- Remove dead legacy code and tests made unreachable by milestones 2-5.
- Exercise release archives for supported build architectures.
- Add clean-machine acceptance scripts for supported Debian-family distributions and GNOME/non-desktop paths.
- Record the breaking schema change and lack of automatic legacy migration in release notes.

TDD gate:

- Required local and CI gates pass from a clean checkout.
- Release archive installation and initialized apply are tested from packaged artifacts, not `cargo run`.

Exit criteria:

- No legacy parser, preset, interpolation, or compatibility branch remains.
- Release and VM criteria below are recorded with artifact IDs and logs.

## Migration order

1. Land milestone 1 without changing embedded preset bytes.
2. Build typed v1 parsing and validation against test fixtures not used by production init.
3. Build and test the typed v1 intent planner without operation lowering, production `main`/`apply` switching, or runtime dual-schema selection.
4. Lower typed intents into fixed-manager operations and complete operation behavior tests while production and embedded presets remain legacy-only.
5. In one atomic integration change, switch `main`/`apply`, all presets, init expectations, planner tests, and production planner callers to v1, then delete the legacy parser and planner. V1 becomes the sole runtime schema at this point.
6. Run release/VM acceptance, remove remaining dead legacy material, and publish the breaking release.

The integration change in step 4 must be atomic. If milestones are split across branches, rebase them before merge so the main branch always has a matching parser and embedded default.

## Required checks per milestone

Run and self-fix all failures before commit:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

Milestones touching packaging also run:

```bash
cargo build --release
scripts/package-release.sh
bash -n install.sh
```

## Release acceptance

- A clean release build produces the expected archive and SHA-256 file for each supported release architecture; this release matrix is independent of the canonical reference's selected packages.
- `install.sh` verifies the checksum, rejects malformed archives, and atomically installs only the Cozydot binary.
- With empty `XDG_CONFIG_HOME`, `cozydot init` emits a validating schema v1 file and bundled dotfiles without network access.
- A second `cozydot init` preserves user edits according to the ownership manifest and interruption journal.
- `cozydot apply` validates the entire file before side effects, prints actionable field paths on invalid input, and succeeds twice against the default preset.
- Packaged-binary smoke tests cover `--help`, `--version`, `init`, dry-run apply, and malformed/unsupported-schema rejection.
- Release notes state that legacy tagged YAML is unsupported and must be rewritten; no automatic migration is claimed.

## VM acceptance

Run from packaged artifacts on clean amd64 and arm64 VMs for each supported base distribution. Record image version, architecture, artifact checksum, command log, and result.

- Minimal server VM: `curl -> install -> init -> apply` succeeds without a desktop, then a second apply is idempotent.
- GNOME VM: theme, terminal, idle controls, configured extensions, dock, rounded corners, fonts, and backup-before-Stow behavior match the preset after relogin where required.
- APT repository VM: distro-specific URL selection, default fallback, detected-codename `system` suite, fixed literal suite, derived signed-by path, and native Debian architecture are correct in generated files.
- Tool VM: Rustup/Rust, official Go, FNM/Node, and UV/Python report the requested versions; Cargo and NPM packages are executable.
- Direct-package VM: the canonical full reference succeeds on amd64 and arm64; no arm32/riscv64 VM success is claimed for this selected package. Deterministic arm32/riscv64 fixture acceptance requires a clear native-selector validation failure because upstream Obsidian lacks those assets. Other direct-package fixtures prove the native selector applies `include` then `exclude`, selects exactly one latest-release asset, and fails before download for missing keys or zero/multiple matches.
- Update VM: `updates.apt` applies `off`, `standard`, and `full` to the system APT package set; omission and `null` behave as `off`, and YAML booleans are rejected. Flatpak targets only configured application IDs while allowing required runtimes, related refs, and upstream-declared end-of-life replacements; Cargo/NPM/direct update only configured declarations, and unrelated installed entries remain untouched. Missing required declarations and empty lists produce no step. Moving tool selectors advance to the current target; exact versions remain pinned and are only verified or repaired. Each leaf changes only its exact manager/package set.
- APT-key VM: armored and binary keys publish canonical binary bytes to the derived signed-by path with root ownership and mode `0644`; malformed keys, collisions, and interrupted conversions publish nothing, failed refresh preserves the old keyring, and a successful refresh atomically replaces it.
- Failure VM: invalid schema, unsupported architecture, unavailable asset, bad checksum, interrupted download, and failed privileged command leave no partially published binary or source file.
- Preset matrix: default, CLI, full, and VM presets validate and complete on their intended desktop/server targets, with unsupported product packages removed from that target's preset rather than hidden by compatibility logic.
