use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;

use super::{Host, TempPath, regular_executable_file, shell::append_profile};

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
    let target_os = if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let Release { version, filename, sha256 } = resolve_release(host, selector, architecture, target_os)?;
    let program = "/usr/local/go/bin/go";
    let expected = format!("go version go{version} {target_os}/{}", architecture.go());
    let current = regular_executable_file(program.as_ref())
        && host.output(program, ["version"]).is_ok_and(|output| {
            output.status.success() && std::str::from_utf8(&output.stdout).is_ok_and(|stdout| stdout.trim() == expected)
        });

    if !current {
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
        host.run("Go installation replacement", "sudo", ["rm", "-rf", "/usr/local/go"])?;
        host.run(
            "Go archive extraction",
            "sudo",
            ["tar", "-xzf", archive.path().to_str().context("Go archive path is not UTF-8")?, "-C", "/usr/local"],
        )?;
    }
    append_profile(host, GO_PATH_INIT)
}

pub(crate) fn update_toolchain(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<()> {
    if matches!(selector, GoToolchainSelector::Version(_)) {
        eprintln!("warning: Go update skipped because shared.tools.go is pinned to an exact version");
        return Ok(());
    }
    install_toolchain(host, selector, architecture)
}

fn resolve_release(
    host: &Host,
    selector: &GoToolchainSelector,
    architecture: Architecture,
    target_os: &str,
) -> Result<Release> {
    let metadata_url = match selector {
        GoToolchainSelector::Latest => "https://go.dev/dl/?mode=json",
        GoToolchainSelector::Version(_) => "https://go.dev/dl/?mode=json&include=all",
    };
    let metadata = host.curl("Go release resolution", metadata_url, ["--proto", "=https"])?;
    let releases: Vec<ReleaseMetadata> = serde_json::from_slice(&metadata.stdout).context("parse Go release JSON")?;
    let release = match selector {
        GoToolchainSelector::Latest => releases.first(),
        GoToolchainSelector::Version(version) => {
            releases.iter().find(|release| release.version.strip_prefix("go") == Some(version))
        }
    }
    .context("Go metadata has no matching release")?;
    let version = release.version.strip_prefix("go").context("Go metadata returned malformed release version")?;
    let filename = format!("go{version}.{target_os}-{}.tar.gz", architecture.go_archive());
    let file = release
        .files
        .iter()
        .find(|file| file.filename == filename)
        .context("Go metadata has no matching architecture archive")?;
    Ok(Release { version: version.to_owned(), filename, sha256: file.sha256.clone() })
}
