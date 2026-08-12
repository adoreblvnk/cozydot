use anyhow::{Context, Result, bail};
use std::ffi::OsStr;

use super::{Host, TempPath, real_executable_file};

const RUSTUP_BOOTSTRAP_FLAGS: [&str; 3] = ["-y", "--default-toolchain", "none"];

pub fn rustup(host: &Host) -> Result<()> {
    let cargo_home = host.managed_dir("CARGO_HOME", ".cargo", "rustup managed CARGO_HOME must be absolute")?;
    if real_executable_file(&cargo_home.join("bin/rustup")) {
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
    if !real_executable_file(&cargo_home.join("bin/rustup")) {
        bail!("rustup bootstrap did not publish the managed rustup executable");
    }
    Ok(())
}

pub fn fnm_bootstrap(host: &Host) -> Result<()> {
    let data_home = host.managed_dir("XDG_DATA_HOME", ".local/share", "FNM managed data directory must be absolute")?;
    let installed = data_home.join("fnm/fnm");
    if real_executable_file(&installed) {
        return Ok(());
    }
    let installer = TempPath::new(host, "fnm-install")?;
    host.require(
        "FNM bootstrap download",
        "curl",
        ["-fsSL", "-o", &installer.path().to_string_lossy(), "https://fnm.vercel.app/install"],
    )?;
    host.require("FNM bootstrap", "bash", [&installer.path().to_string_lossy(), "--skip-shell"])?;
    if !real_executable_file(&installed) {
        bail!("FNM bootstrap did not publish executable {}", installed.display());
    }
    Ok(())
}

pub fn uv_bootstrap(host: &Host) -> Result<()> {
    let install_dir =
        host.managed_dir("UV_INSTALL_DIR", ".local/bin", "uv managed install directory must be absolute")?;
    let installed = install_dir.join("uv");
    if real_executable_file(&installed) {
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
    if !real_executable_file(&installed) {
        bail!("uv bootstrap did not publish executable {}", installed.display());
    }
    Ok(())
}
