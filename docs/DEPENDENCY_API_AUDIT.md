# Dependency API audit

Audited on 2026-08-20 against the latest published API documentation and release notes. The goal is to use dependency features only where they reduce physical LOC, improve readability, remove unnecessary work, or improve behavior without adding speculative abstraction.

Implementation status: the accepted proposals in this report were implemented after the audit. "Current" snippets preserve the pre-change baseline used for comparison.

## Summary

| Dependency | Locked | Latest | Recommendation |
|---|---:|---:|---|
| `anyhow` | 1.0.104 | 1.0.104 | Use `Context` on 3 `Option` results and `ensure!` for 1 simple invariant |
| `clap` | 4.6.2 | 4.6.6 | Update the lockfile; no source API change |
| `humantime` | 2.4.0 | 2.4.0 | Keep current code |
| `serde` | 1.0.229 | 1.0.229 | Keep current derives and manual duration deserializer |
| `serde_path_to_error` | 0.1.20 | 0.1.20 | Remove after adding YAML diagnostic tests |
| `yaml_serde` | 0.10.4 | 0.10.7 | Update and deserialize directly with `from_str` |
| `serde_json` | 1.0.150 | 1.0.151 | Update; use `from_slice` for Docker output and typed maps in tests |
| `sha2` | 0.10.9 | 0.11.0 | Update and use `hex` 0.4.3 for digest encoding |
| `tempfile` | 3.27.0 | 3.27.0 | Chain 2 builders and use 3 prefix constructors |
| `regex` | 1.13.1 | 1.13.1 | Keep dynamic regex compilation; collect matches with an iterator |
| `rustix` | 1.1.4 | 1.1.4 | Add `system` and replace 2 `uname` subprocesses |
| `etc-os-release` | 0.1.1 | 0.1.1 | Keep current code |

The recommended source changes remove approximately 30-35 physical lines after rustfmt. Removing `serde_path_to_error` and adding `hex` keeps the direct dependency count unchanged. The version updates are `clap` 4.6.6, `yaml_serde` 0.10.7, `serde_json` 1.0.151, and `sha2` 0.11.0.

## 1. `anyhow`

### Documentation and new features

Cozydot is already on the latest release, 1.0.104. That release only updates development dependencies; it adds no relevant runtime API. The useful APIs below are existing, underused APIs:

- `Context::context` adds static context to an `Option` or `Result`
- `Context::with_context` remains preferable when constructing context requires formatting
- `ensure!` expresses a required invariant and returns early when it is false

