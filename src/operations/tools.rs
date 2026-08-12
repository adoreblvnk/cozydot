use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use std::path::Path;

use super::{Host, TempPath, ToolchainMode, path_program, real_executable_file};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoToolchainSelector {
    Latest,
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GoRelease {
    version: String,
    filename: String,
}

pub(crate) fn execute_rust(host: &Host, selector: Option<&str>, mode: ToolchainMode) -> Result<()> {
    let rustup = resolve_managed(host, "CARGO_HOME", ".cargo", "bin/rustup")?
        .context("Rust toolchain operation: rustup is unavailable after bootstrap")?;
    match mode {
        ToolchainMode::EnsurePresent => {
            let selector = selector.context("Rust apply operation requires a configured selector")?;
            host.require("Rust toolchain mutation", &rustup, rust_install_args(selector))?;
            host.require("Rust default toolchain mutation", &rustup, ["default", "--", selector])?;
        }
        ToolchainMode::ConvergeLatest => {
            let mut args = vec!["update", "--no-self-update"];
            if let Some(selector) = selector {
                args.extend(["--", selector]);
            }
            host.require("Rust toolchain update", &rustup, args)?;
            if let Some(selector) = selector {
                host.require("Rust default toolchain mutation", &rustup, ["default", "--", selector])?;
            }
        }
    }
    Ok(())
}

pub(crate) fn execute_go(
    host: &Host,
    selector: &GoToolchainSelector,
    architecture: Architecture,
    mode: ToolchainMode,
) -> Result<()> {
    let expected_arch = architecture.go();
    let current = inspect_go(host, "/usr/local/go/bin/go")?;
    let requested = match selector {
        GoToolchainSelector::Latest => "latest",
        GoToolchainSelector::Version(version) => version,
    };
    if current.as_ref().is_some_and(|state| {
        state.architecture == expected_arch
            && match mode {
                ToolchainMode::EnsurePresent => go_selector_matches(requested, &state.version),
                ToolchainMode::ConvergeLatest => numeric_version(requested, 3, 3) && state.version == requested,
            }
    }) {
        return Ok(());
    }

    let GoRelease { version, filename } = resolve_go_release(host, requested, architecture)?;

    if current.as_ref().is_some_and(|state| state.version == version && state.architecture == expected_arch) {
        return Ok(());
    }

    let archive = TempPath::new_with_suffix(host, "go", ".tar.gz")?;
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
    host.require("Go toolchain publication", "sudo", ["rm", "-rf", "--", "/usr/local/go"])?;
    host.require(
        "Go toolchain publication",
        "sudo",
        ["tar", "-xzf", archive.path().to_str().context("Go archive path is not UTF-8")?, "-C", "/usr/local"],
    )?;
    Ok(())
}

pub(crate) fn execute_node(host: &Host, selector: &str, mode: ToolchainMode) -> Result<()> {
    let fnm = resolve_fnm(host)?;
    let default = node_alias(selector);
    let present = mode == ToolchainMode::EnsurePresent
        && host.run(&fnm, ["exec", "--using", default, "--", "node", "--version"])?.status.success();
    if !present {
        if selector == "lts" {
            host.require("Node toolchain mutation", &fnm, ["install", "--progress", "never", "--lts"])?;
        } else {
            host.require("Node toolchain mutation", &fnm, ["install", "--progress", "never", "--", selector])?;
        }
    }
    host.require("Node default toolchain mutation", &fnm, ["default", "--", default])?;
    Ok(())
}

pub(crate) fn execute_python(host: &Host, version: &str, mode: ToolchainMode) -> Result<()> {
    let uv = resolve_managed(host, "UV_INSTALL_DIR", ".local/bin", "uv")?
        .context("Python toolchain operation: uv is unavailable after bootstrap")?;

    if mode == ToolchainMode::EnsurePresent
        && host
            .run(
                &uv,
                [
                    "python",
                    "find",
                    "--no-config",
                    "--managed-python",
                    "--no-python-downloads",
                    "--show-version",
                    "--",
                    version,
                ],
            )?
            .status
            .success()
    {
        return Ok(());
    }
    let mut args = vec!["python", "install", "--no-config", "--managed-python", "--no-progress"];
    if mode == ToolchainMode::ConvergeLatest {
        args.push("--upgrade");
    }
    args.extend(["--default", "--", version]);
    host.require("Python toolchain mutation", &uv, args)?;
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
        let target_os = if cfg!(target_os = "macos") { "darwin" } else { "linux" };
        let (version, filename) = super::super::latest_go(
            std::str::from_utf8(&metadata.stdout).context("Go release metadata is not UTF-8")?,
            requested,
            architecture.go_archive(),
            target_os,
        )?;
        Ok(GoRelease { version, filename })
    }
}

