use super::{publish_file, Host, TempPath};
use crate::json_helpers;
use anyhow::{Context, Result};
use std::fs;

const RELEASE_API: &str =
    "https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous";

pub(super) fn execute(host: &Host<'_>, arch: &str) -> Result<()> {
    let fuse = if host
        .run("apt-cache", ["show", "libfuse2t64"])?
        .status
        .success()
    {
        "libfuse2t64"
    } else {
        "libfuse2"
    };
    if !host.run("dpkg", ["-s", fuse])?.status.success() {
        host.require("appimaged", "sudo", ["apt-get", "update", "-qq"])?;
        host.require("appimaged", "sudo", ["apt-get", "install", "-qq", fuse])?;
    }

    let active = host
        .run("systemctl", ["--user", "-q", "is-active", "appimaged"])?
        .status
        .success();
    if !active {
        let _ = host.run("systemctl", ["--user", "stop", "appimaged.service"])?;
        if host
            .run("dpkg", ["-s", "appimagelauncher"])?
            .status
            .success()
        {
            host.require(
                "appimaged",
                "sudo",
                ["apt-get", "remove", "-qy", "appimagelauncher"],
            )?;
        }

        let home = host.home();
        let service =
            home.join(".config/systemd/user/default.target.wants/appimagelauncherd.service");
        if service.is_file() || service.is_symlink() {
            fs::remove_file(&service)
                .with_context(|| format!("appimaged: remove {}", service.display()))?;
        }
        host.require("appimaged", "systemctl", ["--user", "daemon-reload"])?;
        let applications = home.join(".local/share/applications");
        if let Ok(entries) = fs::read_dir(&applications) {
            for entry in entries {
                let path = entry?.path();
                if path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("appimage"))
                {
                    if path.is_dir() {
                        fs::remove_dir_all(&path)?;
                    } else {
                        fs::remove_file(&path)?;
                    }
                }
            }
        }
        let destination_dir = home.join("Applications");
        fs::create_dir_all(&destination_dir).context("appimaged: create Applications directory")?;
        let release = host.require("appimaged", "curl", ["-fsSL", RELEASE_API])?;
        let pattern = format!("*appimaged*{arch}.AppImage");
        let url = json_helpers::github_asset(
            std::str::from_utf8(&release.stdout)
                .context("appimaged: GitHub response is not UTF-8")?,
            &pattern,
        )?;
        let temporary = TempPath::new(host, "appimaged")?;
        host.require(
            "appimaged",
            "curl",
            ["-fL", "-o", &temporary.path().to_string_lossy(), &url],
        )?;
        let destination = destination_dir.join("appimaged.AppImage");
        publish_file(temporary.path(), &destination, 0o755)
            .context("appimaged: install AppImage")?;
        host.require(
            "appimaged",
            destination.to_string_lossy().as_ref(),
            std::iter::empty::<&str>(),
        )?;
        host.require(
            "appimaged readiness",
            "systemctl",
            ["--user", "-q", "is-active", "appimaged"],
        )?;
    }
    Ok(())
}
