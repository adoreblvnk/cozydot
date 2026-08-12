use super::Host;
use crate::platform::Architecture;
use anyhow::{Context, Result};
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

const RELEASE_API: &str = "https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous";

pub(crate) fn execute(host: &Host, architecture: Architecture) -> Result<()> {
    if !host.run("systemctl", ["--user", "--quiet", "is-active", "appimaged.service"])?.status.success() {
        let _ = host.run("systemctl", ["--user", "stop", "appimaged.service"]);
        let _ = host.run("sudo", ["apt-get", "remove", "-qy", "appimagelauncher"]);

        let home = host.home();
        let _ = remove_if_present(&home.join(".config/systemd/user/default.target.wants/appimagelauncherd.service"));
        let _ = clear_cache(&home.join(".local/share/applications"));

        let applications = home.join("Applications");
        fs::create_dir_all(&applications).context("create Applications directory")?;
        let destination = applications.join("appimaged.AppImage");
        let url = resolve_asset(host, architecture)?;
        host.require(
            "download appimaged",
            "curl",
            [
                "--silent",
                "--show-error",
                "--location",
                "--output",
                destination.to_str().context("appimaged path is not UTF-8")?,
                "--",
                &url,
            ],
        )?;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o755)).context("make appimaged executable")?;
        host.require(
            "launch appimaged",
            destination.to_str().with_context(|| format!("appimaged path is not UTF-8: {}", destination.display()))?,
            std::iter::empty::<&str>(),
        )?;
    }

    ensure_fuse(host)
}

fn resolve_asset(host: &Host, architecture: Architecture) -> Result<String> {
    let output =
        host.require("resolve appimaged release", "curl", ["--silent", "--show-error", "--location", RELEASE_API])?;
    let release: Value = serde_json::from_slice(&output.stdout).context("parse appimaged release JSON")?;
    let suffix = match architecture {
        Architecture::Amd64 => "-x86_64.AppImage",
        Architecture::Arm64 | Architecture::DarwinArm64 => "-aarch64.AppImage",
        Architecture::Arm32 => "-armhf.AppImage",
    };
    release["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find_map(|asset| {
                let name = asset["name"].as_str()?;
                (name.starts_with("appimaged-") && name.ends_with(suffix))
                    .then(|| asset["browser_download_url"].as_str().map(str::to_owned))
                    .flatten()
            })
        })
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
        if host.run("apt-cache", ["show", "libfuse2t64"])?.status.success() { "libfuse2t64" } else { "libfuse2" };
    if !host.run("dpkg", ["--status", package])?.status.success() {
        host.require("refresh APT for AppImages", "sudo", ["apt-get", "update", "-qq"])?;
        host.require("install AppImage FUSE support", "sudo", ["apt-get", "install", "-qq", package])?;
    }
    Ok(())
}
