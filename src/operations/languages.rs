use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, os::unix::fs::PermissionsExt, path::PathBuf};

use super::{Host, TempPath};

const RUSTUP_BOOTSTRAP_FLAGS: [&str; 3] = ["-y", "--default-toolchain", "none"];

pub fn rustup(host: &Host) -> Result<()> {
    let cargo_home = host.value("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".cargo"));
    if !cargo_home.is_absolute() {
        bail!("rustup managed CARGO_HOME must be absolute");
    }
    if executable_file(&cargo_home.join("bin/rustup")) {
        return Ok(());
    }
    let installer = TempPath::new(host, "rustup")?;
    host.require(
        "rustup bootstrap download",
        "curl",
        ["--proto", "=https", "--tlsv1.2", "-sSf", "-o", &installer.path().to_string_lossy(), "https://sh.rustup.rs"],
    )?;
    host.require(
        "rustup bootstrap",
        "sh",
        std::iter::once(installer.path().as_os_str()).chain(RUSTUP_BOOTSTRAP_FLAGS.map(OsStr::new)),
    )?;
    if !executable_file(&cargo_home.join("bin/rustup")) {
        bail!("rustup bootstrap did not publish the managed rustup executable");
    }
    Ok(())
}

pub fn fnm_bootstrap(host: &Host) -> Result<()> {
    let data_home = host.value("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/share"));
    if !data_home.is_absolute() {
        bail!("FNM managed data directory must be absolute");
    }
    let installed = data_home.join("fnm/fnm");
    if executable_file(&installed) {
        return Ok(());
    }
    let installer = TempPath::new(host, "fnm-install")?;
    host.require(
        "FNM bootstrap download",
        "curl",
        ["-fsSL", "-o", &installer.path().to_string_lossy(), "https://fnm.vercel.app/install"],
    )?;
    host.require("FNM bootstrap", "bash", [&installer.path().to_string_lossy(), "--skip-shell"])?;
    if !executable_file(&installed) {
        bail!("FNM bootstrap did not publish executable {}", installed.display());
    }
    Ok(())
}

pub fn uv_bootstrap(host: &Host) -> Result<()> {
    let install_dir = host.value("UV_INSTALL_DIR").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/bin"));
    if !install_dir.is_absolute() {
        bail!("uv managed install directory must be absolute");
    }
    let installed = install_dir.join("uv");
    if executable_file(&installed) {
        return Ok(());
    }
    let installer = TempPath::new(host, "uv-install")?;
    host.require(
        "uv bootstrap download",
        "curl",
        ["-LsSf", "-o", &installer.path().to_string_lossy(), "https://astral.sh/uv/install.sh"],
    )?;
    std::fs::create_dir_all(&install_dir).context("uv bootstrap: create install directory")?;
    host.require(
        "uv bootstrap",
        "env",
        vec![
            format!("UV_UNMANAGED_INSTALL={}", install_dir.display()),
            "sh".into(),
            installer.path().to_string_lossy().into_owned(),
        ],
    )?;
    if !executable_file(&installed) {
        bail!("uv bootstrap did not publish executable {}", installed.display());
    }
    Ok(())
}

fn executable_file(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0)
}
