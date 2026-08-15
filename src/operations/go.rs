use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

use super::{Host, TempPath, real_executable_file, shell::append_profile};

const GO_PATH_INIT: &str = r#"export PATH="/usr/local/go/bin:$PATH""#;

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

pub(crate) fn install_go(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<()> {
    add_go_to_path(host)?;
    let expected_arch = architecture.go();
    let GoRelease { version, filename, sha256 } = resolve_go_release(host, selector, architecture)?;
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

fn select_go_release(input: &str, selector: &GoToolchainSelector, arch: &str, target_os: &str) -> Result<GoRelease> {
    let value: serde_json::Value = serde_json::from_str(input).context("parse Go release JSON")?;
    let releases = value.as_array().context("Go metadata must be an array")?;
    let version = releases
        .iter()
        .filter_map(|release| release["version"].as_str())
        .filter(|v| stable_go_version(v))
        .map(|v| v.trim_start_matches("go"))
        .find(|version| match selector {
            GoToolchainSelector::Latest => true,
            GoToolchainSelector::Version(requested) => {
                *version == requested || version.strip_prefix(requested).is_some_and(|rest| rest.starts_with('.'))
            }
        })
        .context("Go metadata has no matching stable release")?;
    let filename = format!("go{version}.{target_os}-{arch}.tar.gz");
    let file = releases
        .iter()
        .find(|release| release["version"].as_str() == Some(&format!("go{version}")))
        .and_then(|release| release["files"].as_array())
        .and_then(|files| files.iter().find(|file| file["filename"].as_str() == Some(&filename)))
        .context("Go metadata has no matching architecture archive")?;
    let sha256 = file["sha256"].as_str().context("Go archive metadata has no SHA-256 checksum")?;
    Ok(GoRelease { version: version.to_owned(), filename, sha256: sha256.to_owned() })
}

fn stable_go_version(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("go") else {
        return false;
    };
    let parts = rest.split('.').collect::<Vec<_>>();
    (parts.len() == 2 || parts.len() == 3)
        && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

pub fn add_go_to_path(host: &Host) -> Result<()> {
    append_profile(host, GO_PATH_INIT)
}

fn resolve_go_release(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<GoRelease> {
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
    select_go_release(
        std::str::from_utf8(&metadata.stdout).context("Go release metadata is not UTF-8")?,
        selector,
        architecture.go_archive(),
        target_os,
    )
}

#[derive(Debug, PartialEq, Eq)]
struct GoInstallation {
    version: String,
    architecture: String,
}

fn inspect_go_installation(host: &Host, program: &str) -> Result<Option<GoInstallation>> {
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

fn parse_go_version_output(output: &[u8]) -> Result<GoInstallation> {
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
