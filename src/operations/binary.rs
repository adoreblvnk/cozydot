use super::{Host, TempPath};
use crate::{config::BinaryFormat, platform::Architecture};
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use super::parsers::GithubRelease;

const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BinarySourceOperation {
    GithubLatest { repo: String, selector: String },
    Url { url: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryPackageOperation {
    name: String,
    format: BinaryFormat,
    architecture: Architecture,
    source: BinarySourceOperation,
}

pub(crate) mod cargo_binstall {
    use super::super::{Host, executable_file};
    use anyhow::{Context, Result};

    pub(crate) fn install(host: &Host) -> Result<()> {
        if cfg!(target_os = "macos") {
            return super::super::macos::install_formula(host, "cargo-binstall");
        }
        let cargo_home = host.home().join(".cargo");
        let installed = cargo_home.join("bin/cargo-binstall");
        if executable_file(&installed) {
            return Ok(());
        }
        let cargo_bin = cargo_home.join("bin/cargo");
        let program = cargo_bin
            .to_str()
            .with_context(|| format!("Cargo executable path is not UTF-8: {}", cargo_bin.display()))?;

        host.require("cargo-binstall install", program, ["install", "cargo-binstall", "--locked"])?;
        Ok(())
    }

    pub(crate) fn install_cargo_update(host: &Host) -> Result<()> {
        let cargo_home = host.home().join(".cargo");
        if executable_file(&cargo_home.join("bin/cargo-install-update")) {
            return Ok(());
        }
        let binstall = cargo_home.join("bin/cargo-binstall");
        let program = if cfg!(target_os = "macos") {
            super::super::macos::formula_executable(host, "cargo-binstall", "cargo-binstall")?
        } else {
            binstall
                .to_str()
                .with_context(|| format!("cargo-binstall executable path is not UTF-8: {}", binstall.display()))?
                .to_owned()
        };
        host.require("cargo-update install", &program, ["--no-confirm", "--", "cargo-update"])?;
        Ok(())
    }
}

impl BinaryPackageOperation {
    pub fn new(name: String, format: BinaryFormat, architecture: Architecture, source: BinarySourceOperation) -> Self {
        Self { name, format, architecture, source }
    }
}

pub(crate) fn install(host: &Host, operation: &BinaryPackageOperation) -> Result<()> {
    if installed(host, operation) {
        return Ok(());
    }
    let url = resolve(host, operation)?;
    let temporary = download(host, operation, &url)?;
    match operation.format {
        BinaryFormat::Deb => install_deb(host, temporary),
        BinaryFormat::Appimage => install_appimage(host, operation, temporary),
    }
}

fn installed(host: &Host, operation: &BinaryPackageOperation) -> bool {
    match operation.format {
        BinaryFormat::Deb => host.executable_on_path(&operation.name),
        BinaryFormat::Appimage => appimage_destination(host, operation).exists(),
    }
}

fn appimage_destination(host: &Host, operation: &BinaryPackageOperation) -> PathBuf {
    host.home().join("Applications").join(format!("{}.AppImage", operation.name))
}

fn resolve(host: &Host, operation: &BinaryPackageOperation) -> Result<String> {
    match &operation.source {
        BinarySourceOperation::Url { url } => Ok(url.clone()),
        BinarySourceOperation::GithubLatest { repo, selector } => {
            let endpoint = format!("https://api.github.com/repos/{repo}/releases/latest");
            let output = host.curl(
                "resolve binary package release",
                &endpoint,
                [
                    "--proto",
                    "=https",
                    "--header",
                    GITHUB_ACCEPT,
                    "--header",
                    GITHUB_API_VERSION,
                    "--header",
                    USER_AGENT,
                ],
            )?;
            select_asset(&output.stdout, selector, operation)
        }
    }
}

fn select_asset(input: &[u8], selector: &str, operation: &BinaryPackageOperation) -> Result<String> {
    let release: GithubRelease = serde_json::from_slice(input).context("parse GitHub release JSON")?;
    let pattern = Regex::new(selector).context("compile binary asset regex")?;
    let matches = release.assets.iter().filter(|asset| pattern.is_match(&asset.name)).collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!(
            "binary package {:?} ({}) selector matched {} assets",
            operation.name,
            operation.architecture.canonical(),
            matches.len()
        );
    }
    Ok(matches[0].browser_download_url.clone())
}

fn download(host: &Host, operation: &BinaryPackageOperation, url: &str) -> Result<TempPath> {
    let temporary = match operation.format {
        BinaryFormat::Deb => TempPath::new_with_suffix(host, &operation.name, ".deb")?,
        BinaryFormat::Appimage => {
            let applications = host.home().join("Applications");
            fs::create_dir_all(&applications).context("create Applications directory")?;
            // Stage beside the destination so the publishing rename never crosses filesystems.
            TempPath::new_in_with_suffix(&applications, &format!("{}-", operation.name), ".part")?
        }
    };
    host.curl("download binary package", url, ["--output".as_ref(), temporary.path().as_os_str()])?;
    Ok(temporary)
}

fn install_deb(host: &Host, temporary: TempPath) -> Result<()> {
    host.require(
        "binary Debian install",
        "sudo",
        [
            "DEBIAN_FRONTEND=noninteractive".as_ref(),
            "apt-get".as_ref(),
            "install".as_ref(),
            "-y".as_ref(),
            "-qq".as_ref(),
            "--".as_ref(),
            temporary.path().as_os_str(),
        ],
    )?;
    Ok(())
}

fn install_appimage(host: &Host, operation: &BinaryPackageOperation, temporary: TempPath) -> Result<()> {
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o755))?;
    fs::rename(temporary.path(), appimage_destination(host, operation)).context("publish AppImage into Applications")
}
