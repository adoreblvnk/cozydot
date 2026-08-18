use crate::operations::host::{Host, TempPath};
use crate::{
    config::{BinaryFormat, BinaryPackage, BinarySource},
    platform::Architecture,
};

mod appimage;
pub(crate) mod appimaged;
mod github;
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::path::PathBuf;

use self::github::Release;

const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

pub(crate) fn install(host: &Host, package: &BinaryPackage, architecture: Architecture) -> Result<()> {
    if is_installed(host, package) {
        return Ok(());
    }
    let Some(url) = resolve_url(host, package, architecture)? else { return Ok(()) };
    match package.format {
        BinaryFormat::Deb => install_deb(host, download_deb(host, package, &url)?),
        BinaryFormat::AppImage => {
            appimage::install_appimage(host, "download binary package", &url, &appimage_path(host, package))
        }
    }
}

fn is_installed(host: &Host, package: &BinaryPackage) -> bool {
    match package.format {
        BinaryFormat::Deb => host.executable_on_path(&package.name),
        BinaryFormat::AppImage => appimage_path(host, package).exists(),
    }
}

fn appimage_path(host: &Host, package: &BinaryPackage) -> PathBuf {
    host.home().join("Applications").join(format!("{}.AppImage", package.name))
}

fn resolve_url(host: &Host, package: &BinaryPackage, architecture: Architecture) -> Result<Option<String>> {
    match &package.source {
        BinarySource::Url { urls } => Ok(urls.get(architecture).map(str::to_owned)),
        BinarySource::GitHub { repo, assets } => {
            let Some(asset_pattern) = assets.get(architecture) else { return Ok(None) };
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
            select_asset_url(&output.stdout, asset_pattern, &package.name, architecture).map(Some)
        }
    }
}

fn select_asset_url(input: &[u8], asset_pattern: &str, package: &str, architecture: Architecture) -> Result<String> {
    let release: Release = serde_json::from_slice(input).context("parse GitHub release JSON")?;
    let pattern = Regex::new(asset_pattern).context("compile binary asset regex")?;
    let matches = release.assets.iter().filter(|asset| pattern.is_match(&asset.name)).collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!(
            "binary package {:?} ({}) asset pattern matched {} assets",
            package,
            architecture.as_str(),
            matches.len()
        );
    }
    Ok(matches[0].browser_download_url.clone())
}

fn download_deb(host: &Host, package: &BinaryPackage, url: &str) -> Result<TempPath> {
    let temp = TempPath::new_with_suffix(&package.name, ".deb")?;
    host.curl("download binary package", url, ["--output".as_ref(), temp.path().as_os_str()])?;
    Ok(temp)
}

fn install_deb(host: &Host, temp: TempPath) -> Result<()> {
    host.run(
        "Deb package install",
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
