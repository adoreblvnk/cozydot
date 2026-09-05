use crate::operations::host::{self, temp_path};
use anyhow::{Context, Result};

const HOMEBREW_UNAVAILABLE: &str =
    "Homebrew is unavailable after install; expected brew on PATH or /opt/homebrew/bin/brew";

pub(crate) fn is_installed() -> Result<bool> {
    Ok(find_executable()?.is_some())
}

pub(crate) fn any_missing(formulae: &[String], casks: &[String]) -> Result<bool> {
    if formulae.is_empty() && casks.is_empty() {
        return Ok(false);
    }
    let Some(brew) = find_executable()? else { return Ok(true) };
    let missing_formulae = missing_packages(&brew, "--formula", formulae)?;
    if !missing_formulae.is_empty() {
        return Ok(true);
    }
    let missing_casks = missing_packages(&brew, "--cask", casks)?;
    Ok(!missing_casks.is_empty())
}

pub(crate) fn install() -> Result<()> {
    let script = temp_path("homebrew-install", "")?;
    let path = script.to_str().context("Homebrew installer path is not UTF-8")?;
    let args = ["--proto", "=https", "--output", path];
    let url = "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh";
    host::curl("Homebrew installer download", url, args)?;
    host::run("Homebrew install", "/bin/bash", [path])?;
    Ok(())
}

pub(crate) fn install_packages(formulae: &[String], casks: &[String]) -> Result<()> {
    let brew = find_executable()?.context(HOMEBREW_UNAVAILABLE)?;
    let install_args = ["HOMEBREW_NO_INSTALL_UPGRADE=1", brew.as_str(), "install"];
    // keep upgrades in the explicit update workflow
    let missing_formulae = missing_packages(&brew, "--formula", formulae)?;
    if !missing_formulae.is_empty() {
        let mut args = install_args.to_vec();
        args.extend(missing_formulae);
        host::run("Homebrew formula install", "/usr/bin/env", args)?;
    }
    let missing_casks = missing_packages(&brew, "--cask", casks)?;
    if !missing_casks.is_empty() {
        let mut args = install_args.to_vec();
        args.push("--cask");
        args.push("--adopt");
        args.extend(missing_casks);
        host::run("Homebrew cask install", "/usr/bin/env", args)?;
    }
    Ok(())
}

fn missing_packages<'a>(brew: &str, flag: &str, packages: &'a [String]) -> Result<Vec<&'a str>> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    let output = host::run("Homebrew installed package query", brew, ["list", flag])?;
    let stdout = std::str::from_utf8(&output.stdout).context("Homebrew list returned non-UTF-8 output")?;
    let installed: Vec<&str> = stdout.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    let mut missing = Vec::new();
    for package in packages {
        let name = package.as_str();
        if !installed.iter().any(|inst| *inst == name || name.ends_with(&format!("/{inst}"))) {
            missing.push(name);
        }
    }
    Ok(missing)
}

pub(crate) fn executable_path(formula: &str, executable: &str) -> Result<String> {
    let brew = find_executable()?.context(HOMEBREW_UNAVAILABLE)?;
    let output = host::run("Homebrew formula prefix", &brew, ["--prefix", formula])?;
    let prefix = host::stdout_line(&output.stdout, "Homebrew formula prefix")?;
    let path = std::path::Path::new(prefix).join("bin").join(executable);
    host::path_program(&path, "Homebrew executable path")
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
    // Apple Silicon installs Homebrew outside PATH in some non-login environments
    for candidate in ["brew", "/opt/homebrew/bin/brew"] {
        if host::output(candidate, ["--version"]).is_ok_and(|output| output.status.success()) {
            return Ok(Some(candidate.to_owned()));
        }
    }
    Ok(None)
}
