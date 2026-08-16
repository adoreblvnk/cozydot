use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

use super::{Host, TempPath, host::one_record, regular_executable_file, shell::append_profile};

const GO_PATH_INIT: &str = r#"export PATH="/usr/local/go/bin:$PATH""#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoToolchainSelector {
    Latest,
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Release {
    version: String,
    filename: String,
    sha256: String,
}

#[derive(Deserialize)]
struct ReleaseMetadata {
    version: String,
    files: Vec<FileMetadata>,
}

#[derive(Deserialize)]
struct FileMetadata {
    filename: String,
    sha256: String,
}

pub(crate) fn install_toolchain(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<()> {
    append_profile(host, GO_PATH_INIT)?;
    let expected_arch = architecture.go();
    let Release { version, filename, sha256 } = resolve_release(host, selector, architecture)?;
    if inspect_installation(host, "/usr/local/go/bin/go")?
        .is_some_and(|installation| installation.version == version && installation.architecture == expected_arch)
    {
        return Ok(());
    }

    let archive = TempPath::new_with_suffix(host, "go", ".tar.gz")?;
    let url = format!("https://go.dev/dl/{filename}");
    host.curl(
        "Go archive download",
        &url,
        ["--proto".as_ref(), "=https".as_ref(), "--output".as_ref(), archive.path().as_os_str()],
    )?;
    let actual = format!("{:x}", Sha256::digest(fs::read(archive.path()).context("read downloaded Go archive")?));
    if actual != sha256 {
        bail!("downloaded Go archive checksum mismatch");
    }
    // remove whole tree so files missing from new release can't survive replacement
    host.run_checked("Go installation replacement", "sudo", ["rm", "-rf", "--", "/usr/local/go"])?;
    host.run_checked(
        "Go archive extraction",
        "sudo",
        ["tar", "-xzf", archive.path().to_str().context("Go archive path is not UTF-8")?, "-C", "/usr/local"],
    )?;
    Ok(())
}

pub(crate) fn update_toolchain(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<()> {
    if matches!(selector, GoToolchainSelector::Version(_)) {
        eprintln!("warning: Go update skipped because shared.tools.go is pinned to an exact version");
        return Ok(());
    }
    install_toolchain(host, selector, architecture)
}

fn select_release(input: &str, selector: &GoToolchainSelector, arch: &str, target_os: &str) -> Result<Release> {
    let releases: Vec<ReleaseMetadata> = serde_json::from_str(input).context("parse Go release JSON")?;
    let version = releases
        .iter()
        .map(|release| release.version.as_str())
        .filter(|v| stable_version(v))
        .map(|v| v.trim_start_matches("go"))
        .find(|version| match selector {
            GoToolchainSelector::Latest => true,
            GoToolchainSelector::Version(requested) => *version == requested,
        })
        .context("Go metadata has no matching stable release")?;
    let filename = format!("go{version}.{target_os}-{arch}.tar.gz");
    let file = releases
        .iter()
        .find(|release| release.version == format!("go{version}"))
        .and_then(|release| release.files.iter().find(|file| file.filename == filename))
        .context("Go metadata has no matching architecture archive")?;
    Ok(Release { version: version.to_owned(), filename, sha256: file.sha256.clone() })
}

fn stable_version(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("go") else {
        return false;
    };
    let parts = rest.split('.').collect::<Vec<_>>();
    (parts.len() == 2 || parts.len() == 3)
        && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn resolve_release(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<Release> {
    let metadata =
        host.curl("Go release resolution", "https://go.dev/dl/?mode=json&include=all", ["--proto", "=https"])?;
    let target_os = if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    select_release(
        std::str::from_utf8(&metadata.stdout).context("Go release metadata is not UTF-8")?,
        selector,
        architecture.go_archive(),
        target_os,
    )
}

#[derive(Debug, PartialEq, Eq)]
struct Installation {
    version: String,
    architecture: String,
}

fn inspect_installation(host: &Host, program: &str) -> Result<Option<Installation>> {
    if program.starts_with('/') && !regular_executable_file(Path::new(program)) {
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
    parse_version_output(&output.stdout).map(Some)
}

fn parse_version_output(output: &[u8]) -> Result<Installation> {
    let output = one_record(output, "go version")?;
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
    Ok(Installation {
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
