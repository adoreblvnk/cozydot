use super::{Host, TempPath};
use crate::{config::BinaryFormat, platform::Architecture};
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::path::PathBuf;

use super::parsers::GithubRelease;

const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BinarySourceOperation {
    GithubLatest { repo: String, asset_pattern: String },
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
        let cargo_binstall_path = cargo_home.join("bin/cargo-binstall");
        if executable_file(&cargo_binstall_path) {
            return Ok(());
        }
        let cargo_bin = cargo_home.join("bin/cargo");
        let program = cargo_bin
            .to_str()
            .with_context(|| format!("Cargo executable path is not UTF-8: {}", cargo_bin.display()))?;

        host.run("cargo-binstall install", program, ["install", "cargo-binstall", "--locked"])?;
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
        host.run("cargo-update install", &program, ["--no-confirm", "--", "cargo-update"])?;
        Ok(())
    }
}

impl BinaryPackageOperation {
    pub fn new(name: String, format: BinaryFormat, architecture: Architecture, source: BinarySourceOperation) -> Self {
        Self { name, format, architecture, source }
    }
}

pub(crate) fn install(host: &Host, package: &BinaryPackageOperation) -> Result<()> {
    if is_installed(host, package) {
        return Ok(());
    }
    let url = resolve_url(host, package)?;
    match package.format {
        BinaryFormat::Deb => install_deb(host, download_deb(host, package, &url)?),
        BinaryFormat::Appimage => {
            super::appimage::install_appimage(host, "download binary package", &url, &appimage_path(host, package))
        }
    }
}

fn is_installed(host: &Host, package: &BinaryPackageOperation) -> bool {
    match package.format {
        BinaryFormat::Deb => host.executable_on_path(&package.name),
        BinaryFormat::Appimage => appimage_path(host, package).exists(),
    }
}

fn appimage_path(host: &Host, package: &BinaryPackageOperation) -> PathBuf {
    host.home().join("Applications").join(format!("{}.AppImage", package.name))
}

fn resolve_url(host: &Host, package: &BinaryPackageOperation) -> Result<String> {
    match &package.source {
        BinarySourceOperation::Url { url } => Ok(url.clone()),
        BinarySourceOperation::GithubLatest { repo, asset_pattern } => {
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
            select_asset_url(&output.stdout, asset_pattern, package)
        }
    }
}

fn select_asset_url(input: &[u8], asset_pattern: &str, package: &BinaryPackageOperation) -> Result<String> {
    let release: GithubRelease = serde_json::from_slice(input).context("parse GitHub release JSON")?;
    let pattern = Regex::new(asset_pattern).context("compile binary asset regex")?;
    let matches = release.assets.iter().filter(|asset| pattern.is_match(&asset.name)).collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!(
            "binary package {:?} ({}) asset pattern matched {} assets",
            package.name,
            package.architecture.canonical(),
            matches.len()
        );
    }
    Ok(matches[0].browser_download_url.clone())
}

fn download_deb(host: &Host, package: &BinaryPackageOperation, url: &str) -> Result<TempPath> {
    let temp = TempPath::new_with_suffix(host, &package.name, ".deb")?;
    host.curl("download binary package", url, ["--output".as_ref(), temp.path().as_os_str()])?;
    Ok(temp)
}

fn install_deb(host: &Host, temp: TempPath) -> Result<()> {
    host.run(
        "binary Debian install",
        "sudo",
        [
            "DEBIAN_FRONTEND=noninteractive".as_ref(),
            "apt-get".as_ref(),
            "install".as_ref(),
            "-y".as_ref(),
            "-qq".as_ref(),
            "--".as_ref(),
            temp.path().as_os_str(),
        ],
    )?;
    Ok(())
}
