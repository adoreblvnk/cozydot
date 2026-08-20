use crate::operations::host::{Host, TempPath};
use anyhow::{Result, bail};

pub(crate) fn install(host: &Host) -> Result<()> {
    if find_brew(host).is_ok() {
        return Ok(());
    }
    let script = TempPath::new("homebrew-install")?;
    host.curl(
        "Homebrew installer download",
        "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh",
        [
            "--proto",
            "=https",
            "--output",
            script.path().to_str().ok_or_else(|| anyhow::anyhow!("Homebrew installer path is not UTF-8"))?,
        ],
    )?;
    host.run(
        "Homebrew install",
        "/bin/bash",
        [script.path().to_str().ok_or_else(|| anyhow::anyhow!("Homebrew installer path is not UTF-8"))?],
    )?;
    Ok(())
}

pub(crate) fn install_packages(host: &Host, formulae: &[String], casks: &[String]) -> Result<()> {
    let brew = find_brew(host)?;
    let install_args = ["HOMEBREW_NO_INSTALL_UPGRADE=1", brew.as_str(), "install"];
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
    let output = host.run("Homebrew formula prefix", &brew, ["--prefix", formula])?;
    let prefix = std::str::from_utf8(&output.stdout)?.trim();
    let program = std::path::Path::new(prefix).join("bin").join(executable);
    program.to_str().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("Homebrew executable path is not UTF-8"))
}

pub(crate) fn update_and_upgrade(host: &Host, formulae: bool, casks: bool) -> Result<()> {
    let brew = find_brew(host)?;
    host.run("Homebrew update", &brew, ["update"])?;
    if formulae {
        host.run("Homebrew formula upgrade", &brew, ["upgrade"])?;
    }
    if casks {
        host.run("Homebrew cask upgrade", &brew, ["upgrade", "--cask"])?;
    }
    Ok(())
}

fn find_brew(host: &Host) -> Result<String> {
    for candidate in ["brew", "/opt/homebrew/bin/brew"] {
        if host.output(candidate, ["--version"]).is_ok_and(|output| output.status.success()) {
            return Ok(candidate.to_owned());
        }
    }
    bail!("Homebrew is unavailable after install; expected brew on PATH or /opt/homebrew/bin/brew")
}
