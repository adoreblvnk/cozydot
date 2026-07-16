use anyhow::{bail, Result};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

use super::{Host, TempPath};

pub fn rustup(host: &Host<'_>) -> Result<()> {
    let cargo_home = host
        .value("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".cargo"));
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
        [
            "--proto",
            "=https",
            "--tlsv1.2",
            "-sSf",
            "-o",
            &installer.path().to_string_lossy(),
            "https://sh.rustup.rs",
        ],
    )?;
    host.require(
        "rustup bootstrap",
        "sh",
        [installer.path().as_os_str(), "-y".as_ref()],
    )?;
    if !executable_file(&cargo_home.join("bin/rustup")) {
        bail!("rustup bootstrap did not publish the managed rustup executable");
    }
    Ok(())
}

fn executable_file(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0
    })
}
