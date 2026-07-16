use crate::{domain::HttpsUrl, json_helpers, platform::Architecture};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::{managed_state::ManagedState, Host, TempDir, TempPath};

const TOOL_STATE_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolMutationMode {
    EnsurePresent,
    UpdateMoving,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RustToolchainSelector {
    Stable,
    Beta,
    Nightly,
    DatedNightly(String),
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustToolchainOperation {
    selector: RustToolchainSelector,
    architecture: Architecture,
    mode: ToolMutationMode,
}

impl RustToolchainOperation {
    pub fn new(
        selector: RustToolchainSelector,
        architecture: Architecture,
        mode: ToolMutationMode,
    ) -> Result<Self> {
        validate_rust_selector(&selector)?;
        Ok(Self {
            selector,
            architecture,
            mode,
        })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "rust-toolchain".into(),
            mutation_name(self.mode).into(),
            rust_selector_name(&self.selector).into(),
            self.architecture.rust_target().into(),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoToolchainSelector {
    Latest,
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoToolchainOperation {
    selector: GoToolchainSelector,
    architecture: Architecture,
    mode: ToolMutationMode,
}

impl GoToolchainOperation {
    pub fn new(
        selector: GoToolchainSelector,
        architecture: Architecture,
        mode: ToolMutationMode,
    ) -> Result<Self> {
        if let GoToolchainSelector::Version(version) = &selector {
            validate_numeric_version(version, 2, 3, "Go")?;
        }
        Ok(Self {
            selector,
            architecture,
            mode,
        })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "go-toolchain".into(),
            mutation_name(self.mode).into(),
            match &self.selector {
                GoToolchainSelector::Latest => "latest",
                GoToolchainSelector::Version(version) => version,
            }
            .into(),
            self.architecture.go_archive().into(),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeToolchainSelector {
    Lts,
    Latest,
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeToolchainOperation {
    selector: NodeToolchainSelector,
    architecture: Architecture,
    mode: ToolMutationMode,
}

impl NodeToolchainOperation {
    pub fn new(
        selector: NodeToolchainSelector,
        architecture: Architecture,
        mode: ToolMutationMode,
    ) -> Result<Self> {
        if let NodeToolchainSelector::Version(version) = &selector {
            validate_numeric_version(version, 1, 3, "Node")?;
        }
        Ok(Self {
            selector,
            architecture,
            mode,
        })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "node-toolchain".into(),
            mutation_name(self.mode).into(),
            node_selector_name(&self.selector).into(),
            self.architecture.canonical().into(),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonToolchainOperation {
    version: String,
    architecture: Architecture,
}

impl PythonToolchainOperation {
    pub fn new(version: impl Into<String>, architecture: Architecture) -> Result<Self> {
        let version = version.into();
        validate_numeric_version(&version, 2, 3, "Python")?;
        Ok(Self {
            version,
            architecture,
        })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "python-toolchain".into(),
            self.version.clone(),
            self.architecture.canonical().into(),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolKind {
    Rust,
    Go,
    Node,
    Python,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolStatus {
    Pending,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolRecord {
    version: u64,
    status: ToolStatus,
    tool: ToolKind,
    requested: String,
    resolved: String,
    release: String,
    platform: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolResolution {
    resolved: String,
    release: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GoRelease {
    resolution: ToolResolution,
    filename: String,
    checksum: String,
}

pub(crate) fn execute_rust(host: &Host<'_>, operation: &RustToolchainOperation) -> Result<()> {
    validate_rust_selector(&operation.selector).context("validate Rust toolchain operation")?;
    let rustup = resolve_managed(host, "CARGO_HOME", ".cargo", "bin/rustup")?
        .context("Rust toolchain operation: rustup is unavailable after bootstrap")?;
    let target = operation.architecture.rust_target();
    let requested = rust_selector_name(&operation.selector);
    let (state_store, lock, record) = read_tool_record(host, "rust", ToolKind::Rust)?;
    let refresh = operation.mode == ToolMutationMode::UpdateMoving
        && rust_selector_is_moving(&operation.selector);
    let needs_pending =
        reusable_record(record.as_ref(), ToolKind::Rust, requested, target, refresh)?.is_none();
    let resolution = select_resolution(
        record.as_ref(),
        ToolKind::Rust,
        requested,
        target,
        refresh,
        || resolve_rust_release(host, &operation.selector, target),
    )?;
    if needs_pending {
        publish_tool_record(
            &state_store,
            &lock,
            ToolStatus::Pending,
            ToolKind::Rust,
            requested,
            &resolution,
            target,
        )?;
    }
    let toolchain = format!("{}-{target}", resolution.resolved);
    let current = inspect_rust(host, &rustup, &toolchain)?;
    if current
        .as_ref()
        .is_none_or(|state| state.release != resolution.release || state.host != target)
    {
        host.require(
            "Rust toolchain mutation",
            &rustup,
            [
                "toolchain",
                "install",
                &toolchain,
                "--profile",
                "minimal",
                "--no-self-update",
            ],
        )?;
    }
    let default = rust_default(host, &rustup)?;
    if default.as_deref() != Some(toolchain.as_str()) {
        host.require(
            "Rust default toolchain mutation",
            &rustup,
            ["default", &toolchain],
        )?;
    }
    let state = inspect_rust(host, &rustup, &toolchain)?.with_context(|| {
        format!("Rust toolchain mutation did not install requested toolchain {toolchain}")
    })?;
    if state.release != resolution.release || state.host != target {
        bail!("Rust toolchain mutation produced mismatched release or host state");
    }
    if rust_default(host, &rustup)?.as_deref() != Some(toolchain.as_str()) {
        bail!("Rust default toolchain mutation did not select {toolchain}");
    }
    publish_completed_record(
        &state_store,
        &lock,
        record.as_ref(),
        ToolKind::Rust,
        requested,
        &resolution,
        target,
    )
}

pub(crate) fn execute_go(host: &Host<'_>, operation: &GoToolchainOperation) -> Result<()> {
    if let GoToolchainSelector::Version(version) = &operation.selector {
        validate_numeric_version(version, 2, 3, "Go")?;
    }
    let expected_arch = operation.architecture.go();
    let platform = operation.architecture.canonical();
    let requested = match &operation.selector {
        GoToolchainSelector::Latest => "latest",
        GoToolchainSelector::Version(version) => version,
    };
    let (state_store, lock, record) = read_tool_record(host, "go", ToolKind::Go)?;
    let refresh = operation.mode == ToolMutationMode::UpdateMoving
        && operation.selector == GoToolchainSelector::Latest;
    let reusable = reusable_record(record.as_ref(), ToolKind::Go, requested, platform, refresh)?;
    let mut release = None;
    let resolution = match reusable {
        Some(record) => ToolResolution {
            resolved: record.resolved.clone(),
            release: record.release.clone(),
        },
        None => {
            let resolved = resolve_go_release(host, requested, operation.architecture)?;
            let resolution = resolved.resolution.clone();
            release = Some(resolved);
            resolution
        }
    };
    if reusable.is_none() {
        publish_tool_record(
            &state_store,
            &lock,
            ToolStatus::Pending,
            ToolKind::Go,
            requested,
            &resolution,
            platform,
        )?;
    }
    let current = inspect_go(host, "/usr/local/go/bin/go")?;
    if current.as_ref().is_some_and(|state| {
        state.version == resolution.release && state.architecture == expected_arch
    }) {
        return publish_completed_record(
            &state_store,
            &lock,
            record.as_ref(),
            ToolKind::Go,
            requested,
            &resolution,
            platform,
        );
    }
    let release = release.map(Ok).unwrap_or_else(|| {
        resolve_go_release(host, &resolution.resolved, operation.architecture)
    })?;
    if release.resolution != resolution {
        bail!("Go release metadata changed for the pinned managed release");
    }
    let version = release.resolution.release;
    let filename = release.filename;
    let checksum = release.checksum;
    if checksum.len() != 64
        || !checksum
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("Go release metadata contains an invalid SHA-256 checksum");
    }
    let archive = TempPath::new_with_suffix(host, "go", ".tar.gz")?;
    let stage = TempDir::new(host, "go-stage")?;
    let url = format!("https://go.dev/dl/{filename}");
    host.require(
        "Go archive download",
        "curl",
        [
            "--proto".as_ref(),
            "=https".as_ref(),
            "--location".as_ref(),
            "--fail".as_ref(),
            "--silent".as_ref(),
            "--show-error".as_ref(),
            "--retry".as_ref(),
            "3".as_ref(),
            "--retry-all-errors".as_ref(),
            "--output".as_ref(),
            archive.path().as_os_str(),
            "--".as_ref(),
            url.as_ref(),
        ],
    )?;
    let checksum_input = format!("{checksum}  {}\n", archive.path().display());
    host.require_input(
        "Go archive checksum",
        "sha256sum",
        ["--check", "--status", "-"],
        checksum_input.as_bytes(),
    )?;
    let listing = host.require(
        "Go archive preflight",
        "tar",
        [
            "--list",
            "--gzip",
            "--file",
            &archive.path().to_string_lossy(),
        ],
    )?;
    validate_go_archive_listing(&listing.stdout)?;
    host.require(
        "Go archive extraction",
        "tar",
        [
            "--extract",
            "--gzip",
            "--directory",
            &stage.path().to_string_lossy(),
            "--file",
            &archive.path().to_string_lossy(),
        ],
    )?;
    let staged = stage.path().join("go");
    let staged_binary = staged.join("bin/go");
    let staged_program = path_program(&staged_binary, "staged Go executable")?;
    let staged_state = inspect_go(host, &staged_program)?
        .context("Go archive does not contain an executable Go toolchain")?;
    if staged_state.version != version || staged_state.architecture != expected_arch {
        bail!("Go archive toolchain does not match resolved release metadata");
    }
    host.require(
        "Go toolchain publication",
        "sudo",
        ["rm", "-rf", "--", "/usr/local/go"],
    )?;
    host.require(
        "Go toolchain publication",
        "sudo",
        [
            "mv".as_ref(),
            "--".as_ref(),
            staged.as_os_str(),
            "/usr/local/go".as_ref(),
        ],
    )?;
    let installed = inspect_go(host, "/usr/local/go/bin/go")?
        .context("Go toolchain publication did not create /usr/local/go/bin/go")?;
    if installed.version != version || installed.architecture != expected_arch {
        bail!("Go toolchain publication produced mismatched version or architecture");
    }
    publish_completed_record(
        &state_store,
        &lock,
        record.as_ref(),
        ToolKind::Go,
        requested,
        &resolution,
        platform,
    )
}

pub(crate) fn execute_node(host: &Host<'_>, operation: &NodeToolchainOperation) -> Result<()> {
    if let NodeToolchainSelector::Version(version) = &operation.selector {
        validate_numeric_version(version, 1, 3, "Node")?;
    }
    let fnm = resolve_fnm(host)?;
    let requested = node_selector_name(&operation.selector);
    let platform = operation.architecture.canonical();
    let (state_store, lock, record) = read_tool_record(host, "node", ToolKind::Node)?;
    let refresh = operation.mode == ToolMutationMode::UpdateMoving
        && !matches!(operation.selector, NodeToolchainSelector::Version(_));
    let needs_pending = reusable_record(
        record.as_ref(),
        ToolKind::Node,
        requested,
        platform,
        refresh,
    )?
    .is_none();
    let resolution = select_resolution(
        record.as_ref(),
        ToolKind::Node,
        requested,
        platform,
        refresh,
        || {
            resolve_node_version(host, &fnm, &operation.selector).map(|resolved| ToolResolution {
                release: resolved.clone(),
                resolved,
            })
        },
    )?;
    if needs_pending {
        publish_tool_record(
            &state_store,
            &lock,
            ToolStatus::Pending,
            ToolKind::Node,
            requested,
            &resolution,
            platform,
        )?;
    }
    let alias = node_alias(&operation.selector);
    let current = inspect_node(host, &fnm, &alias)?;
    if current.as_deref() != Some(resolution.resolved.as_str()) {
        host.require(
            "Node toolchain mutation",
            &fnm,
            ["install", &resolution.resolved, "--progress", "never"],
        )?;
        if current.is_some() {
            host.require(
                "Node toolchain alias replacement",
                &fnm,
                ["unalias", &alias],
            )?;
        }
        host.require(
            "Node toolchain alias publication",
            &fnm,
            ["alias", &resolution.resolved, &alias],
        )?;
    }
    let default = fnm_default(host, &fnm)?;
    if default.as_deref() != Some(resolution.resolved.as_str()) {
        host.require(
            "Node default toolchain mutation",
            &fnm,
            ["default", &resolution.resolved],
        )?;
    }
    let installed = inspect_node(host, &fnm, &alias)?
        .context("Node toolchain mutation did not publish the managed selector alias")?;
    if installed != resolution.release || installed != resolution.resolved {
        bail!("Node toolchain mutation produced mismatched version state");
    }
    if fnm_default(host, &fnm)?.as_deref() != Some(resolution.resolved.as_str()) {
        bail!(
            "Node default toolchain mutation did not select {}",
            resolution.resolved
        );
    }
    publish_completed_record(
        &state_store,
        &lock,
        record.as_ref(),
        ToolKind::Node,
        requested,
        &resolution,
        platform,
    )
}

pub(crate) fn execute_python(host: &Host<'_>, operation: &PythonToolchainOperation) -> Result<()> {
    validate_numeric_version(&operation.version, 2, 3, "Python")?;
    let uv = resolve_managed(host, "UV_INSTALL_DIR", ".local/bin", "uv")?
        .context("Python toolchain operation: uv is unavailable after bootstrap")?;
    let platform = operation.architecture.canonical();
    let (state_store, lock, record) = read_tool_record(host, "python", ToolKind::Python)?;
    let needs_pending = reusable_record(
        record.as_ref(),
        ToolKind::Python,
        &operation.version,
        platform,
        false,
    )?
    .is_none();
    let resolution = select_resolution(
        record.as_ref(),
        ToolKind::Python,
        &operation.version,
        platform,
        false,
        || resolve_python_version(host, &uv, &operation.version, operation.architecture),
    )?;
    if needs_pending {
        publish_tool_record(
            &state_store,
            &lock,
            ToolStatus::Pending,
            ToolKind::Python,
            &operation.version,
            &resolution,
            platform,
        )?;
    }
    let current = inspect_python(host, &uv, &resolution.resolved)?;
    if current.as_deref() != Some(resolution.release.as_str()) {
        host.require(
            "Python toolchain mutation",
            &uv,
            [
                "python",
                "install",
                "--no-config",
                "--managed-python",
                "--no-progress",
                "--default",
                &resolution.resolved,
            ],
        )?;
    }
    let installed = inspect_python(host, &uv, &resolution.resolved)?
        .context("Python toolchain mutation did not install a managed interpreter")?;
    if installed != resolution.release {
        bail!("Python toolchain mutation installed mismatched version {installed}");
    }
    publish_completed_record(
        &state_store,
        &lock,
        record.as_ref(),
        ToolKind::Python,
        &operation.version,
        &resolution,
        platform,
    )
}

fn read_tool_record(
    host: &Host<'_>,
    stem: &str,
    expected_tool: ToolKind,
) -> Result<(ManagedState, File, Option<ToolRecord>)> {
    let state = ManagedState::open(host, "tools", stem, "toolchain")?;
    let lock = state.acquire_lock()?;
    let record = state
        .read()?
        .map(|bytes| {
            let record: ToolRecord = super::managed_state::parse_strict_json(&bytes)
                .context("parse strict toolchain managed record")?;
            validate_tool_record(&record)?;
            if record.tool != expected_tool {
                bail!("toolchain managed record has a mismatched tool identity");
            }
            Ok(record)
        })
        .transpose()?;
    state.validate_lock_entry(&lock)?;
    Ok((state, lock, record))
}

fn reusable_record<'a>(
    record: Option<&'a ToolRecord>,
    tool: ToolKind,
    requested: &str,
    platform: &str,
    refresh: bool,
) -> Result<Option<&'a ToolRecord>> {
    let Some(record) = record else {
        return Ok(None);
    };
    let matches =
        record.tool == tool && record.requested == requested && record.platform == platform;
    Ok((matches && (record.status == ToolStatus::Pending || !refresh)).then_some(record))
}

fn select_resolution<F>(
    record: Option<&ToolRecord>,
    tool: ToolKind,
    requested: &str,
    platform: &str,
    refresh: bool,
    resolve: F,
) -> Result<ToolResolution>
where
    F: FnOnce() -> Result<ToolResolution>,
{
    match reusable_record(record, tool, requested, platform, refresh)? {
        Some(record) => Ok(ToolResolution {
            resolved: record.resolved.clone(),
            release: record.release.clone(),
        }),
        None => resolve(),
    }
}

fn publish_tool_record(
    state: &ManagedState,
    lock: &File,
    status: ToolStatus,
    tool: ToolKind,
    requested: &str,
    resolution: &ToolResolution,
    platform: &str,
) -> Result<()> {
    let record = ToolRecord {
        version: TOOL_STATE_VERSION,
        status,
        tool,
        requested: requested.into(),
        resolved: resolution.resolved.clone(),
        release: resolution.release.clone(),
        platform: platform.into(),
    };
    validate_tool_record(&record)?;
    state.validate_lock_entry(lock)?;
    state.publish(&serde_json::to_vec(&record).context("serialize toolchain managed record")?)
}

fn publish_completed_record(
    state: &ManagedState,
    lock: &File,
    existing: Option<&ToolRecord>,
    tool: ToolKind,
    requested: &str,
    resolution: &ToolResolution,
    platform: &str,
) -> Result<()> {
    let completed = ToolRecord {
        version: TOOL_STATE_VERSION,
        status: ToolStatus::Completed,
        tool,
        requested: requested.into(),
        resolved: resolution.resolved.clone(),
        release: resolution.release.clone(),
        platform: platform.into(),
    };
    if existing == Some(&completed) {
        return Ok(());
    }
    validate_tool_record(&completed)?;
    state.validate_lock_entry(lock)?;
    state.publish(&serde_json::to_vec(&completed).context("serialize toolchain managed record")?)
}

fn validate_tool_record(record: &ToolRecord) -> Result<()> {
    if record.version != TOOL_STATE_VERSION {
        bail!(
            "unsupported toolchain managed record version {}",
            record.version
        );
    }
    match record.tool {
        ToolKind::Rust => validate_rust_record(record),
        ToolKind::Go => {
            validate_canonical_architecture(&record.platform)?;
            if record.requested != "latest" {
                validate_numeric_version(&record.requested, 2, 3, "Go record")?;
            }
            validate_numeric_version(&record.resolved, 3, 3, "Go resolved record")?;
            if record.release != record.resolved
                || record.requested != "latest"
                    && !version_matches(&record.resolved, &record.requested)
            {
                bail!("Go managed record does not match its declaration");
            }
            Ok(())
        }
        ToolKind::Node => {
            validate_canonical_architecture(&record.platform)?;
            if record.requested != "lts" && record.requested != "latest" {
                validate_numeric_version(&record.requested, 1, 3, "Node record")?;
            }
            let numeric = record
                .resolved
                .strip_prefix('v')
                .context("Node managed record resolved version must start with v")?;
            validate_numeric_version(numeric, 3, 3, "Node resolved record")?;
            if record.release != record.resolved
                || record.requested != "lts"
                    && record.requested != "latest"
                    && !version_matches(numeric, &record.requested)
            {
                bail!("Node managed record does not match its declaration");
            }
            Ok(())
        }
        ToolKind::Python => {
            validate_canonical_architecture(&record.platform)?;
            validate_numeric_version(&record.requested, 2, 3, "Python record")?;
            validate_numeric_version(&record.resolved, 3, 3, "Python resolved record")?;
            if record.release != record.resolved
                || !version_matches(&record.resolved, &record.requested)
            {
                bail!("Python managed record does not match its declaration");
            }
            Ok(())
        }
    }
}

fn validate_rust_record(record: &ToolRecord) -> Result<()> {
    if !canonical_rust_target(&record.platform) || !valid_rust_release(&record.release) {
        bail!("Rust managed record has an invalid target or release");
    }
    match record.requested.as_str() {
        "stable" => {
            if !numeric_release(&record.release) || record.resolved != record.release {
                bail!("Rust stable managed record has a mismatched release");
            }
        }
        "beta" => {
            if !record.release.contains("-beta") || record.resolved != record.release {
                bail!("Rust beta managed record has a mismatched release");
            }
        }
        "nightly" => {
            validate_rust_selector(&RustToolchainSelector::DatedNightly(
                record.resolved.clone(),
            ))?;
            if !record.release.ends_with("-nightly") {
                bail!("Rust nightly managed record has a mismatched release");
            }
        }
        requested if requested.starts_with("nightly-") => {
            validate_rust_selector(&RustToolchainSelector::DatedNightly(requested.into()))?;
            if record.resolved != requested || !record.release.ends_with("-nightly") {
                bail!("dated Rust nightly managed record has a mismatched release");
            }
        }
        requested => {
            validate_numeric_version(requested, 2, 3, "Rust record")?;
            if !numeric_release(&record.release)
                || record.resolved != record.release
                || !version_matches(&record.release, requested)
            {
                bail!("Rust numeric managed record has a mismatched release");
            }
        }
    }
    Ok(())
}

fn validate_canonical_architecture(value: &str) -> Result<()> {
    if Architecture::normalize(value)?.canonical() != value {
        bail!("toolchain managed record architecture is not canonical");
    }
    Ok(())
}

fn canonical_rust_target(value: &str) -> bool {
    [
        Architecture::Amd64,
        Architecture::Arm64,
        Architecture::Arm32,
        Architecture::Riscv64,
    ]
    .iter()
    .any(|architecture| architecture.rust_target() == value)
}

mod resolution;
mod state;

use resolution::*;
use state::*;

fn mutation_name(mode: ToolMutationMode) -> &'static str {
    match mode {
        ToolMutationMode::EnsurePresent => "ensure-present",
        ToolMutationMode::UpdateMoving => "update-moving",
    }
}

fn rust_selector_name(selector: &RustToolchainSelector) -> &str {
    match selector {
        RustToolchainSelector::Stable => "stable",
        RustToolchainSelector::Beta => "beta",
        RustToolchainSelector::Nightly => "nightly",
        RustToolchainSelector::DatedNightly(value) | RustToolchainSelector::Version(value) => value,
    }
}

fn rust_selector_is_moving(selector: &RustToolchainSelector) -> bool {
    matches!(
        selector,
        RustToolchainSelector::Stable
            | RustToolchainSelector::Beta
            | RustToolchainSelector::Nightly
    )
}

fn validate_rust_selector(selector: &RustToolchainSelector) -> Result<()> {
    match selector {
        RustToolchainSelector::Stable
        | RustToolchainSelector::Beta
        | RustToolchainSelector::Nightly => Ok(()),
        RustToolchainSelector::Version(version) => validate_numeric_version(version, 2, 3, "Rust"),
        RustToolchainSelector::DatedNightly(value) => {
            let Some(date) = value.strip_prefix("nightly-") else {
                bail!("invalid dated Rust nightly selector {value:?}");
            };
            let parts = date.split('-').collect::<Vec<_>>();
            if parts.len() != 3
                || parts[0].len() != 4
                || parts[1].len() != 2
                || parts[2].len() != 2
                || parts
                    .iter()
                    .any(|part| !part.bytes().all(|byte| byte.is_ascii_digit()))
            {
                bail!("invalid dated Rust nightly selector {value:?}");
            }
            let year = parts[0].parse::<u16>()?;
            let month = parts[1].parse::<u8>()?;
            let day = parts[2].parse::<u8>()?;
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            let days = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if leap => 29,
                2 => 28,
                _ => 0,
            };
            if year == 0 || day == 0 || day > days {
                bail!("invalid dated Rust nightly selector {value:?}");
            }
            Ok(())
        }
    }
}

fn node_selector_name(selector: &NodeToolchainSelector) -> &str {
    match selector {
        NodeToolchainSelector::Lts => "lts",
        NodeToolchainSelector::Latest => "latest",
        NodeToolchainSelector::Version(version) => version,
    }
}

fn node_alias(selector: &NodeToolchainSelector) -> String {
    match selector {
        NodeToolchainSelector::Lts => "cozydot-lts".into(),
        NodeToolchainSelector::Latest => "cozydot-latest".into(),
        NodeToolchainSelector::Version(version) => {
            format!("cozydot-v{}", version.replace('.', "_"))
        }
    }
}

fn valid_rust_release(value: &str) -> bool {
    let numeric = value
        .strip_suffix("-nightly")
        .or_else(|| value.split_once("-beta").map(|parts| parts.0))
        .unwrap_or(value);
    numeric_version(numeric, 3, 3)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

fn valid_rust_host(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn numeric_release(value: &str) -> bool {
    numeric_version(value, 3, 3)
}

fn version_matches(actual: &str, requested: &str) -> bool {
    actual == requested
        || actual
            .strip_prefix(requested)
            .is_some_and(|rest| rest.starts_with('.'))
}

fn validate_numeric_version(
    value: &str,
    min_parts: usize,
    max_parts: usize,
    tool: &str,
) -> Result<()> {
    if !numeric_version(value, min_parts, max_parts) {
        bail!(
            "invalid {tool} version {value:?}; expected {min_parts} to {max_parts} numeric components"
        );
    }
    Ok(())
}

fn numeric_version(value: &str, min_parts: usize, max_parts: usize) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    (min_parts..=max_parts).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        })
}

fn single_line<'a>(output: &'a [u8], command: &str) -> Result<&'a str> {
    let output = std::str::from_utf8(output)
        .with_context(|| format!("{command} returned non-UTF-8 state"))?;
    let output = output.strip_suffix('\n').unwrap_or(output);
    if output.is_empty() || output.contains(['\n', '\r']) {
        bail!("{command} returned malformed multiline state");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_tool_state_parsers_accept_expected_cli_forms() {
        assert_eq!(
            parse_rust_state(
                b"rustc 1.85.1 (abc 2025-01-01)\nbinary: rustc\ncommit-hash: abc\ncommit-date: 2025-01-01\nhost: aarch64-unknown-linux-gnu\nrelease: 1.85.1\nLLVM version: 19.1.7\n"
            )
            .unwrap(),
            RustState {
                release: "1.85.1".into(),
                host: "aarch64-unknown-linux-gnu".into(),
            }
        );
        assert_eq!(
            parse_go_state(b"go version go1.26.1 linux/arm64\n").unwrap(),
            GoState {
                version: "1.26.1".into(),
                architecture: "arm64".into(),
            }
        );
        assert_eq!(parse_node_version(b"v24.4.1\n").unwrap(), "v24.4.1");
        assert_eq!(
            parse_remote_node_version(b"v24.4.1 (Krypton)\n").unwrap(),
            "v24.4.1"
        );
    }

    #[test]
    fn tool_state_parsers_reject_ambiguous_or_malformed_output() {
        for output in [
            b"go version devel go1.27 linux/amd64\n".as_slice(),
            b"go version go1.26.1 linux/mips64\n".as_slice(),
            b"go version go01.26.1 linux/amd64\n".as_slice(),
            b"go version go1.26.1 linux/amd64\nextra\n".as_slice(),
        ] {
            assert!(parse_go_state(output).is_err());
        }
        for output in [
            b"24.4.1\n".as_slice(),
            b"v24.4\n".as_slice(),
            b"v24.04.1\n".as_slice(),
            b"v24.4.1\nextra\n".as_slice(),
        ] {
            assert!(parse_node_version(output).is_err());
        }
        assert!(parse_rust_state(b"host: x86_64-unknown-linux-gnu\n").is_err());
        assert!(parse_rust_state(
            b"host: x86_64-unknown-linux-gnu\nrelease: 1.85.0\nrelease: 1.86.0\n"
        )
        .is_err());
    }

    #[test]
    fn archive_paths_and_operation_inputs_are_validated() {
        validate_go_archive_listing(b"go/README.md\ngo/bin/go\n").unwrap();
        for listing in [
            b"bin/go\n".as_slice(),
            b"go/../etc/passwd\ngo/bin/go\n".as_slice(),
            b"go/README.md\n".as_slice(),
        ] {
            assert!(validate_go_archive_listing(listing).is_err());
        }
        assert!(NodeToolchainOperation::new(
            NodeToolchainSelector::Version("22; touch /tmp/pwn".into()),
            Architecture::Amd64,
            ToolMutationMode::EnsurePresent,
        )
        .is_err());
        assert!(RustToolchainOperation::new(
            RustToolchainSelector::DatedNightly("nightly-2025-02-29".into()),
            Architecture::Amd64,
            ToolMutationMode::EnsurePresent,
        )
        .is_err());
    }

    #[test]
    fn partial_versions_match_only_component_boundaries() {
        assert!(version_matches("1.26.1", "1.26"));
        assert!(!version_matches("1.260.1", "1.26"));
        assert!(version_matches("22.14.0", "22"));
    }

    #[test]
    fn rust_manifest_resolution_requires_version_date_and_native_availability() {
        let manifest = r#"
manifest-version = "2"
date = "2026-01-01"

[pkg.rust]
version = "1.90.0 (abc 2026-01-01)"

[pkg.rust.target.x86_64-unknown-linux-gnu]
available = true
"#;
        assert_eq!(
            parse_rust_manifest(manifest, "x86_64-unknown-linux-gnu").unwrap(),
            ("2026-01-01".into(), "1.90.0".into())
        );
        assert!(parse_rust_manifest(
            &manifest.replace("available = true", "available = false"),
            "x86_64-unknown-linux-gnu"
        )
        .is_err());
        assert!(parse_rust_manifest(
            &manifest.replace("date = \"2026-01-01\"", "date = \"2026-02-30\""),
            "x86_64-unknown-linux-gnu"
        )
        .is_err());
        assert!(parse_rust_manifest(
            &manifest.replace(
                "version = \"1.90.0 (abc 2026-01-01)\"",
                "version = \"1.90.0 (abc)\"\nversion = \"1.91.0 (def)\""
            ),
            "x86_64-unknown-linux-gnu"
        )
        .is_err());
    }

    #[test]
    fn tool_records_reconstruct_exact_selector_relationships() {
        for record in [
            ToolRecord {
                version: TOOL_STATE_VERSION,
                status: ToolStatus::Completed,
                tool: ToolKind::Rust,
                requested: "1.90".into(),
                resolved: "1.90.1".into(),
                release: "1.90.1".into(),
                platform: Architecture::Amd64.rust_target().into(),
            },
            ToolRecord {
                version: TOOL_STATE_VERSION,
                status: ToolStatus::Completed,
                tool: ToolKind::Go,
                requested: "latest".into(),
                resolved: "1.26.1".into(),
                release: "1.26.1".into(),
                platform: "arm64".into(),
            },
            ToolRecord {
                version: TOOL_STATE_VERSION,
                status: ToolStatus::Pending,
                tool: ToolKind::Node,
                requested: "22".into(),
                resolved: "v22.14.0".into(),
                release: "v22.14.0".into(),
                platform: "amd64".into(),
            },
            ToolRecord {
                version: TOOL_STATE_VERSION,
                status: ToolStatus::Completed,
                tool: ToolKind::Python,
                requested: "3.13".into(),
                resolved: "3.13.7".into(),
                release: "3.13.7".into(),
                platform: "riscv64".into(),
            },
        ] {
            validate_tool_record(&record).unwrap();
        }

        let mut invalid = ToolRecord {
            version: TOOL_STATE_VERSION,
            status: ToolStatus::Completed,
            tool: ToolKind::Node,
            requested: "22".into(),
            resolved: "v23.0.0".into(),
            release: "v23.0.0".into(),
            platform: "amd64".into(),
        };
        assert!(validate_tool_record(&invalid).is_err());
        invalid.requested = "22".into();
        invalid.resolved = "v22.14.0".into();
        invalid.release = "v22.15.0".into();
        assert!(validate_tool_record(&invalid).is_err());
        invalid.release = invalid.resolved.clone();
        invalid.platform = "AMD64".into();
        assert!(validate_tool_record(&invalid).is_err());
        invalid.platform = "amd64".into();
        invalid.version = 2;
        assert!(validate_tool_record(&invalid).is_err());
    }

    #[test]
    fn changed_declarations_invalidate_pending_resolutions() {
        let record = ToolRecord {
            version: TOOL_STATE_VERSION,
            status: ToolStatus::Pending,
            tool: ToolKind::Node,
            requested: "22".into(),
            resolved: "v22.14.0".into(),
            release: "v22.14.0".into(),
            platform: "amd64".into(),
        };
        let resolution =
            select_resolution(Some(&record), ToolKind::Node, "23", "amd64", false, || {
                Ok(ToolResolution {
                    resolved: "v23.1.0".into(),
                    release: "v23.1.0".into(),
                })
            })
            .unwrap();
        assert_eq!(resolution.resolved, "v23.1.0");
    }
}
