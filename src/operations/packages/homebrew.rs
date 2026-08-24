use crate::operations::host::{self, temp_path};
use anyhow::{Context, Result};
use std::ffi::OsStr;

const HOMEBREW_UNAVAILABLE: &str =
    "Homebrew is unavailable after install; expected brew on PATH or /opt/homebrew/bin/brew";

pub(crate) fn install() -> Result<()> {
    if find_executable()?.is_some() {
        return Ok(());
    }
    let script = temp_path("homebrew-install", "")?;
    let url = "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh";
    let args: [&OsStr; 4] = ["--proto".as_ref(), "=https".as_ref(), "--output".as_ref(), script.as_os_str()];
    host::curl("Homebrew installer download", url, args)?;
    host::run("Homebrew install", "/bin/bash", [script.as_os_str()])?;
    Ok(())
}

pub(crate) fn install_packages(formulae: &[String], casks: &[String]) -> Result<()> {
    let brew = find_executable()?.context(HOMEBREW_UNAVAILABLE)?;
    let install_args = ["HOMEBREW_NO_INSTALL_UPGRADE=1", brew.as_str(), "install"];
    // keep upgrades in the explicit update workflow
    if !formulae.is_empty() {
        let mut args = install_args.to_vec();
        args.extend(formulae.iter().map(String::as_str));
        host::run("Homebrew formula install", "/usr/bin/env", args)?;
    }
    if !casks.is_empty() {
        let mut args = install_args.to_vec();
        args.push("--cask");
        args.extend(casks.iter().map(String::as_str));
        host::run("Homebrew cask install", "/usr/bin/env", args)?;
    }
    Ok(())
}

pub(crate) fn executable_path(formula: &str, executable: &str) -> Result<String> {
    let brew = find_executable()?.context(HOMEBREW_UNAVAILABLE)?;
    let output = host::run("Homebrew formula prefix", &brew, ["--prefix", formula])?;
    let prefix = std::str::from_utf8(&output.stdout)?.trim();
    let program = std::path::Path::new(prefix).join("bin").join(executable);
    program.to_str().map(str::to_owned).context("Homebrew executable path is not UTF-8")
}

pub(crate) fn update_and_upgrade(formulae: bool, casks: bool) -> Result<()> {
    let brew = find_executable()?.context(HOMEBREW_UNAVAILABLE)?;
    host::run("Homebrew update", &brew, ["update"])?;
    if formulae {
        host::run("Homebrew formula upgrade", &brew, ["upgrade"])?;
    }
    if casks {
        host::run("Homebrew cask upgrade", &brew, ["upgrade", "--cask"])?;
    }
    Ok(())
}

fn find_executable() -> Result<Option<String>> {
    for candidate in ["brew", "/opt/homebrew/bin/brew"] {
        if host::output(candidate, ["--version"]).is_ok_and(|output| output.status.success()) {
            return Ok(Some(candidate.to_owned()));
        }
    }
    Ok(None)
}
