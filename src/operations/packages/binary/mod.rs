use crate::config::{BinaryFormat, BinaryPackage};
use crate::operations::host::{self, temp_path};
use anyhow::{Context, Result};
use std::path::PathBuf;

pub(crate) mod appimage;
mod executable;

pub(crate) fn is_installed(package: &BinaryPackage) -> Result<bool> {
    Ok(match package.format {
        BinaryFormat::Deb => host::has_executable_on_path(&package.name),
        BinaryFormat::AppImage => host::is_regular_executable(&appimage_path(package)?),
        BinaryFormat::Executable => host::is_regular_executable(&executable::path(package)?),
    })
}

pub(crate) fn install(package: &BinaryPackage, url: &str) -> Result<()> {
    match package.format {
        BinaryFormat::Deb => install_deb(package, url),
        BinaryFormat::AppImage => appimage::install_appimage("download binary package", url, &appimage_path(package)?),
        BinaryFormat::Executable => executable::install("download executable", url, &executable::path(package)?),
    }
}

fn appimage_path(package: &BinaryPackage) -> Result<PathBuf> {
    Ok(host::home()?.join("Applications").join(format!("{}.AppImage", package.name)))
}

fn install_deb(package: &BinaryPackage, url: &str) -> Result<()> {
    let temp = temp_path(&package.name, ".deb")?;
    let temp_path = temp.to_str().context("deb package temp path is not UTF-8")?;
    host::curl("download binary package", url, ["--output", temp_path])?;
    let args = ["DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y", "-qq", "--", temp_path];
    host::run("Deb package install", "sudo", args)?;
    Ok(())
}