Sources: [API](https://docs.rs/anyhow/1.0.104/anyhow/), [`Context`](https://docs.rs/anyhow/1.0.104/anyhow/trait.Context.html), [`ensure!`](https://docs.rs/anyhow/1.0.104/anyhow/macro.ensure.html), [1.0.104 release](https://github.com/dtolnay/anyhow/releases/tag/1.0.104).

### Proposal: use `Option::context`

`src/operations/packages/homebrew.rs:40`

Current:

```rust
program.to_str().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("Homebrew executable path is not UTF-8"))
```

Proposed:

```rust
program.to_str().map(str::to_owned).context("Homebrew executable path is not UTF-8")
```

This is LOC-neutral but removes an unnecessary closure and explicit error construction. Add `Context` to the existing `anyhow` import.

`src/operations/toolchains/fnm.rs:36-38`

Current:

```rust
let Some(fnm) = find_executable(host)? else {
    bail!("fnm install: fnm is unavailable after install");
};
```

Proposed:

```rust
let fnm = find_executable(host)?.context("fnm install: fnm is unavailable after install")?;
```

`src/operations/packages/npm.rs:5-7` has the same shape:

```rust
let fnm = fnm::find_executable(host)?.context("npm install: managed fnm is unavailable after install")?;
```

These 2 conversions remove 4 physical lines total and keep the same messages and return timing. This is both syntactic sugar and a readability improvement.

**Recommendation:** use all 3 conversions.

### Proposal: use `ensure!` for a positive invariant

`src/init.rs:237-242`

Current:

```rust
fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 record");
    }
    Ok(())
}
```

Proposed:

```rust
fn validate_hash(hash: &str) -> Result<()> {
    ensure!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()), "invalid SHA-256 record");
    Ok(())
}
```

This removes 3 physical lines. The positive valid-hash predicate is easy to read. Do not mechanically convert complex validation branches such as `Repo::validate`: those require De Morgan inversions and become denser despite saving lines.

**Recommendation:** use only for this simple invariant.

## 2. `clap`

### Documentation and new features

The lockfile has 4.6.2; 4.6.6 is current. Changes through 4.6.6 include derive attributes accepting expressions, corrected help rendering for optional named values, internal `syn` updates, and `Command::get_overridden_usage`. None simplifies Cozydot's CLI.

Sources: [latest API](https://docs.rs/clap/4.6.6/clap/), [derive reference](https://docs.rs/clap/4.6.6/clap/_derive/), [changelog](https://github.com/clap-rs/clap/blob/v4.6.6/CHANGELOG.md), [4.6.6 release](https://github.com/clap-rs/clap/releases/tag/v4.6.6).

### Proposal: update the lockfile

`Cargo.toml:15` already permits the update:

```toml
clap = { version = "4.6", features = ["derive"] }
```

Proposed command:

```sh
cargo update -p clap
```

This has no source LOC effect. Run the CLI contract tests because help output is part of Cozydot's behavior.

**Recommendation:** update.

### Rejected: required subcommand

`src/main.rs:14-16,59-64`

Current:

```rust
#[command(subcommand)]
command: Option<Command>,
```

It is tempting to use:

```rust
#[command(subcommand)]
command: Command,
```

This would remove the manual no-command help branch and about 5 lines, but it changes bare `cozydot` from successful help output to Clap's missing-subcommand error behavior. `tests/cli.rs:78-89` deliberately requires success.

**Recommendation:** keep the optional subcommand and manual help path.

## 3. `humantime`

### Documentation and new features

Cozydot is on the latest release, 2.4.0. Its notable addition is `humantime::Duration::new` becoming `const`; Cozydot parses runtime configuration and cannot benefit from it.

Sources: [API](https://docs.rs/humantime/2.4.0/humantime/), [`parse_duration`](https://docs.rs/humantime/2.4.0/humantime/fn.parse_duration.html), [`Duration`](https://docs.rs/humantime/2.4.0/humantime/struct.Duration.html), [2.4.0 release](https://github.com/chronotope/humantime/releases/tag/v2.4.0).

### Rejected: parse through the wrapper type

`src/config.rs:571-573`

Current:

```rust
let value = String::deserialize(deserializer)?;
let duration = humantime::parse_duration(&value).map_err(de::Error::custom)?;
```

Alternative:

```rust
let value = String::deserialize(deserializer)?;
let duration: humantime::Duration = value.parse().map_err(de::Error::custom)?;
```

This saves no lines and introduces a wrapper used only locally. It does not remove Cozydot's whole-second or `u32` range checks.

**Recommendation:** keep `parse_duration`.

## 4. `serde`

### Documentation and new features

Cozydot is on the latest release, 1.0.229. The release updates derive internals to `syn` 3 but adds no relevant user-facing feature. Cozydot already uses the important APIs correctly: `Deserialize`, `deny_unknown_fields`, defaults, enum tagging, and renaming.

Sources: [API](https://docs.rs/serde/1.0.229/serde/), [attributes](https://serde.rs/attributes.html), [container attributes](https://serde.rs/container-attrs.html), [field attributes](https://serde.rs/field-attrs.html), [1.0.229 release](https://github.com/serde-rs/serde/releases/tag/v1.0.229).

### Rejected: derive through `try_from`

`src/config.rs:566-581`

Current code keeps YAML-facing duration validation in one `Deserialize` implementation. It could be split into:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct DesktopIdleDuration(u32);

impl TryFrom<String> for DesktopIdleDuration {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        // same parsing and domain validation
    }
}
```

This is approximately LOC-neutral, allocates string errors, and separates validation from its only deserialization consumer.

`#[serde(flatten)]` is also inappropriate because it is incompatible with Cozydot's strict `deny_unknown_fields` policy.

**Recommendation:** keep the manual deserializer and strict attributes.

## 5. `serde_path_to_error`

### Documentation and new features

Cozydot is on the latest release, 0.1.20. Its main recent addition is `no_std`, which does not apply. The current code already uses its convenience `deserialize` function.

Sources: [API](https://docs.rs/serde_path_to_error/0.1.20/serde_path_to_error/), [`Error`](https://docs.rs/serde_path_to_error/0.1.20/serde_path_to_error/struct.Error.html), [`Path`](https://docs.rs/serde_path_to_error/0.1.20/serde_path_to_error/struct.Path.html), [0.1.20 release](https://github.com/dtolnay/path-to-error/releases/tag/0.1.20).

### Proposal: remove it in favor of `yaml_serde` path tracking

`Cargo.toml:18`:

```toml
serde_path_to_error = "0.1"
```

Remove this line together with the `yaml_serde` proposal below.

`yaml_serde` already tracks map keys, sequence indices, custom deserializer failures, and line/column locations. A local probe against 0.10.4 produced:

```text
direct: nested: unknown field `typo`, expected `value` at line 2 column 3
wrapped path: nested.typo
wrapped inner: nested: unknown field `typo`, expected `value` at line 2 column 3
```

The current formatting therefore produces duplicated paths such as `nested.typo: nested: ...`. Direct YAML errors are shorter, still identify the unknown field, and preserve its parent path and location. The wrapper can provide a more specific machine path for unknown keys, so diagnostic tests should be added before removal for nested structs, sequence entries, custom duration errors, and root syntax errors.

This removes 1 manifest line and the only use of the crate.

**Recommendation:** remove after locking the desired diagnostics in tests.

## 6. `yaml_serde`

### Documentation and new features

The lockfile has 0.10.4; 0.10.7 is current. Releases 0.10.5 through 0.10.7 mainly add and refine `no_std + alloc`, feature-gate reader support, and update dependency constraints. Cozydot should retain the default `std` feature.

Sources: [latest API](https://docs.rs/yaml_serde/0.10.7/yaml_serde/), [`from_str`](https://docs.rs/yaml_serde/0.10.7/yaml_serde/fn.from_str.html), [`Error`](https://docs.rs/yaml_serde/0.10.7/yaml_serde/struct.Error.html), [0.10.4 to 0.10.7](https://github.com/yaml/yaml-serde/compare/0.10.4...0.10.7).

### Proposal: use direct deserialization

`src/config.rs:39-46`

Current:

```rust
fn deserialize_str(text: &str) -> Result<Self> {
    let deserializer = yaml_serde::Deserializer::from_str(text);
    serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let path = error.path().to_string();
        let path = if path == "." { "config" } else { path.as_str() };
        anyhow::anyhow!("{path}: {}", error.inner())
    })
}
```

Proposed:

```rust
fn deserialize_str(text: &str) -> Result<Self> {
    yaml_serde::from_str(text).context("config")
}
```

This removes 5 source lines and the duplicated path rendering described above. Validation semantics and all `deny_unknown_fields` attributes remain unchanged. Error presentation changes, so add the diagnostic tests before applying it.

Update the manifest requirement to preserve the reviewed version:

```toml
yaml_serde = "0.10.7"
```

**Recommendation:** apply with diagnostic tests.

## 7. `serde_json`

### Documentation and new features

The lockfile has 1.0.150; 1.0.151 is current. Its new API is unsafe `RawValue::from_string_unchecked` behind `raw_value`. Cozydot should not enable or use it. Existing `from_slice`, `from_str`, `to_vec_pretty`, `Value`, and `Map` APIs remain appropriate.

Sources: [API](https://docs.rs/serde_json/1.0.151/serde_json/), [1.0.151 release](https://github.com/serde-rs/json/releases/tag/v1.0.151), [1.0.150 to 1.0.151](https://github.com/serde-rs/json/compare/v1.0.150...v1.0.151).

### Proposal: parse Docker output from bytes

`src/operations/integrations/docker.rs:36-38`

Current:

```rust
let output = host.run("Docker daemon config read", "sudo", ["cat", DOCKER_DAEMON_CONFIG])?;
let text = std::str::from_utf8(&output.stdout).context("Docker daemon config is not valid UTF-8")?;
serde_json::from_str(text).context("Docker daemon config must be a JSON object")
```

Proposed:

```rust
let output = host.run("Docker daemon config read", "sudo", ["cat", DOCKER_DAEMON_CONFIG])?;
serde_json::from_slice(&output.stdout).context("Docker daemon config must be a JSON object")
```

This removes 1 line and one explicit conversion. JSON parsing already validates UTF-8. The tradeoff is combining invalid UTF-8 and invalid JSON under the JSON-object context.

**Recommendation:** use `from_slice`; the distinction is not actionable here.

### Proposal: deserialize test fragments into maps

`tests/cli.rs:34-38`

Current:

```rust
let shared: Value = yaml_serde::from_str(shared).unwrap();
let linux: Value = yaml_serde::from_str(linux).unwrap();
value["shared"].as_object_mut().unwrap().extend(shared.as_object().unwrap().clone());
value["linux"].as_object_mut().unwrap().extend(linux.as_object().unwrap().clone());
```

Proposed:

```rust
let shared: Map<String, Value> = yaml_serde::from_str(shared).unwrap();
let linux: Map<String, Value> = yaml_serde::from_str(linux).unwrap();
value["shared"].as_object_mut().unwrap().extend(shared);
value["linux"].as_object_mut().unwrap().extend(linux);
```

Change the import to:

```rust
use serde_json::{Map, Value, json};
```

This is LOC-neutral but removes 2 runtime type checks and 2 complete map clones. The types now state that test fragments must be mappings.

**Recommendation:** use the typed maps.

Update to 1.0.151 as routine patch maintenance.

## 8. `sha2`

### Documentation and new features

The lockfile has 0.10.9; 0.11.0 is current. Version 0.11 moves to Rust 2024/MSRV 1.85, updates the digest ecosystem, and enables runtime-selected SHA acceleration by default on supported x86/x86-64 and AArch64 processors. Cozydot's supported toolchain and architectures satisfy those requirements.

Sources: [0.11 API](https://docs.rs/sha2/0.11.0/sha2/), [`Digest`](https://docs.rs/sha2/0.11.0/sha2/trait.Digest.html), [changelog](https://github.com/RustCrypto/hashes/blob/master/sha2/CHANGELOG.md).

### Proposal: update to 0.11

`Cargo.toml:21`

Current:

```toml
sha2 = "0.10"
```

Proposed:

```toml
sha2 = "0.11"
```

Version 0.11 no longer implements `LowerHex` for digest output. Use the focused `hex` crate for encoding:

```rust
hex::encode(Sha256::digest(bytes))
```

Add the latest `hex` release:

```toml
hex = "0.4"
```

This has no source LOC effect and produces the same lowercase SHA-256 text. The performance feature is automatic, though Cozydot hashes small configuration files and should not expect a meaningful user-visible speedup.

**Recommendation:** update as routine major-version maintenance and run the hash/config initialization tests.

### Rejected: stream managed-file hashes

`src/init.rs:256-258` reads a complete managed file before hashing:

```rust
fn hash_file(path: &Path) -> Result<String> {
    Ok(hash_bytes(&fs::read(path)?))
}
```

Incremental hashing would add about 8 lines and bounded memory, but managed files are small configuration and dotfile assets. No measured problem justifies the added loop.

**Recommendation:** keep the one-shot API.

## 9. `tempfile`

### Documentation and new features

Cozydot is on the latest release, 3.27.0. Newer APIs include `TempPath::try_from_path`; it does not apply because Cozydot creates its own temporary paths. `NamedTempFile::with_prefix_in` is an established convenience constructor that directly replaces 3 one-option builders.

Sources: [API](https://docs.rs/tempfile/3.27.0/tempfile/), [`NamedTempFile`](https://docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html), [`Builder`](https://docs.rs/tempfile/3.27.0/tempfile/struct.Builder.html), [changelog](https://github.com/Stebalien/tempfile/blob/master/CHANGELOG.md).

### Proposal: chain builders in the wrapper

`src/operations/host/mod.rs:96-109`

Current:

```rust
let mut builder = tempfile::Builder::new();
builder.prefix(stem);
builder.suffix(suffix);
let file = builder.tempfile().context("create temporary file")?;
```

Proposed:

```rust
let file = tempfile::Builder::new().prefix(stem).suffix(suffix).tempfile().context("create temporary file")?;
```

Current in-directory variant:

```rust
let mut builder = tempfile::Builder::new();
builder.prefix(stem);
builder.suffix(suffix);
let file = builder.tempfile_in(parent).context("create temporary file")?;
```

Proposed:

```rust
let file =
    tempfile::Builder::new().prefix(stem).suffix(suffix).tempfile_in(parent).context("create temporary file")?;
```

This removes approximately 5 physical lines after rustfmt and matches Cozydot's convention to chain short combinators.

**Recommendation:** use both chains.

### Proposal: use prefix constructors

`src/init.rs:148`

```rust
let mut temp = tempfile::NamedTempFile::with_prefix_in(".cozydot.", dest_parent)?;
```

`src/init.rs:219`

```rust
let mut temp = tempfile::NamedTempFile::with_prefix_in(".managed-files.", parent)?;
```

`src/operations/desktop/mod.rs:65`

```rust
let mut temp = tempfile::NamedTempFile::with_prefix_in(".xdg-terminals.", &config_home)?;
```

Each replaces:

```rust
let mut temp = tempfile::Builder::new().prefix("...").tempfile_in(parent)?;
```

The changes are LOC-neutral but state the returned type and one-option intent directly.

**Recommendation:** use the convenience constructors.

Do not remove Cozydot's `TempPath` wrapper. It centralizes prefix/suffix construction, conversion, cleanup ownership, and contextual errors for 12 call sites.

## 10. `regex`

### Documentation and new features

Cozydot is on the latest release, 1.13.1. Version 1.13 added `regex!` for lazily compiled literal patterns, but Cozydot's asset patterns come from configuration and therefore cannot use that macro. `Regex::new` plus `is_match` is the documented efficient API when only match existence is needed.

Sources: [API](https://docs.rs/regex/1.13.1/regex/), [`Regex`](https://docs.rs/regex/1.13.1/regex/struct.Regex.html), [changelog](https://github.com/rust-lang/regex/blob/master/CHANGELOG.md).

### Proposal: collect matching assets directly

`src/operations/packages/binary/mod.rs:61-67`

Current:

```rust
let pattern = Regex::new(asset_pattern).context("compile binary asset regex")?;
let mut matches = Vec::new();
for asset in &release.assets {
    if pattern.is_match(&asset.name) {
        matches.push(asset);
    }
}
```

Proposed:

```rust
let pattern = Regex::new(asset_pattern).context("compile binary asset regex")?;
let matches = release.assets.iter().filter(|asset| pattern.is_match(&asset.name)).collect::<Vec<_>>();
```

This removes 5 physical lines, preserves release order and the exact-one-match behavior, and keeps allocation behavior materially unchanged.

**Recommendation:** use the iterator form.

### Rejected: `regex!` or `RegexSet`

`regex!` requires a literal, while `asset_pattern` is configuration data. `RegexSet` is optimized for many patterns against one haystack; Cozydot has one pattern and many asset names.

**Recommendation:** keep `Regex::new` and `is_match`.

## 11. `rustix`

### Documentation and new features

Cozydot is on the latest release, 1.1.4. The existing `process::geteuid` use is already direct and correct. The underused `system::uname` API returns the same kernel `utsname.machine` field as `uname -m` without spawning a process.

Sources: [API](https://docs.rs/rustix/1.1.4/rustix/), [`geteuid`](https://docs.rs/rustix/1.1.4/rustix/process/fn.geteuid.html), [`uname`](https://docs.rs/rustix/1.1.4/rustix/system/fn.uname.html), [`Uname`](https://docs.rs/rustix/1.1.4/rustix/system/struct.Uname.html), [changes](https://github.com/bytecodealliance/rustix/blob/v1.1.4/CHANGES.md).

### Proposal: replace `uname -m` subprocesses

`Cargo.toml:24`

Current:

```toml
rustix = { version = "1.1", features = ["process"] }
```

Proposed:

```toml
rustix = { version = "1.1", features = ["process", "system"] }
```

`src/platform.rs:17-36` currently spawns `uname -m` separately in the macOS and Linux paths, then `src/platform.rs:205-213` parses its output.

Current core shape:

```rust
let uname = Command::new("uname").arg("-m").output().context("run uname -m")?;
let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
```

Proposed at the start of `Platform::detect`:

```rust
let uname = rustix::system::uname();
let arch = uname.machine().to_str().context("uname machine architecture is not UTF-8")?;
if arch.is_empty() {
    bail!("uname returned an empty machine architecture");
}
```

Use `arch` in both target branches and delete:

```rust
use std::process::Command;
```

and:

```rust
fn parse_uname_machine(success: bool, stdout: &[u8]) -> Result<String> {
    if !success {
        bail!("uname -m failed");
    }
    let machine = std::str::from_utf8(stdout).context("uname -m output is not UTF-8")?.trim();
    if machine.is_empty() {
        bail!("uname -m returned an empty machine architecture");
    }
    Ok(machine.into())
}
```

This removes approximately 10 physical lines and one process spawn from every platform detection. `Uname::machine` has no command-output newline, so trimming is unnecessary. Rustix documents POSIX, Linux, and Apple implementations, covering Cozydot's supported systems.

**Recommendation:** use `rustix::system::uname`.

### Rejected: use `OsRelease::architecture`

The `ARCHITECTURE` field is optional on supported distributions and describes userspace architecture rather than necessarily matching the running kernel. Its values can also use systemd spellings such as `x86-64`, which do not match Cozydot's current parser.

**Recommendation:** keep kernel architecture detection.

## 12. `etc-os-release`

### Documentation and new features

Cozydot is on the latest release, 0.1.1. Its release changes are documentation and dependency/MSRV maintenance. `OsRelease::open` correctly follows `/etc/os-release` and `/usr/lib/os-release`; `id`, `version_codename`, and `get_value` are appropriate for Cozydot's supported distributions.

Sources: [API](https://docs.rs/etc-os-release/0.1.1/etc_os_release/struct.OsRelease.html), [`id_like`](https://docs.rs/etc-os-release/0.1.1/etc_os_release/struct.OsRelease.html#method.id_like), [changelog](https://github.com/gifnksm/etc-os-release/blob/v0.1.1/CHANGELOG.md), [0.1.1 release](https://github.com/gifnksm/etc-os-release/releases/tag/v0.1.1).

### Rejected: use `id_like`

`src/platform.rs:41-48,85-99` currently passes only the raw `ID_LIKE` value into `Distro::family`, which checks Ubuntu before Debian and includes the original value in errors.

A typed rewrite would need to pass the entire `OsRelease`, call `id_like()` more than once or collect it, and still call `get_value("ID_LIKE")` for the useful diagnostic. It adds approximately 3 lines and increases coupling.

**Recommendation:** keep the current raw-value boundary.

## Proposed order

1. Update `clap`, `yaml_serde`, `serde_json`, and `sha2`, then run the locked test/Clippy matrix.
2. Replace `uname -m` with `rustix::system::uname`.
3. Apply the `anyhow`, `tempfile`, `regex`, and `serde_json` local simplifications.
4. Add YAML diagnostic tests, switch to `yaml_serde::from_str`, and remove `serde_path_to_error` if the resulting messages are accepted.
