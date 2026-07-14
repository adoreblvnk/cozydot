use crate::{json_helpers, platform::Architecture};
use anyhow::{bail, Context, Result};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::{Host, TempDir, TempPath};

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
    mode: ToolMutationMode,
}

impl NodeToolchainOperation {
    pub fn new(selector: NodeToolchainSelector, mode: ToolMutationMode) -> Result<Self> {
        if let NodeToolchainSelector::Version(version) = &selector {
            validate_numeric_version(version, 1, 3, "Node")?;
        }
        Ok(Self { selector, mode })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "node-toolchain".into(),
            mutation_name(self.mode).into(),
            node_selector_name(&self.selector).into(),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PythonToolchainOperation {
    version: String,
}

impl PythonToolchainOperation {
    pub fn new(version: impl Into<String>) -> Result<Self> {
        let version = version.into();
        validate_numeric_version(&version, 2, 3, "Python")?;
        Ok(Self { version })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec!["python-toolchain".into(), self.version.clone()]
    }
}

pub(crate) fn execute_rust(host: &Host<'_>, operation: &RustToolchainOperation) -> Result<()> {
    validate_rust_selector(&operation.selector).context("validate Rust toolchain operation")?;
    let rustup = resolve_managed_or_path(host, "CARGO_HOME", ".cargo", "bin/rustup", "rustup")?
        .context("Rust toolchain operation: rustup is unavailable after bootstrap")?;
    let target = operation.architecture.rust_target();
    let toolchain = format!("{}-{target}", rust_selector_name(&operation.selector));
    let current = inspect_rust(host, &rustup, &toolchain)?;
    let refresh = operation.mode == ToolMutationMode::UpdateMoving
        && rust_selector_is_moving(&operation.selector);
    if current
        .as_ref()
        .is_none_or(|state| !rust_state_matches(state, &operation.selector, target))
        || refresh
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
    if !rust_state_matches(&state, &operation.selector, target) {
        bail!("Rust toolchain mutation produced mismatched release or host state");
    }
    if rust_default(host, &rustup)?.as_deref() != Some(toolchain.as_str()) {
        bail!("Rust default toolchain mutation did not select {toolchain}");
    }
    Ok(())
}

pub(crate) fn execute_go(host: &Host<'_>, operation: &GoToolchainOperation) -> Result<()> {
    if let GoToolchainSelector::Version(version) = &operation.selector {
        validate_numeric_version(version, 2, 3, "Go")?;
    }
    let expected_arch = operation.architecture.go();
    let current = inspect_go(host, "/usr/local/go/bin/go")?;
    let valid_for_ensure = current.as_ref().is_some_and(|state| {
        state.architecture == expected_arch
            && match &operation.selector {
                GoToolchainSelector::Latest => true,
                GoToolchainSelector::Version(version) => version_matches(&state.version, version),
            }
    });
    let refresh = operation.mode == ToolMutationMode::UpdateMoving
        && operation.selector == GoToolchainSelector::Latest;
    if valid_for_ensure && !refresh {
        return Ok(());
    }

    let metadata = host.require(
        "Go release resolution",
        "curl",
        [
            "--proto",
            "=https",
            "--location",
            "--fail",
            "--silent",
            "--show-error",
            "--retry",
            "3",
            "--retry-all-errors",
            "--",
            "https://go.dev/dl/?mode=json&include=all",
        ],
    )?;
    let requested = match &operation.selector {
        GoToolchainSelector::Latest => "latest",
        GoToolchainSelector::Version(version) => version,
    };
    let (version, filename, checksum) = json_helpers::latest_go(
        std::str::from_utf8(&metadata.stdout).context("Go release metadata is not UTF-8")?,
        requested,
        operation.architecture.go_archive(),
    )?;
    if current
        .as_ref()
        .is_some_and(|state| state.version == version && state.architecture == expected_arch)
    {
        return Ok(());
    }
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
    Ok(())
}

pub(crate) fn execute_node(host: &Host<'_>, operation: &NodeToolchainOperation) -> Result<()> {
    if let NodeToolchainSelector::Version(version) = &operation.selector {
        validate_numeric_version(version, 1, 3, "Node")?;
    }
    let fnm = resolve_fnm(host)?;
    let alias = node_alias(&operation.selector);
    let current = inspect_node(host, &fnm, &alias)?;
    let refresh = operation.mode == ToolMutationMode::UpdateMoving
        && !matches!(operation.selector, NodeToolchainSelector::Version(_));
    let selected = if current
        .as_ref()
        .is_some_and(|version| node_selector_matches(version, &operation.selector))
        && !refresh
    {
        current.expect("checked as present")
    } else {
        let resolved = resolve_node_version(host, &fnm, &operation.selector)?;
        if current.as_deref() != Some(resolved.as_str()) {
            host.require(
                "Node toolchain mutation",
                &fnm,
                ["install", &resolved, "--progress", "never"],
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
                ["alias", &resolved, &alias],
            )?;
        }
        resolved
    };
    let default = fnm_default(host, &fnm)?;
    if default.as_deref() != Some(selected.as_str()) {
        host.require(
            "Node default toolchain mutation",
            &fnm,
            ["default", &selected],
        )?;
    }
    let installed = inspect_node(host, &fnm, &alias)?
        .context("Node toolchain mutation did not publish the managed selector alias")?;
    if installed != selected || !node_selector_matches(&installed, &operation.selector) {
        bail!("Node toolchain mutation produced mismatched version state");
    }
    if fnm_default(host, &fnm)?.as_deref() != Some(selected.as_str()) {
        bail!("Node default toolchain mutation did not select {selected}");
    }
    Ok(())
}

pub(crate) fn execute_python(host: &Host<'_>, operation: &PythonToolchainOperation) -> Result<()> {
    validate_numeric_version(&operation.version, 2, 3, "Python")?;
    let uv = resolve_managed_or_path(host, "UV_INSTALL_DIR", ".local/bin", "uv", "uv")?
        .context("Python toolchain operation: uv is unavailable after bootstrap")?;
    let current = inspect_python(host, &uv, &operation.version)?;
    if current
        .as_ref()
        .is_none_or(|version| !version_matches(version, &operation.version))
    {
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
                &operation.version,
            ],
        )?;
    }
    let installed = inspect_python(host, &uv, &operation.version)?
        .context("Python toolchain mutation did not install a managed interpreter")?;
    if !version_matches(&installed, &operation.version) {
        bail!("Python toolchain mutation installed mismatched version {installed}");
    }
    Ok(())
}

