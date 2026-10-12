use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Deserialize;

use crate::operations::{host, packages::apt};
use crate::platform::Arch;

const RELEASE_API: &str = "https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous";

#[derive(Deserialize)]
struct Release {
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub(super) fn install_appimage(label: &str, url: &str, destination: &Path) -> Result<()> {
    let parent = destination.parent();
    let parent = parent.with_context(|| format!("AppImage destination has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent).context("create AppImage destination directory")?;
    // keep incomplete downloads hidden until permissions & contents are ready
    let temp = tempfile::NamedTempFile::new_in(parent).context("create temporary AppImage")?;
    host::curl(label, url, ["--output".as_ref(), temp.path().as_os_str()])?;
    temp.as_file().set_permissions(fs::Permissions::from_mode(0o755)).context("make AppImage executable")?;
    temp.persist(destination).with_context(|| format!("install AppImage at {}", destination.display()))?;
    Ok(())
}

pub(crate) fn install_daemon(arch: Arch) -> Result<()> {
    ensure_fuse()?;
    if !host::output("systemctl", ["--user", "--quiet", "is-active", "appimaged.service"])?.status.success() {
        // https://github.com/probonopd/go-appimage/blob/master/src/appimaged/README.md#initial-setup
        // cleanup is idempotent; an absent service or package is already the desired state
        let _ = host::output("systemctl", ["--user", "stop", "appimaged.service"]);
        let _ = host::output("sudo", ["apt-get", "-y", "purge", "appimagelauncher"]);

        let home = host::home()?;
        let service = home.join(".config/systemd/user/default.target.wants/appimagelauncherd.service");
        let _ = fs::remove_file(&service);
        host::run("reload user services", "systemctl", ["--user", "daemon-reload"])?;
        let cache = home.join(".local/share/applications");
        let cache_path = cache.to_str().context("applications cache directory is not UTF-8")?;
        // pass the cache path as $1 so the shell never parses user-controlled path contents
        host::run("clear AppImage cache", "sh", ["-c", r#"rm -f -- "$1"/appimage*"#, "sh", cache_path])?;

        let destination = home.join("Applications/appimaged.AppImage");
        install_appimage("download appimaged", &resolve_asset_url(arch)?, &destination)?;
        let appimaged = host::path_program(&destination, "appimaged path")?;
        host::run("launch appimaged", &appimaged, std::iter::empty::<&str>())?;
    }

    Ok(())
}

fn resolve_asset_url(arch: Arch) -> Result<String> {
    let output = host::curl("resolve appimaged release", RELEASE_API, std::iter::empty::<&str>())?;
    let release: Release = serde_json::from_slice(&output.stdout).context("parse GitHub release JSON")?;
    let pattern = format!(r"^appimaged-.*-{}\.AppImage$", arch.as_str());
    let regex = Regex::new(&pattern).context("compile appimaged asset regex")?;
    let matches = release.assets.iter().filter(|asset| regex.is_match(&asset.name)).collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!("appimaged asset pattern matched {} assets for {}", matches.len(), arch.as_str());
    }
    Ok(matches[0].browser_download_url.clone())
}

fn ensure_fuse() -> Result<()> {
    // newer releases use libfuse2t64 while older releases retain libfuse2
    let output = host::output("apt-cache", ["show", "libfuse2t64"])?;
    let package = if output.status.success() { "libfuse2t64" } else { "libfuse2" };
    apt::install(&[package.into()])
}
