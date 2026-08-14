use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

use super::{Host, TempPath, path_program, real_executable_file};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoToolchainSelector {
    Latest,
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GoRelease {
    version: String,
    filename: String,
    sha256: String,
}

pub(crate) fn install_default_rust_toolchain(host: &Host, selector: &str) -> Result<()> {
    let rustup =
        managed_executable(host, ".cargo/bin/rustup", "Rust toolchain operation: rustup is unavailable after install")?;
    host.require("rustup toolchain install", &rustup, rust_install_args(selector))?;
    host.require("rustup default", &rustup, ["default", "--", selector])?;
    Ok(())
}

pub(crate) fn update_rust(host: &Host) -> Result<()> {
    let rustup =
        managed_executable(host, ".cargo/bin/rustup", "Rust toolchain update: rustup is unavailable after install")?;
    host.require("Rust toolchain update", &rustup, ["update"])?;
    Ok(())
}

pub(crate) fn install_go(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<()> {
    super::languages::add_go_to_path(host)?;
    let expected_arch = architecture.go();
    let requested = match selector {
        GoToolchainSelector::Latest => "latest",
        GoToolchainSelector::Version(version) => version,
    };
    let GoRelease { version, filename, sha256 } = resolve_go_release(host, requested, architecture)?;
    if inspect_go_installation(host, "/usr/local/go/bin/go")?
        .is_some_and(|installation| installation.version == version && installation.architecture == expected_arch)
    {
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
    let actual = format!("{:x}", Sha256::digest(fs::read(archive.path()).context("read downloaded Go archive")?));
    if actual != sha256 {
        bail!("downloaded Go archive checksum mismatch");
    }
    host.require("Go installation replacement", "sudo", ["rm", "-rf", "--", "/usr/local/go"])?;
    host.require(
        "Go archive extraction",
        "sudo",
        ["tar", "-xzf", archive.path().to_str().context("Go archive path is not UTF-8")?, "-C", "/usr/local"],
    )?;
    Ok(())
}

pub(crate) fn update_go(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<()> {
    if matches!(selector, GoToolchainSelector::Version(_)) {
        eprintln!("warning: Go update skipped because shared.tools.go is pinned to an exact version");
        return Ok(());
    }
    install_go(host, selector, architecture)
}

pub(crate) fn install_default_node_toolchain(host: &Host, selector: &str) -> Result<()> {
    let fnm = resolve_fnm(host)?;
    fnm_install(host, &fnm, selector)?;
    host.require("fnm default", &fnm, ["default", "--", fnm_alias(selector)])?;
    Ok(())
}

pub(crate) fn update_node(host: &Host, selector: &str) -> Result<()> {
    install_default_node_toolchain(host, selector)
}

fn fnm_install(host: &Host, fnm: &str, selector: &str) -> Result<()> {
    if selector == "lts" {
        host.require("fnm install", fnm, ["install", "--progress", "never", "--lts"])?;
    } else {
        host.require("fnm install", fnm, ["install", "--progress", "never", "--", selector])?;
    }
    Ok(())
}

pub(crate) fn install_default_python(host: &Host, version: &str) -> Result<()> {
    let uv = managed_executable(host, ".local/bin/uv", "Python toolchain operation: uv is unavailable after install")?;
    host.require(
        "uv python install",
        &uv,
        ["python", "install", "--no-config", "--managed-python", "--no-progress", "--default", "--", version],
    )?;
    Ok(())
}

pub(crate) fn update_python(host: &Host) -> Result<()> {
    let uv = managed_executable(host, ".local/bin/uv", "Python toolchain update: uv is unavailable after install")?;
    host.require("uv self update", &uv, ["self", "update"])?;
    host.require(
        "Python toolchain update",
        &uv,
        ["python", "upgrade", "--no-config", "--managed-python", "--no-progress"],
    )?;
    Ok(())
}

fn managed_executable(host: &Host, relative_path: &str, message: &str) -> Result<String> {
    let path = host.home().join(relative_path);
    if real_executable_file(&path) {
        return path_program(&path, "managed tool executable path");
    }
    bail!("{message}")
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
        let (version, filename, sha256) = super::super::select_go_release(
            std::str::from_utf8(&metadata.stdout).context("Go release metadata is not UTF-8")?,
            requested,
            architecture.go_archive(),
            target_os,
        )?;
        Ok(GoRelease { version, filename, sha256 })
    }
}

mod state {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct GoInstallation {
        pub(super) version: String,
        pub(super) architecture: String,
    }

    pub(super) fn inspect_go_installation(host: &Host, program: &str) -> Result<Option<GoInstallation>> {
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
        parse_go_version_output(&output.stdout).map(Some)
    }

    pub(super) fn parse_go_version_output(output: &[u8]) -> Result<GoInstallation> {
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
        Ok(GoInstallation {
            version: version.to_owned(),
            architecture: fields[3].trim_start_matches(&format!("{target_os}/")).to_owned(),
        })
    }

    pub(super) fn resolve_fnm(host: &Host) -> Result<String> {
        if cfg!(target_os = "macos") {
            return super::super::macos::formula_executable(host, "fnm", "fnm");
        }
        let data_home = host.home().join(".local/share");
        let managed = data_home.join("fnm/fnm");
        if real_executable_file(&managed) {
            return path_program(&managed, "managed fnm executable path");
        }
        bail!("Node toolchain operation: fnm is unavailable after install")
    }
}

use resolution::*;
use state::*;

fn rust_install_args(toolchain: &str) -> [&str; 7] {
    ["toolchain", "install", "--profile", "minimal", "--no-self-update", "--", toolchain]
}

fn fnm_alias(selector: &str) -> &str {
    match selector {
        "lts" => "lts-latest",
        value => value,
    }
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