mod state {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct GoState {
        pub(super) version: String,
        pub(super) architecture: String,
    }

    pub(super) fn inspect_go(host: &Host, program: &str) -> Result<Option<GoState>> {
        if program.starts_with('/') && !real_executable_file(Path::new(program)) {
            return Ok(None);
        }
        let output = match host.run(program, ["version"]) {
            Ok(output) => output,
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                    error.kind() == std::io::ErrorKind::NotFound || error.kind() == std::io::ErrorKind::PermissionDenied
                }) =>
            {
                return Ok(None);
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
        let target_os = if cfg!(target_os = "macos") { "darwin" } else { "linux" };
        if fields.len() != 4
            || fields[0] != "go"
            || fields[1] != "version"
            || !matches!(fields[3], "linux/amd64" | "linux/arm64" | "linux/arm" | "darwin/amd64" | "darwin/arm64")
            || !fields[3].starts_with(target_os)
        {
            bail!("go returned malformed version state");
        }
        let version = fields[2]
            .strip_prefix("go")
            .filter(|version| numeric_version(version, 2, 3))
            .context("go returned malformed version state")?;
        Ok(GoState {
            version: version.to_owned(),
            architecture: fields[3].trim_start_matches(&format!("{target_os}/")).to_owned(),
        })
    }

    pub(super) fn resolve_fnm(host: &Host) -> Result<String> {
        let data_home =
            host.managed_dir("XDG_DATA_HOME", ".local/share", "managed FNM data directory must be absolute")?;
        let managed = data_home.join("fnm/fnm");
        if real_executable_file(&managed) {
            return path_program(&managed, "managed fnm executable path");
        }
        bail!("Node toolchain operation: fnm is unavailable after bootstrap")
    }

    pub(super) fn resolve_managed(
        host: &Host,
        directory_variable: &str,
        default_directory: &str,
        relative_program: &str,
    ) -> Result<Option<String>> {
        let base =
            host.managed_dir(directory_variable, default_directory, "managed tool directory must be absolute")?;
        let managed = base.join(relative_program);
        if real_executable_file(&managed) {
            return path_program(&managed, "managed tool executable path").map(Some);
        }
        Ok(None)
    }
}

use resolution::*;
use state::*;

fn rust_install_args(toolchain: &str) -> [&str; 8] {
    ["toolchain", "install", "--profile", "minimal", "--no-self-update", "--no-update", "--", toolchain]
}

fn node_alias(selector: &str) -> &str {
    match selector {
        "lts" => "lts-latest",
        value => value,
    }
}

fn go_selector_matches(selector: &str, version: &str) -> bool {
    if selector == "latest" {
        return numeric_version(version, 2, 3);
    }
    if !numeric_version(selector, 1, 3) || !numeric_version(version, 2, 3) {
        return false;
    }
    let requested = selector.split('.').collect::<Vec<_>>();
    let installed = version.split('.').collect::<Vec<_>>();
    requested.len() <= installed.len() && requested.iter().zip(installed).all(|(left, right)| *left == right)
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
    let output = std::str::from_utf8(output).with_context(|| format!("{command} returned non-UTF-8 state"))?;
    let output = output.strip_suffix('\n').unwrap_or(output);
    if output.is_empty() || output.contains(['\n', '\r']) {
        bail!("{command} returned malformed multiline state");
    }
    Ok(output)
}
