use crate::{config::HttpsUrl, platform::Architecture};
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
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoToolchainSelector {
    Latest,
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeToolchainSelector {
    Lts,
    Latest,
    Version(String),
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

pub(crate) fn execute_rust(
    host: &Host,
    selector: &RustToolchainSelector,
    architecture: Architecture,
    mode: ToolMutationMode,
) -> Result<()> {
    let rustup = resolve_managed(host, "CARGO_HOME", ".cargo", "bin/rustup")?
        .context("Rust toolchain operation: rustup is unavailable after bootstrap")?;
    let target = architecture.rust_target();
    let refresh = mode == ToolMutationMode::UpdateMoving && rust_selector_is_moving(selector);
    let toolchain = rust_toolchain_name(selector, target);
    let current = inspect_rust(host, &rustup, &toolchain)?;
    if refresh
        || current
            .as_ref()
            .is_none_or(|state| state.host != target || !rust_release_matches(&state.release, selector))
    {
        host.require("Rust toolchain mutation", &rustup, rust_install_args(&toolchain))?;
    }
    let default = rust_default(host, &rustup)?;
    if default.as_deref() != Some(toolchain.as_str()) {
        host.require("Rust default toolchain mutation", &rustup, ["default", &toolchain])?;
    }
    let state = inspect_rust(host, &rustup, &toolchain)?
        .with_context(|| format!("Rust toolchain mutation did not install requested toolchain {toolchain}"))?;
    if state.host != target || !rust_release_matches(&state.release, selector) {
        bail!("Rust toolchain mutation produced mismatched release or host state");
    }
    if rust_default(host, &rustup)?.as_deref() != Some(toolchain.as_str()) {
        bail!("Rust default toolchain mutation did not select {toolchain}");
    }
    Ok(())
}

pub(crate) fn execute_go(
    host: &Host,
    selector: &GoToolchainSelector,
    architecture: Architecture,
    mode: ToolMutationMode,
) -> Result<()> {
    let expected_arch = architecture.go();

    let current = inspect_go(host, "/usr/local/go/bin/go")?;
    if mode == ToolMutationMode::EnsurePresent {
        if let Some(state) = &current {
            if state.architecture == expected_arch {
                match selector {
                    GoToolchainSelector::Latest => {
                        return Ok(());
                    }
                    GoToolchainSelector::Version(requested) => {
                        if version_matches(&state.version, requested) {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    let requested = match selector {
        GoToolchainSelector::Latest => "latest",
        GoToolchainSelector::Version(version) => version,
    };

    let release = resolve_go_release(host, requested, architecture)?;
    let version = release.resolution.release.clone();

    if current
        .as_ref()
        .is_some_and(|state| state.version == version && state.architecture == expected_arch)
    {
        return Ok(());
    }

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
        ["--list", "--gzip", "--file", &archive.path().to_string_lossy()],
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
    let staged_state =
        inspect_go(host, &staged_program)?.context("Go archive does not contain an executable Go toolchain")?;
    if staged_state.version != version || staged_state.architecture != expected_arch {
        bail!("Go archive toolchain does not match resolved release metadata");
    }
    host.require("Go toolchain publication", "sudo", ["rm", "-rf", "--", "/usr/local/go"])?;
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

pub(crate) fn execute_node(
    host: &Host,
    selector: &NodeToolchainSelector,
    _architecture: Architecture,
    mode: ToolMutationMode,
) -> Result<()> {
    let fnm = resolve_fnm(host)?;

    let alias = node_alias(selector);
    let current = inspect_node(host, &fnm, &alias)?;

    if mode == ToolMutationMode::EnsurePresent {
        if let Some(version) = &current {
            let accepted = match selector {
                NodeToolchainSelector::Latest | NodeToolchainSelector::Lts => true,
                NodeToolchainSelector::Version(requested) => {
                    version_matches(version.trim_start_matches('v'), requested)
                }
            };
            if accepted {
                let default = fnm_default(host, &fnm)?;
                if default.as_deref() != Some(version.as_str()) {
                    host.require("Node default toolchain mutation", &fnm, ["default", version])?;
                }
                if fnm_default(host, &fnm)?.as_deref() != Some(version.as_str()) {
                    bail!("Node default toolchain mutation did not select {}", version);
                }
                return Ok(());
            }
        }
    }

    let resolved_version = resolve_node_version(host, &fnm, selector)?;

    if current.as_deref() != Some(resolved_version.as_str()) {
        host.require(
            "Node toolchain mutation",
            &fnm,
            ["install", &resolved_version, "--progress", "never"],
        )?;
        if current.is_some() {
            host.require("Node toolchain alias replacement", &fnm, ["unalias", &alias])?;
        }
        host.require(
            "Node toolchain alias publication",
            &fnm,
            ["alias", &resolved_version, &alias],
        )?;
    }
    let default = fnm_default(host, &fnm)?;
    if default.as_deref() != Some(resolved_version.as_str()) {
        host.require("Node default toolchain mutation", &fnm, ["default", &resolved_version])?;
    }
    let installed = inspect_node(host, &fnm, &alias)?
        .context("Node toolchain mutation did not publish the managed selector alias")?;
    if installed != resolved_version {
        bail!("Node toolchain mutation produced mismatched version state");
    }
    if fnm_default(host, &fnm)?.as_deref() != Some(resolved_version.as_str()) {
        bail!("Node default toolchain mutation did not select {}", resolved_version);
    }
    Ok(())
}

pub(crate) fn execute_python(host: &Host, version: &str, architecture: Architecture) -> Result<()> {
    let uv = resolve_managed(host, "UV_INSTALL_DIR", ".local/bin", "uv")?
        .context("Python toolchain operation: uv is unavailable after bootstrap")?;

    if let Some(current_version) = inspect_python(host, &uv, version)? {
        if version_matches(&current_version, version) {
            return Ok(());
        }
    }

    let resolution = resolve_python_version(host, &uv, version, architecture)?;

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
    Ok(())
}

mod resolution {
    use super::*;

    pub(super) fn resolve_go_release(host: &Host, requested: &str, architecture: Architecture) -> Result<GoRelease> {
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
        let (version, filename, checksum) = super::super::latest_go(
            std::str::from_utf8(&metadata.stdout).context("Go release metadata is not UTF-8")?,
            requested,
            architecture.go_archive(),
        )?;
        Ok(GoRelease {
            resolution: ToolResolution {
                resolved: version.clone(),
                release: version,
            },
            filename,
            checksum,
        })
    }

    pub(super) fn resolve_python_version(
        host: &Host,
        uv: &str,
        requested: &str,
        architecture: Architecture,
    ) -> Result<ToolResolution> {
        let output = host.require(
            "Python release availability",
            uv,
            [
                "python",
                "list",
                requested,
                "--all-versions",
                "--only-downloads",
                "--output-format",
                "json",
                "--no-config",
                "--no-progress",
            ],
        )?;
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).context("uv returned malformed Python release JSON")?;
        let entries = value.as_array().context("uv Python release state must be an array")?;
        let expected_arch = match architecture {
            Architecture::Amd64 => "x86_64",
            Architecture::Arm64 => "aarch64",
            Architecture::Arm32 => "armv7",
        };
        let mut matches = Vec::new();
        for (index, value) in entries.iter().enumerate() {
            let entry = value
                .as_object()
                .with_context(|| format!("uv Python release {index} must be an object"))?;
            let string = |field: &str| -> Result<&str> {
                entry
                    .get(field)
                    .with_context(|| format!("uv Python release {index} is missing {field}"))?
                    .as_str()
                    .with_context(|| format!("uv Python release {index} {field} must be a string"))
            };
            let version = string("version")?;
            validate_numeric_version(version, 3, 3, "uv Python release")?;
            let url = HttpsUrl::parse(string("url")?)?;
            if url.as_str() != string("url")? {
                bail!("uv Python release URL is not canonical");
            }
            if string("implementation")? == "cpython"
                && string("os")? == "linux"
                && string("variant")? == "default"
                && string("arch")? == expected_arch
                && string("libc")? == "gnu"
                && version_matches(version, requested)
            {
                matches.push(version.to_owned());
            }
        }
        matches.sort_by_key(|version| numeric_version_key(version));
        matches.dedup();
        let resolved = matches
            .pop()
            .with_context(|| format!("uv has no managed Python release matching {requested:?}"))?;
        Ok(ToolResolution {
            release: resolved.clone(),
            resolved,
        })
    }

    fn numeric_version_key(value: &str) -> (u64, u64, u64) {
        let mut parts = value.split('.').map(|part| part.parse::<u64>().unwrap_or_default());
        (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        )
    }
}

mod state {
    use super::*;

    pub(super) fn inspect_rust(host: &Host, rustup: &str, toolchain: &str) -> Result<Option<RustState>> {
        let output = host.run(rustup, ["run", toolchain, "rustc", "--version", "--verbose"])?;
        if !output.status.success() {
            return Ok(None);
        }
        parse_rust_state(&output.stdout).map(Some)
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct RustState {
        pub(super) release: String,
        pub(super) host: String,
    }

    pub(super) fn parse_rust_state(output: &[u8]) -> Result<RustState> {
        let output = std::str::from_utf8(output).context("rustc returned non-UTF-8 state")?;
        let mut release = None;
        let mut host = None;
        for line in output.lines() {
            if let Some(value) = line.strip_prefix("release: ") {
                if release.replace(value.to_owned()).is_some() || !numeric_release(value) {
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

    pub(super) fn rust_default(host: &Host, rustup: &str) -> Result<Option<String>> {
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
    pub(super) struct GoState {
        pub(super) version: String,
        pub(super) architecture: String,
    }

    pub(super) fn inspect_go(host: &Host, program: &str) -> Result<Option<GoState>> {
        if program.starts_with('/') && !executable_file(Path::new(program)) {
            return Ok(None);
        }
        let output = match host.run(program, ["version"]) {
            Ok(output) => output,
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                    error.kind() == std::io::ErrorKind::NotFound || error.kind() == std::io::ErrorKind::PermissionDenied
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

    pub(super) fn parse_go_state(output: &[u8]) -> Result<GoState> {
        let output = single_line(output, "go version")?;
        let fields = output.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 4
            || fields[0] != "go"
            || fields[1] != "version"
            || fields[3] != "linux/amd64" && fields[3] != "linux/arm64" && fields[3] != "linux/arm"
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

    pub(super) fn validate_go_archive_listing(output: &[u8]) -> Result<()> {
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

    pub(super) fn resolve_fnm(host: &Host) -> Result<String> {
        let data_home = host
            .value("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".local/share"));
        if !data_home.is_absolute() {
            bail!("managed FNM data directory must be absolute");
        }
        let managed = data_home.join("fnm/fnm");
        if executable_file(&managed) {
            return path_program(&managed, "managed fnm executable");
        }
        bail!("Node toolchain operation: fnm is unavailable after bootstrap")
    }

    pub(super) fn inspect_node(host: &Host, fnm: &str, selector: &str) -> Result<Option<String>> {
        let output = host.run(fnm, ["exec", "--using", selector, "--", "node", "--version"])?;
        if !output.status.success() {
            return Ok(None);
        }
        parse_node_version(&output.stdout).map(Some)
    }

    pub(super) fn resolve_node_version(host: &Host, fnm: &str, selector: &NodeToolchainSelector) -> Result<String> {
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

    pub(super) fn parse_remote_node_version(output: &[u8]) -> Result<String> {
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

    pub(super) fn parse_node_version(output: &[u8]) -> Result<String> {
        let version = single_line(output, "node --version")?;
        let numeric = version
            .strip_prefix('v')
            .filter(|version| numeric_version(version, 3, 3))
            .context("node returned malformed version state")?;
        Ok(format!("v{numeric}"))
    }

    pub(super) fn fnm_default(host: &Host, fnm: &str) -> Result<Option<String>> {
        let output = host.run(fnm, ["default"])?;
        if !output.status.success() {
            return Ok(None);
        }
        if output.stdout.is_empty() || output.stdout == b"none\n" || output.stdout == b"none" {
            return Ok(None);
        }
        parse_node_version(&output.stdout).map(Some)
    }

    pub(super) fn inspect_python(host: &Host, uv: &str, request: &str) -> Result<Option<String>> {
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

    pub(super) fn resolve_managed(
        host: &Host,
        directory_variable: &str,
        default_directory: &str,
        relative_program: &str,
    ) -> Result<Option<String>> {
        let base = host
            .value(directory_variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(default_directory));
        if !base.is_absolute() {
            bail!("managed tool directory must be absolute");
        }
        let managed = base.join(relative_program);
        if executable_file(&managed) {
            return path_program(&managed, "managed tool executable").map(Some);
        }
        Ok(None)
    }

    fn executable_file(path: &Path) -> bool {
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0)
    }

    pub(super) fn path_program(path: &Path, description: &str) -> Result<String> {
        path.to_str()
            .map(str::to_owned)
            .with_context(|| format!("{description} path is not UTF-8: {}", path.display()))
    }
}

use resolution::*;
use state::*;

fn rust_selector_name(selector: &RustToolchainSelector) -> &str {
    match selector {
        RustToolchainSelector::Stable => "stable",
        RustToolchainSelector::Version(value) => value,
    }
}

fn rust_toolchain_name(selector: &RustToolchainSelector, target: &str) -> String {
    format!("{}-{target}", rust_selector_name(selector))
}

fn rust_install_args(toolchain: &str) -> [&str; 6] {
    [
        "toolchain",
        "install",
        toolchain,
        "--profile",
        "minimal",
        "--no-self-update",
    ]
}

fn rust_selector_is_moving(selector: &RustToolchainSelector) -> bool {
    matches!(selector, RustToolchainSelector::Stable)
        || matches!(selector, RustToolchainSelector::Version(value) if value.split('.').count() == 2)
}

fn rust_release_matches(release: &str, selector: &RustToolchainSelector) -> bool {
    numeric_release(release)
        && match selector {
            RustToolchainSelector::Stable => true,
            RustToolchainSelector::Version(requested) => version_matches(release, requested),
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
    actual == requested || actual.strip_prefix(requested).is_some_and(|rest| rest.starts_with('.'))
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

fn validate_numeric_version(value: &str, min_parts: usize, max_parts: usize, label: &str) -> Result<()> {
    if !numeric_version(value, min_parts, max_parts) {
        bail!("invalid {label} version {value:?}; expected {min_parts} to {max_parts} numeric components");
    }
    Ok(())
}

fn single_line<'a>(output: &'a [u8], command: &str) -> Result<&'a str> {
    let output = std::str::from_utf8(output).with_context(|| format!("{command} returned non-UTF-8 state"))?;
    let output = output.strip_suffix('\n').unwrap_or(output);
    if output.is_empty() || output.contains(['\n', '\r']) {
        bail!("{command} returned malformed multiline state");
    }
    Ok(output)
}
