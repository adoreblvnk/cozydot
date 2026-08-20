use crate::operations::host::{Host, TempPath};
use anyhow::{Result, bail};
use std::ffi::OsStr;

pub(crate) fn install(host: &Host) -> Result<()> {
    if find_brew(host).is_ok() {
        return Ok(());
    }
    let script = TempPath::new("homebrew-install")?;
    let url = "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh";
    let args: [&OsStr; 4] = ["--proto".as_ref(), "=https".as_ref(), "--output".as_ref(), script.path().as_os_str()];
    host.curl("Homebrew installer download", url, args)?;
    host.run("Homebrew install", "/bin/bash", [script.path().as_os_str()])?;
    Ok(())
}

pub(crate) fn install_packages(host: &Host, formulae: &[String], casks: &[String]) -> Result<()> {
    let brew = find_brew(host)?;
    let install_args = ["HOMEBREW_NO_INSTALL_UPGRADE=1", brew, "install"];
    // keep upgrades in the explicit update workflow
    if !formulae.is_empty() {
        let mut args = install_args.to_vec();
        args.extend(formulae.iter().map(String::as_str));
        host.run("Homebrew formula install", "/usr/bin/env", args)?;
    }
    if !casks.is_empty() {
        let mut args = install_args.to_vec();
        args.push("--cask");
        args.extend(casks.iter().map(String::as_str));
        host.run("Homebrew cask install", "/usr/bin/env", args)?;
    }
    Ok(())
}

pub(crate) fn formula_executable(host: &Host, formula: &str, executable: &str) -> Result<String> {
    let brew = find_brew(host)?;
    let output = host.run("Homebrew formula prefix", brew, ["--prefix", formula])?;
    let prefix = std::str::from_utf8(&output.stdout)?.trim();
    let program = std::path::Path::new(prefix).join("bin").join(executable);
    program.to_str().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("Homebrew executable path is not UTF-8"))
}

pub(crate) fn update_and_upgrade(host: &Host, formulae: bool, casks: bool) -> Result<()> {
    let brew = find_brew(host)?;
    host.run("Homebrew update", brew, ["update"])?;
    if formulae {
        host.run("Homebrew formula upgrade", brew, ["upgrade"])?;
    }
    if casks {
        host.run("Homebrew cask upgrade", brew, ["upgrade", "--cask"])?;
    }
    Ok(())
}

fn find_brew(host: &Host) -> Result<&'static str> {
    for candidate in ["brew", "/opt/homebrew/bin/brew"] {
        if host.output(candidate, ["--version"]).is_ok_and(|output| output.status.success()) {
            return Ok(candidate);
        }
    }
    bail!("Homebrew is unavailable after install; expected brew on PATH or /opt/homebrew/bin/brew")
}
