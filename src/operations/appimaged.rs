use super::Host;
use super::parsers::GithubRelease;
use crate::platform::Architecture;
use anyhow::{Context, Result};
use std::{fs, path::Path};

const RELEASE_API: &str = "https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous";

pub(crate) fn install(host: &Host, architecture: Architecture) -> Result<()> {
    if !host.output("systemctl", ["--user", "--quiet", "is-active", "appimaged.service"])?.status.success() {
        // legacy cleanup is best-effort so failures don't block install
        let _ = host.output("systemctl", ["--user", "stop", "appimaged.service"]);
        let _ = host.output("sudo", ["apt-get", "remove", "-qy", "appimagelauncher"]);

        let home = host.home();
        let _ = remove_if_present(&home.join(".config/systemd/user/default.target.wants/appimagelauncherd.service"));
        let _ = clear_cache(&home.join(".local/share/applications"));

        let applications = home.join("Applications");
        let destination = applications.join("appimaged.AppImage");
        let url = resolve_asset_url(host, architecture)?;
        super::appimage::install_appimage(host, "download appimaged", &url, &destination)?;
        host.run(
            "launch appimaged",
            destination.to_str().with_context(|| format!("appimaged path is not UTF-8: {}", destination.display()))?,
            std::iter::empty::<&str>(),
        )?;
    }

    ensure_fuse(host)
}

fn resolve_asset_url(host: &Host, architecture: Architecture) -> Result<String> {
    let output = host.curl("resolve appimaged release", RELEASE_API, std::iter::empty::<&str>())?;
    let release: GithubRelease = serde_json::from_slice(&output.stdout).context("parse appimaged release JSON")?;
    let suffix = match architecture {
        Architecture::Amd64 => "-x86_64.AppImage",
        Architecture::Arm64 | Architecture::DarwinArm64 => "-aarch64.AppImage",
        Architecture::Arm32 => "-armhf.AppImage",
    };
    release
        .assets
        .into_iter()
        .find(|asset| asset.name.starts_with("appimaged-") && asset.name.ends_with(suffix))
        .map(|asset| asset.browser_download_url)
        .with_context(|| format!("appimaged release has no asset for {}", architecture.canonical()))
}

fn clear_cache(directory: &Path) -> Result<()> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry?.path();
        if path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("appimage")) {
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn ensure_fuse(host: &Host) -> Result<()> {
    let package =
        if host.output("apt-cache", ["show", "libfuse2t64"])?.status.success() { "libfuse2t64" } else { "libfuse2" };
    if !host.output("dpkg", ["--status", package])?.status.success() {
        host.run("APT update for AppImages", "sudo", ["apt-get", "update", "-qq"])?;
        host.run("AppImage FUSE support install", "sudo", ["apt-get", "install", "-qq", package])?;
    }
    Ok(())
}
