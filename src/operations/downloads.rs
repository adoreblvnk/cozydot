use super::{Host, TempPath};
use crate::json_helpers;
use anyhow::{bail, Context, Result};
use std::{fs, os::unix::fs::PermissionsExt};

pub fn nerdfont(host: &Host<'_>, font: &str) -> Result<()> {
    let family = format!(":family={font} Nerd Font");
    if host
        .require("nerdfont", "fc-list", [&family])?
        .stdout
        .is_empty()
    {
        let destination = format!("/usr/share/fonts/{font}");
        let archive = TempPath::new(host, "font.tar.xz")?;
        let url = format!(
            "https://github.com/ryanoasis/nerd-fonts/releases/latest/download/{font}.tar.xz"
        );
        host.require(
            "nerdfont",
            "curl",
            ["-fL", "-o", &archive.path().to_string_lossy(), &url],
        )?;
        host.require("nerdfont", "sudo", ["rm", "-rf", &destination])?;
        host.require("nerdfont", "sudo", ["mkdir", "-p", &destination])?;
        host.require(
            "nerdfont",
            "sudo",
            [
                "tar",
                "-xJ",
                "-C",
                &destination,
                "-f",
                &archive.path().to_string_lossy(),
            ],
        )?;
        host.require("nerdfont", "fc-cache", ["-f"])?;
    }
    Ok(())
}

pub fn binary(host: &Host<'_>, name: &str, url: &str, repo: &str, pattern: &str) -> Result<()> {
    let destination = host.home().join("Applications").join(name);
    if name.ends_with(".deb") {
        if host.command_exists(name.trim_end_matches(".deb")) {
            return Ok(());
        }
        if destination.exists() {
            fs::remove_file(&destination).context("remove incomplete Debian package")?;
        }
    } else if name.ends_with(".AppImage") {
        if executable_nonempty_file(&destination) {
            return Ok(());
        }
        if destination.exists() {
            fs::remove_file(&destination).context("remove incomplete AppImage")?;
        }
    }
    fs::create_dir_all(
        destination
            .parent()
            .context("binary destination has no parent")?,
    )?;
    let resolved_url = if repo.is_empty() {
        url.to_owned()
    } else {
        let endpoint = format!("https://api.github.com/repos/{repo}/releases/latest");
        let output = host.require("download binary", "curl", ["-fsSL", &endpoint])?;
        json_helpers::github_asset(&String::from_utf8(output.stdout)?, pattern)?
    };
    let temporary = TempPath::new(host, name)?;
    host.require(
        "download binary",
        "curl",
        [
            "-fL",
            "-o",
            &temporary.path().to_string_lossy(),
            &resolved_url,
        ],
    )?;
    if name.ends_with(".AppImage") {
        let mut permissions = fs::metadata(temporary.path())?.permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        fs::set_permissions(temporary.path(), permissions)?;
        fs::rename(temporary.path(), destination)?;
    } else if name.ends_with(".deb") {
        fs::rename(temporary.path(), &destination)?;
        let result = host.require(
            "download binary",
            "sudo",
            ["apt-get", "install", "-qq", &destination.to_string_lossy()],
        );
        let _ = fs::remove_file(&destination);
        result?;
    } else {
        bail!("download binary: unsupported package {name}");
    }
    Ok(())
}

fn executable_nonempty_file(path: &std::path::Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && metadata.len() > 0 && metadata.permissions().mode() & 0o111 != 0
    })
}
