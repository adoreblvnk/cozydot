use crate::operations::host::{self, temp_path};
use crate::{
    config::{BinaryFormat, BinaryPackage, BinarySource},
    platform::Arch,
};
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::path::PathBuf;

use self::github::Release;

mod appimage;
pub(crate) mod appimaged;
mod github;

const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

pub(crate) enum SelectedSource<'a> {
    GitHub { repo: &'a str, asset_pattern: &'a str },
    Url(&'a str),
}

pub(crate) fn select_source(package: &BinaryPackage, arch: Arch) -> Option<SelectedSource<'_>> {
    match &package.source {
        BinarySource::GitHub { repo, assets } => {
            assets.get(arch).map(|asset_pattern| SelectedSource::GitHub { repo, asset_pattern })
        }
        BinarySource::Url { urls } => urls.get(arch).map(SelectedSource::Url),
    }
}

pub(crate) fn install(package: &BinaryPackage, arch: Arch, source: SelectedSource<'_>) -> Result<()> {
    let home = host::home()?;
    if is_installed(&home, package) {
        return Ok(());
    }
    let url = resolve_url(package, arch, source)?;
    match package.format {
        BinaryFormat::Deb => install_deb(package, &url),
        BinaryFormat::AppImage => {
            appimage::install_appimage("download binary package", &url, &appimage_path(&home, package))
        }
    }
}

fn is_installed(home: &std::path::Path, package: &BinaryPackage) -> bool {
    match package.format {
        BinaryFormat::Deb => host::has_executable_on_path(&package.name),
        BinaryFormat::AppImage => appimage_path(home, package).exists(),
    }
}

fn appimage_path(home: &std::path::Path, package: &BinaryPackage) -> PathBuf {
    home.join("Applications").join(format!("{}.AppImage", package.name))
}

fn resolve_url(package: &BinaryPackage, arch: Arch, source: SelectedSource<'_>) -> Result<String> {
    match source {
        SelectedSource::Url(url) => Ok(url.to_owned()),
        SelectedSource::GitHub { repo, asset_pattern } => {
            let endpoint = format!("https://api.github.com/repos/{repo}/releases/latest");
            let accept = GITHUB_ACCEPT;
            let version = GITHUB_API_VERSION;
            let args = ["--proto", "=https", "--header", accept, "--header", version, "--header", USER_AGENT];
            let output = host::curl("resolve binary package release", &endpoint, args)?;
            select_asset_url(&output.stdout, asset_pattern, &package.name, arch)
        }
    }
}

fn select_asset_url(input: &[u8], asset_pattern: &str, package: &str, arch: Arch) -> Result<String> {
    let release: Release = serde_json::from_slice(input).context("parse GitHub release JSON")?;
    let pattern = Regex::new(asset_pattern).context("compile binary asset regex")?;
    let matches = release.assets.iter().filter(|asset| pattern.is_match(&asset.name)).collect::<Vec<_>>();
    if matches.len() != 1 {
        let arch = arch.as_str();
        bail!("binary package {:?} ({}) asset pattern matched {} assets", package, arch, matches.len());
    }
    Ok(matches[0].browser_download_url.clone())
}

fn install_deb(package: &BinaryPackage, url: &str) -> Result<()> {
    let temp = temp_path(&package.name, ".deb")?;
    host::curl("download binary package", url, ["--output".as_ref(), temp.as_os_str()])?;
    host::run(
        "Deb package install",
        "sudo",
        [
            "DEBIAN_FRONTEND=noninteractive".as_ref(),
            "apt-get".as_ref(),
            "install".as_ref(),
            "-y".as_ref(),
            "-qq".as_ref(),
            "--".as_ref(),
            temp.as_os_str(),
        ],
    )?;
    Ok(())
}