fn inspect_rust(host: &Host<'_>, rustup: &str, toolchain: &str) -> Result<Option<RustState>> {
    let output = host.run(
        rustup,
        ["run", toolchain, "rustc", "--version", "--verbose"],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    parse_rust_state(&output.stdout).map(Some)
}

#[derive(Debug, PartialEq, Eq)]
struct RustState {
    release: String,
    host: String,
}

fn parse_rust_state(output: &[u8]) -> Result<RustState> {
    let output = std::str::from_utf8(output).context("rustc returned non-UTF-8 state")?;
    let mut release = None;
    let mut host = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("release: ") {
            if release.replace(value.to_owned()).is_some() || !valid_rust_release(value) {
                bail!("rustc returned malformed release state");
            }
        } else if let Some(value) = line.strip_prefix("host: ") {
            if host.replace(value.to_owned()).is_some() || !valid_rust_host(value) {
                bail!("rustc returned malformed host state");
            }
        }
    }
    Ok(RustState {
        release: release.context("rustc state is missing release")?,
        host: host.context("rustc state is missing host")?,
    })
}

fn rust_state_matches(state: &RustState, selector: &RustToolchainSelector, target: &str) -> bool {
    state.host == target
        && match selector {
            RustToolchainSelector::Stable => numeric_release(&state.release),
            RustToolchainSelector::Beta => state.release.contains("-beta"),
            RustToolchainSelector::Nightly | RustToolchainSelector::DatedNightly(_) => {
                state.release.ends_with("-nightly")
            }
            RustToolchainSelector::Version(version) => version_matches(&state.release, version),
        }
}

