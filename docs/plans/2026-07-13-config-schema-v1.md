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
- Define `Arm32` as Armv7/armhf, normalize only its supported machine aliases, and translate Debian, Go, Rust, and release-asset names internally. Keep Go's official `armv6l` archive name as an output translation, not a supported host alias.
- Migrate current architecture callers without changing embedded YAML.

TDD gate:

- Table tests cover every required normalization alias and all ecosystem translations, and prove `armv6l` is rejected rather than assigned the Armv7 Rust target.
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
- `src/main.rs`
- `tests/fixtures/`
- new focused configuration tests under `tests/`

Work:

- Define Serde-backed structs and enums matching only the authoritative contract.
- Require integer `schema: 1`; deny unknown fields recursively.
- Preserve the contract's distinction between omitted/`null` host-state controls and an explicit `false`; do not create alternate public forms while normalizing enable-only controls and empty collections.
- Validate names, URLs, versions, duration strings, duplicate entries, managed APT components, repository URL selection, `system` suite resolution, direct-asset selector mappings, and cross-field requirements.
- Return field-path errors before platform mutation or command execution.
- Delete tag handling, string-path lookup, purge-tag mutation, and legacy validation after all callers use typed values.

TDD gate:

- Start with one minimal valid fixture and one complete valid fixture.
- Add one negative test for each unknown field, wrong type, enum, URL, duplicate, missing required child, schema version, invalid component, invalid duration string, invalid wildcard pattern, invalid field dependency, and native-asset failure class. Prove numeric YAML values are invalid for `desktop.idle.timeout`.
- Assert omission and `null` preservation, meaningful explicit `false` host-state controls, enable-only `false`, and empty-collection behavior for their respective fields.
- Reject every removed system, dotfile, integration, and desktop form, plus boolean or nested `updates.apt` values, scalar `provides`, scalar asset selectors, selectors missing `include` or `exclude`, and architecture interpolation or substitution.
- `cargo test --all-targets --all-features` passes with no legacy fixture dependency.

Exit criteria:

- Runtime accepts schema v1 only.
- Invalid input never reaches the planner.
- No generic `serde_yaml::Value` path access remains in production configuration code.

### 3. Replace planner inputs with typed intent

Affected paths:

- `src/planner.rs`
- `src/operations/mod.rs`
- `src/runner.rs` where typed steps require adjustment
- `tests/plans.rs`
- `tests/behavior.rs`

Work:

- Plan in contract order: explicit system controls, removals, repositories/APT, Flatpak, tools, Cargo/NPM/direct packages, fonts, dotfiles, integrations, desktop, then enabled granular updates.
- Infer prerequisites from enabled features and coalesce APT metadata refresh/install work.
- Generate managed distribution sources from detected platform data and configured components. Generate third-party APT source entries from typed fields, the selected distro URL, `system` codename resolution or fixed literal suite, inferred key path, and `Architecture::debian()`.
- Select one direct-package asset selector by canonical architecture, match its required anchored `include`, remove matches for its required `exclude` sequence, require exactly one remaining latest-release asset, and use the fixed format handler.
- Plan fixed backup-before-adoption dotfile behavior and the canonical Docker, VirtualBox, VS Code, theme, terminal, idle, and GNOME controls independently.
- Translate `updates.apt` scalar policies and each Flatpak, tool, and package update leaf independently.
- Remove interpolation expansion and every raw legacy repository/binary planning path.
- Keep config-derived values as process arguments or stdin to fixed operations; never generate arbitrary shell source.

TDD gate:

