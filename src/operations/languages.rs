use anyhow::{bail, Context, Result};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

use super::{Host, TempPath};

pub fn fnm_bootstrap(host: &Host<'_>) -> Result<()> {
    let data_home = host
        .value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"));
    let installed = data_home.join("fnm/fnm");
    if host.command_exists("fnm") || executable_file(&installed) {
        return Ok(());
    }
    let installer = TempPath::new(host, "fnm-install")?;
    host.require(
        "FNM bootstrap download",
        "curl",
        [
            "-fsSL",
            "-o",
            &installer.path().to_string_lossy(),
            "https://fnm.vercel.app/install",
        ],
    )?;
    host.require(
        "FNM bootstrap",
        "bash",
        [&installer.path().to_string_lossy(), "--skip-shell"],
    )?;
    if !executable_file(&installed) {
        bail!(
            "FNM bootstrap did not publish executable {}",
            installed.display()
        );
    }
    Ok(())
}

pub fn uv_bootstrap(host: &Host<'_>) -> Result<()> {
    let install_dir = host.home().join(".local/bin");
    let installed = install_dir.join("uv");
    if host.command_exists("uv") || executable_file(&installed) {
        return Ok(());
    }
    let installer = TempPath::new(host, "uv-install")?;
    host.require(
        "UV bootstrap download",
        "curl",
        [
            "-LsSf",
            "-o",
            &installer.path().to_string_lossy(),
            "https://astral.sh/uv/install.sh",
        ],
    )?;
    std::fs::create_dir_all(&install_dir).context("UV bootstrap: create install directory")?;
    host.require(
        "UV bootstrap",
        "env",
        vec![
            format!("UV_UNMANAGED_INSTALL={}", install_dir.display()),
            "sh".into(),
            installer.path().to_string_lossy().into_owned(),
        ],
    )?;
    if !executable_file(&installed) {
        bail!(
            "UV bootstrap did not publish executable {}",
            installed.display()
        );
    }
    Ok(())
}

fn executable_file(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