fn rust_default(host: &Host<'_>, rustup: &str) -> Result<Option<String>> {
    let output = host.run(rustup, ["default"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let output = single_line(&output.stdout, "rustup default")?;
    if output == "no default toolchain configured" {
        return Ok(None);
    }
    let Some(toolchain) = output.strip_suffix(" (default)") else {
        bail!("rustup returned malformed default toolchain state");
    };
    if toolchain.is_empty() || toolchain.chars().any(char::is_whitespace) {
        bail!("rustup returned malformed default toolchain state");
    }
    Ok(Some(toolchain.to_owned()))
}

#[derive(Debug, PartialEq, Eq)]
struct GoState {
    version: String,
    architecture: String,
}

fn inspect_go(host: &Host<'_>, program: &str) -> Result<Option<GoState>> {
    if program.starts_with('/') && !executable_file(Path::new(program)) {
        return Ok(None);
    }
    let output = match host.run(program, ["version"]) {
        Ok(output) => output,
        Err(error)
            if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::PermissionDenied
            }) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    if !output.status.success() {
        return Ok(None);
    }
    parse_go_state(&output.stdout).map(Some)
}

fn parse_go_state(output: &[u8]) -> Result<GoState> {
    let output = single_line(output, "go version")?;
    let fields = output.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 4
        || fields[0] != "go"
        || fields[1] != "version"
        || fields[3] != "linux/amd64"
            && fields[3] != "linux/arm64"
            && fields[3] != "linux/arm"
            && fields[3] != "linux/riscv64"
    {
        bail!("go returned malformed version state");
    }
    let version = fields[2]
        .strip_prefix("go")
        .filter(|version| numeric_version(version, 2, 3))
        .context("go returned malformed version state")?;
    Ok(GoState {
        version: version.to_owned(),
        architecture: fields[3].trim_start_matches("linux/").to_owned(),
    })
}

fn validate_go_archive_listing(output: &[u8]) -> Result<()> {
    let output = std::str::from_utf8(output).context("Go archive listing is not UTF-8")?;
    let mut saw_binary = false;
    for entry in output.lines() {
        if entry.is_empty()
            || !entry.starts_with("go/")
            || entry.split('/').any(|component| component == "..")
            || entry.chars().any(char::is_control)
        {
            bail!("Go archive contains an unsafe path");
        }
        saw_binary |= entry == "go/bin/go";
    }
    if !saw_binary {
        bail!("Go archive listing does not contain go/bin/go");
    }
    Ok(())
}

fn resolve_fnm(host: &Host<'_>) -> Result<String> {
    if executable_on_path(host, "fnm").is_some() {
        return Ok("fnm".into());
    }
    let data_home = host
        .value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"));
    let managed = data_home.join("fnm/fnm");
    if executable_file(&managed) {
        return path_program(&managed, "managed fnm executable");
    }
    bail!("Node toolchain operation: fnm is unavailable after bootstrap")
}

fn inspect_node(host: &Host<'_>, fnm: &str, selector: &str) -> Result<Option<String>> {
    let output = host.run(
        fnm,
        ["exec", "--using", selector, "--", "node", "--version"],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    parse_node_version(&output.stdout).map(Some)
}

fn resolve_node_version(
    host: &Host<'_>,
    fnm: &str,
    selector: &NodeToolchainSelector,
) -> Result<String> {
    if let NodeToolchainSelector::Version(version) = selector {
        if numeric_version(version, 3, 3) {
            return Ok(format!("v{version}"));
        }
    }
    let mut args = vec!["list-remote", "--latest"];
    match selector {
        NodeToolchainSelector::Lts => args.push("--lts"),
        NodeToolchainSelector::Latest => {}
        NodeToolchainSelector::Version(version) => {
            args.extend(["--filter", version]);
        }
    }
    let output = host.require("Node release resolution", fnm, args)?;
    parse_remote_node_version(&output.stdout)
}

fn parse_remote_node_version(output: &[u8]) -> Result<String> {
    let output = single_line(output, "fnm list-remote")?;
    let version = output
        .split_whitespace()
        .next()
        .context("fnm list-remote returned empty state")?;
    if !output
        .chars()
        .all(|character| !character.is_control() || character == '\t')
    {
        bail!("fnm list-remote returned malformed state");
    }
    parse_node_version(format!("{version}\n").as_bytes())
}

fn parse_node_version(output: &[u8]) -> Result<String> {
    let version = single_line(output, "node --version")?;
    let numeric = version
        .strip_prefix('v')
        .filter(|version| numeric_version(version, 3, 3))
        .context("node returned malformed version state")?;
    Ok(format!("v{numeric}"))
}

fn fnm_default(host: &Host<'_>, fnm: &str) -> Result<Option<String>> {
    let output = host.run(fnm, ["default"])?;
    if !output.status.success() {
        return Ok(None);
    }
    if output.stdout.is_empty() || output.stdout == b"none\n" || output.stdout == b"none" {
        return Ok(None);
    }
    parse_node_version(&output.stdout).map(Some)
}

fn inspect_python(host: &Host<'_>, uv: &str, request: &str) -> Result<Option<String>> {
    let output = host.run(
        uv,
        [
            "python",
            "find",
            "--no-project",
            "--managed-python",
            "--show-version",
            request,
        ],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let version = single_line(&output.stdout, "uv python find")?;
    if !numeric_version(version, 3, 3) {
        bail!("uv returned malformed managed Python version state");
    }
    Ok(Some(version.to_owned()))
}

fn resolve_managed_or_path(
    host: &Host<'_>,
    directory_variable: &str,
    default_directory: &str,
    relative_program: &str,
    path_program_name: &str,
) -> Result<Option<String>> {
    let base = host
        .value(directory_variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(default_directory));
    let managed = base.join(relative_program);
    if executable_file(&managed) {
        return path_program(&managed, "managed tool executable").map(Some);
    }
    Ok(executable_on_path(host, path_program_name).map(|_| path_program_name.to_owned()))
}

fn executable_on_path(host: &Host<'_>, name: &str) -> Option<PathBuf> {
    host.value("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|path| executable_file(path))
    })
}

fn executable_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0
    })
}

fn path_program(path: &Path, description: &str) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .with_context(|| format!("{description} path is not UTF-8: {}", path.display()))
}

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

fn node_selector_matches(version: &str, selector: &NodeToolchainSelector) -> bool {
    let Some(version) = version.strip_prefix('v') else {
        return false;
    };
    match selector {
        NodeToolchainSelector::Lts | NodeToolchainSelector::Latest => {
            numeric_version(version, 3, 3)
        }
        NodeToolchainSelector::Version(requested) => version_matches(version, requested),
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
        assert!(node_selector_matches(
            "v22.14.0",
            &NodeToolchainSelector::Version("22".into())
        ));
    }
}