- Snapshot or exact-step tests cover minimal, complete, disabled, and mixed configurations.
- Ordering tests prove prerequisites precede consumers and each shared refresh/bootstrap occurs once.
- System tests cover preserve/managed APT sources, component compatibility, admin membership, unattended upgrades, every supported lowercase distro ID, and Ubuntu Snap/codecs controls on the Ubuntu upstream family including supported derivatives and their non-Ubuntu-family skip behavior, without an opaque preparation path.
- Repository tests cover distro URL precedence, default fallback, `system` codename resolution, fixed literal suites, signed-by output, and native architecture.
- Direct-package tests cover all canonical architecture keys, absent-native-selector failure, required `include` and `exclude` children, exclusion of overlapping architecture assets, one-remaining-asset selection, and clear zero/multiple-remaining-asset failures. They reject paths, substitutions, and invalid wildcard syntax.
- Dotfile tests prove every conflict is backed up before Stow and that backup failure prevents adoption.
- Integration and desktop tests cover each child independently, accept `desktop.idle.timeout` duration strings including disabling `0s`, and reject numeric idle values plus boolean or legacy shorthand forms.
- Update tests cover `off`, `standard`, and `full` APT string policies, reject YAML booleans, and cover every granular sibling without cross-enablement. Omitted and `null` APT policies behave as `off`.
- Existing operation behavior tests are migrated rather than duplicated.

Exit criteria:

- Planner has no dotted string lookups, YAML tags, raw repository lines, public interpolation, or user-selectable conflict/manager algorithms. Direct-package wildcard matching is confined to latest-release asset selection.
- Every installation is sourced from `packages` or inferred as an internal prerequisite.

### 4. Complete fixed-manager operations

Affected paths:

- `src/operations/`
- `src/json_helpers.rs`
- `src/runner.rs`
- focused operation tests in `tests/behavior.rs`

Work:

- Ensure Rustup, cargo-binstall, official Go archives, FNM, NPM, UV, APT, Flatpak, and Stow are the only manager implementations.
- Add Rust toolchain selection and canonical Rust target use where host targeting is required.
- Implement deterministic latest-GitHub-release selector matching in include-then-exclude order, with exactly one remaining asset and fixed `deb`/`appimage` installation.
- Make state checks and updates deterministic for `provides`, tools, and configured package sets.
- Make direct-package presence checks require the complete non-empty `provides` sequence.
- Implement the one fixed dotfile backup policy and parameterized integration/desktop operations without adding alternate schema forms.
- Preserve checksum verification where upstream metadata provides it and document remaining trust boundaries.

TDD gate:

- Fake-host tests prove first install, idempotent second apply, requested granular update, include/exclude overlap, zero/one/multiple remaining asset matches, malformed metadata, checksum failure, backup failure, and interrupted download behavior.
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

- Parse and plan every preset on each supported canonical architecture using deterministic platform fixtures and latest-release asset manifests.
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
3. Move planner and operation callers to typed configuration while the embedded preset remains legacy only on the pre-migration branch; do not merge a revision in which production `apply` cannot parse its embedded default.
4. In one integration change, switch `main`, all presets, init expectations, and planner tests to v1 and delete the legacy parser.
5. Complete fixed-manager behavior and update tests before broadening release testing.
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

- A clean release build produces the expected archive and SHA-256 file for each supported release architecture.
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
- APT repository VM: distro-specific URL selection, default fallback, detected-codename `system` suite, fixed literal suite, inferred signed-by path, and native Debian architecture are correct in generated files.
- Tool VM: Rustup/Rust, official Go, FNM/Node, and UV/Python report the requested versions; Cargo and NPM packages are executable.
- Direct-package VM: the native canonical selector applies `include` then `exclude` and selects exactly one remaining latest-release asset; missing architecture keys and zero/multiple remaining matches fail before download.
- Update VM: `updates.apt` implements exactly the string policies `off`, `standard`, and `full`; omission and `null` behave as `off`, YAML booleans are rejected, and each other update leaf changes only its manager/package set. Omitted, `null`, `false`, and empty enable-only controls produce no update step for those boolean leaves.
- Failure VM: invalid schema, unsupported architecture, unavailable asset, bad checksum, interrupted download, and failed privileged command leave no partially published binary or source file.
- Preset matrix: default, CLI, full, and VM presets validate and complete on their intended desktop/server targets, with unsupported product packages removed from that target's preset rather than hidden by compatibility logic.
