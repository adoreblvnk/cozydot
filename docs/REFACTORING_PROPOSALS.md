# Refactoring Proposals

Audited at `6851aae` after reviewing the repository and its previous 50 commits. Line numbers refer to that revision.

The proposals continue 3 themes already visible in the history:

1. Remove structure that no longer carries policy or meaning.
2. Keep declarations, validation, documentation, and tests aligned with execution order.
3. Make the implemented and tested product boundary match the stated product boundary.

## Provenance standard

Every recommendation identifies why its name, shape, or behavior is authoritative:

- **External** — follows an upstream API, format, command, glossary, platform contract, or established Rust convention.
- **Internal** — follows an existing Cozydot concept, execution path, repository convention, or explicit commit intent.
- **Both** — the external term or behavior also resolves an inconsistency inside Cozydot.

This distinction explains earlier renames more precisely:

| Earlier change | Provenance | Justification |
|---|---|---|
| `MacOS` to `Macos` | External | Rust's [API naming guidelines](https://rust-lang.github.io/api-guidelines/naming.html#casing-conforms-to-rfc-430-c-case) treat acronyms as words in `UpperCamelCase`. |
| APT `urls` to `uris` | External | APT's [`sources.list(5)`](https://manpages.debian.org/testing/apt/sources.list.5.en.html#URI_SPECIFICATION) calls these values URIs and permits schemes beyond HTTP(S). |
| GNOME `extension` value to `uuid` | External | GNOME calls the extension identifier a [`uuid`](https://gjs.guide/extensions/overview/anatomy.html#uuid). |
| Stow `root` and `sources` to `stow_dir` and `package_dirs` | External | The [GNU Stow glossary](https://www.gnu.org/software/stow/manual/stow.html#Terminology) defines those terms and warns that “source” is ambiguous. |
| `ActiveHost::root` to `config_dir` | Both | The [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/latest/) defines the configuration directory, while `src/paths.rs` already used `config_dir`. |
| `resolve` and `find_brew` to `find_executable` | Internal | `fnm` and Homebrew perform the same executable-discovery operation; one local verb now describes that concept in both modules. |
| `executable_file` to `is_executable` and `executable_on_path` to `has_executable_on_path` | Internal | Boolean helpers now read as predicates and use the same executable vocabulary throughout host operations. |

## 1. Removing unnecessary architecture

### 1.1 Inline the one-use YAML deserializer

| Field | Value |
|---|---|
| Priority | Low |
| File & lines | `src/config.rs:30-40` |
| Provenance | Both |

**Proposal:** inline `yaml_serde::from_str(&text).context("config")?` into `Config::load` and remove `Config::deserialize_str`.

**Reasoning:** `deserialize_str` has 1 caller, contains 1 expression, is not a test seam, and adds no Cozydot policy. Reading `Config::load` should show read, deserialize, validate, and return in one place.

**Justification:** internally, `AGENTS.md` says to inline helpers used only once. Externally, [`yaml_serde::from_str`](https://docs.rs/yaml_serde/latest/yaml_serde/fn.from_str.html) is already the direct typed-deserialization API, so the wrapper does not improve the dependency vocabulary.

**Verification:** existing configuration diagnostics in `tests/cli.rs:230-283` must remain unchanged.

### 1.2 Narrow Linux update input to architecture

| Field | Value |
|---|---|
| Priority | Medium |
| File & lines | `src/workflow.rs:55-60`, `src/workflow.rs:238-257` |
| Provenance | Internal |

**Proposal:** change `linux_update(config, platform)` to `linux_update(config, platform.architecture)` and accept `Architecture`, matching `macos_update`.

**Reasoning:** `linux_update` reads only `platform.architecture`. Accepting the complete `Platform` suggests distro, codenames, desktop, or identity affect updates when none do. The narrower input exposes the real dependency at the call site.

**Justification:** this continues commit `5941863` (`refactor: simplify workflow dependencies`), which narrowed helpers from `Config` to the sections they consume. It also makes Linux and macOS update dispatch internally consistent without adding a new type or abstraction.

**Verification:** `empty_apply_and_update_establish_the_linux_baseline` and `update_runs_only_the_selected_apt_upgrade_command` must keep the same command logs.

### 1.3 Remove obsolete `uname` test doubles

| Field | Value |
|---|---|
| Priority | High |
| File & lines | `tests/cli.rs:73-76`, `tests/cli.rs:230-254` |
| Provenance | Both |

**Proposal:** remove the fake `uname` from `write_linux_host_fakes`; remove the platform probe from `validation_happens_before_platform_detection_or_mutation`; rename that test to the behavior it still proves, such as `invalid_config_prevents_host_mutation`.

**Reasoning:** commit `0e34fce` moved platform detection from a subprocess to `rustix::system::uname`, but the old executable fakes remained. The probe can no longer observe platform detection, so the present test name overstates its coverage. The mutation sentinels still prove the useful contract: invalid configuration prevents host-changing commands.

**Justification:** internally, test setup should name and exercise current behavior. Externally, [`rustix::system::uname`](https://docs.rs/rustix/latest/rustix/system/fn.uname.html) reads runtime OS and hardware information directly; it does not resolve `uname` through `PATH`. The Rust Book describes tests as setup, execution, and assertions about the behavior actually under test ([How to Write Tests](https://doc.rust-lang.org/book/ch11-01-writing-tests.html)).

**Verification:** the renamed test must still fail on malformed YAML and prove that none of the mutation sentinels ran.

## 2. Making execution readable in source order

### 2.1 Order apply-time tool fields by apply execution

| Field | Value |
|---|---|
| Priority | Low |
| File & lines | `src/config.rs:102-109`, `src/workflow.rs:208-225`, `configs/cozydot.yaml:5-9` |
| Provenance | Internal |

**Proposal:** order `Tools` as `rust`, `node`, `python`, `go`.

**Reasoning:** the primary preset declares tools in that order and `apply_tools` executes them in that order, but the Rust struct currently places `go` second. Serde mappings are name-based, so this is behavior-neutral and makes the schema definition mirror the user-facing declaration and runtime.

**Justification:** this continues commits `2c307a7`, `43501e5`, and `d45aa43`, which made source read in execution order. It is internal consistency; there is no upstream authority for ordering unrelated toolchains.

**Verification:** `cargo test` and `scripts/generate-configs.sh --check` must remain unchanged.

### 2.2 Validate configuration in schema order

| Field | Value |
|---|---|
| Priority | Medium |
| File & lines | `src/config.rs:43-69`, `src/config.rs:90-183`, `src/config.rs:185-278`, `src/config.rs:581-670` |
| Provenance | Internal |

**Proposal:** order `Config::validate` as shared validation, Linux validation, then macOS validation, following `Config { shared, linux, macos }`. Within shared validation, keep tool/package dependency checks beside `shared.tools` and `shared.packages` before fonts and dotfiles.

**Reasoning:** validation currently starts at `linux.packages`, jumps to shared fonts and dotfiles, visits both platform dotfile sections, then returns to shared tool/package relationships. A reader cannot scan it in the same order as the schema or YAML. Reordering needs no new section-level validator.

**Justification:** the provenance is Cozydot's own schema order and the established source-order refactors. The only behavior change is which error appears first when one file contains multiple independent errors; declaration order is the more predictable precedence.

**Verification:** retain every existing error string and add one multi-error assertion only if first-error precedence is considered a public CLI contract.

### 2.3 Resolve unowned TODO comments

| Field | Value |
|---|---|
| Priority | Low |
| File & lines | `src/init.rs:204`, `src/operations/desktop/gnome.rs:85`, `src/operations/desktop/gnome.rs:101`, `src/operations/packages/apt/repo.rs:90` |
| Provenance | Both |

**Proposal:** remove these 4 comments unless a concrete change is still intended; if work remains, replace each with a tracked issue reference and the constraint that makes the current implementation temporary.

**Reasoning:** “do we really need this,” “can we simplify this,” and “review this” do not explain behavior or define an action. Git history already contains the relevant implementation review, while these comments now interrupt otherwise linear code.

**Justification:** internally, `AGENTS.md` requires comments to be directly useful. Externally, Apollo's [Rust Best Practices](https://github.com/apollographql/rust-best-practices) says TODOs should become issues and comments should explain non-obvious context rather than restate uncertainty.

**Verification:** no runtime verification is needed; `cargo fmt` and Clippy are sufficient after comment-only removal.

### 2.4 Make the macOS Stow documentation match execution

| Field | Value |
|---|---|
| Priority | Medium |
| File & lines | `docs/ARCHITECTURE.md:68-80`, `src/workflow.rs:178-205` |
| Provenance | Internal |

**Proposal:** change the macOS apply steps to state that Stow is an unconditional macOS prerequisite and is added to the Homebrew formula list even when no dotfile package is configured.

**Reasoning:** the document still says the workflow derives whether dotfiles require Stow and adds it only when dotfiles are configured. The implementation always adds it, intentionally introduced by commit `1813bb3` (`refactor: make Stow a macOS prerequisite`). The guide should describe the current source order, not the superseded conditional behavior.

**Justification:** internal execution and explicit commit intent are authoritative here. GNU Stow's manual confirms that Stow operates on named package directories, but the choice to make it a baseline macOS prerequisite is Cozydot policy ([GNU Stow terminology](https://www.gnu.org/software/stow/manual/stow.html#Terminology)).

**Verification:** compare the numbered macOS apply sequence with `macos_apply` line by line.

## 3. Tightening the actual product scope

### 3.1 Remove the untracked `bin` package from the public preset contract

| Field | Value |
|---|---|
| Priority | Critical |
| File & lines | `configs/cozydot.yaml:32-44`, `configs/cli.yaml:26-35`, `configs/vm.yaml:20-29`, `build.rs:16-20`, `build.rs:46-66`, `.github/workflows/rust.yml:31-48`, `.gitignore:1` |
| Provenance | Both |

**Proposal:** remove `bin` from `configs/cozydot.yaml`, regenerate `cli.yaml` and `vm.yaml`, remove the CI assertion for `round`, and either move the private `dotfiles/bin` tree outside the embedded `dotfiles` root or explicitly exclude that package from `build.rs`. Add the chosen local-only path to `.gitignore`.

**Reasoning:** commit `8e877cd` deliberately stopped tracking every file in `dotfiles/bin`, but all 3 presets still select the package and CI still requires a deleted executable. `build.rs` walks the live filesystem, so a local build silently embeds the untracked scripts while a clean checkout does not. A clean-checkout build was verified to initialize every tracked package but no `dotfiles/bin`; the current CI assertion at line 47 therefore cannot pass from `HEAD`.

The simplest product boundary is that official presets refer only to bundled packages. A private `bin` package can still be added directly to the user's active Cozydot config directory, but it should not be an undeclared input to the public binary.

**Justification:** internally, this completes the intent of `8e877cd` and restores the claimed deterministic release input. Externally, Git defines `.gitignore` for [intentionally untracked files](https://git-scm.com/docs/gitignore), Cargo build scripts consume and monitor filesystem paths rather than Git index membership ([Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html#change-detection)), and GNU Stow requires a selected package to have a package directory in the stow directory ([terminology](https://www.gnu.org/software/stow/manual/stow.html#Terminology)).

**Verification:** build from `git archive HEAD`, run `cozydot init` for all 3 presets, assert every configured dotfile package exists, and run the release smoke test in a clean checkout.

### 3.2 Add executable tests for the supported distro contract

| Field | Value |
|---|---|
| Priority | High |
| File & lines | `src/platform.rs:39-60`, `src/platform.rs:64-99`, `src/platform.rs:140-175` |
| Provenance | Both |

**Proposal:** add focused unit tests beside `platform.rs` for Debian bookworm/trixie, Ubuntu, Pop!_OS, Ubuntu-based Linux Mint, Debian-based Linux Mint, both Linux architectures, rejected distros, and rejected Debian releases.

**Reasoning:** these are explicit product promises in `AGENTS.md`, but current integration tests read the CI host's real `/etc/os-release`. They therefore prove only the runner's distro, not Pop!_OS or either Linux Mint family. `etc_os_release::OsRelease::from_str` already makes table-free unit fixtures possible without adding dependency injection.

**Justification:** externally, the [`os-release` specification](https://github.com/systemd/systemd/blob/main/man/os-release.xml) defines `ID` and derivative relationships through `ID_LIKE`; [`etc-os-release`](https://docs.rs/etc-os-release/latest/etc_os_release/struct.OsRelease.html) supports parsing fixtures from strings. Internally, the expected identities and families come from Cozydot's supported-platform list, so tests make that scope executable rather than implied.

**Verification:** each supported identity/family and rejection rule gets a descriptive test with one behavior, then `cargo test --all-targets --all-features` passes.

### 3.3 Build and smoke-test Linux ARM64 in CI

| Field | Value |
|---|---|
| Priority | High |
| File & lines | `.github/workflows/rust.yml:10-50`, `install.sh:8-12`, `scripts/package-release.sh:11-15`, `src/platform.rs:140-151` |
| Provenance | Both |

**Proposal:** add a native `ubuntu-24.04-arm` release build and installer smoke test for `cozydot-1.0.0-linux-arm64.tar.gz`. Keep the existing x86_64 test job for linting and the full suite unless duplicated test time is intentionally accepted.

**Reasoning:** the installer, packager, platform model, binary maps, and supported-platform contract all advertise Linux ARM64, but CI currently builds Linux only on x86_64. Native ARM CI checks compilation, packaging, installer asset naming, and embedded initialization without cross-linker setup.

**Justification:** internally, this tests an already-promised architecture instead of expanding scope. Externally, GitHub documents `ubuntu-24.04-arm` as its native Linux ARM64 runner label ([GitHub-hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)). The runner is currently public preview, so the tradeoff is possible runner instability; if that is unacceptable, the honest alternative is to stop advertising an unverified release artifact.

**Verification:** checksum, one-file archive, installer, `cozydot init`, and `--version` assertions should mirror the x86_64 release smoke test with the ARM64 asset name.

### 3.4 Select the VirtualBox repository suite by base family

| Field | Value |
|---|---|
| Priority | High |
| File & lines | `configs/cozydot.yaml:172-183`, `src/config.rs:308-315`, `src/config.rs:386-392`, `src/platform.rs:84-99` |
| Provenance | Both |

**Proposal:** replace the VirtualBox repo's `uris.default` entry with identical `uris.ubuntu` and `uris.debian` entries. This makes `suite: codename` select `base_codename` for Pop!_OS and both Linux Mint families through the existing family fallback.

**Reasoning:** `default` intentionally uses `distro_codename`; family keys use `base_codename`. Oracle publishes VirtualBox APT suites for Debian and Ubuntu codenames, not Linux Mint release names. The existing distro-map mechanism already models this distinction, so no schema or compatibility layer is needed.

**Justification:** externally, Oracle's [VirtualBox Linux download instructions](https://www.virtualbox.org/wiki/Linux_Downloads) require a supported Debian or Ubuntu distribution codename, and the upstream [repository index](https://download.virtualbox.org/virtualbox/debian/dists/) exposes those suites. Internally, `Distro::family` and `select_repo_codename` already exist specifically to map derivatives to their base codename.

This proposal is separate from, and does not implement, an ARM64 guard for VirtualBox group integration.

**Verification:** add config-selection tests proving Pop!_OS and Ubuntu-based Mint select an Ubuntu base codename, Debian-based Mint selects a Debian base codename, and Ubuntu/Debian remain unchanged.

### 3.5 Restart only affected macOS processes

| Field | Value |
|---|---|
| Priority | Medium |
| File & lines | `src/operations/desktop/macos.rs:7-50`, `src/config.rs:613-657` |
| Provenance | Both |

**Proposal:** restart Dock only when a Dock preference was written and Finder only when a Finder-visible preference was written. Theme-only, keyboard-only, and trackpad-only applies should restart neither process.

**Reasoning:** `write_defaults` currently kills both processes after any macOS desktop intent. Since theme is now shared, applying only `shared.desktop.theme` restarts two unrelated processes. The side effects should follow the preference domains actually changed.

**Justification:** internally, `MacDesktop` already separates Dock, Finder, keyboard, and trackpad intent. Externally, Apple documents that preferences belong to application or global domains and shows restarting the affected process only when necessary; its example writes `com.apple.dock` and restarts Dock ([Edit property lists in Terminal](https://support.apple.com/guide/terminal/edit-property-lists-apda49a1bb2-577e-4721-8f25-ffc0836f6997/mac)).

**Verification:** extract only enough command observation to prove Dock settings restart Dock, Finder settings restart Finder, and theme/keyboard/trackpad settings do not restart unrelated processes.

## Deliberate non-proposals

- Do not reintroduce `allowed_platforms`. `Platform::detect` owns host support; repo architecture filters and binary architecture maps own artifact applicability.
- Do not add the excluded VirtualBox ARM64 group-integration guard.
- Do not modify `README.md` as part of this audit.
- Do not replace the flat `Platform` with nested Linux/macOS state types. The current empty macOS codename strings are inelegant, but no consumer reads them on macOS and the redesign would add access layers without fixing observed behavior.
- Do not inline semantic one-use helpers such as `build_apt_repo` or `MacDesktop::has_intent`; their names isolate policy and keep workflow stages readable.
- Do not unify apply and update tool order merely for symmetry. Each current order is documented and behaviorally observable through failure timing; only declaration order that already disagrees with its own execution path should move.

## Recommended sequence

1. Fix the `bin` bundle boundary and clean-checkout CI failure.
2. Correct obsolete `uname` tests and add platform contract tests.
3. Add Linux ARM64 release coverage.
4. Correct VirtualBox base-family suite selection.
5. Apply behavior-neutral source-order and dependency-narrowing changes.
6. Tighten macOS process restarts.
7. Remove or track TODOs and synchronize the architecture guide.
