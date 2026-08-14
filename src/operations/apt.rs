use super::Host;
use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AptUpgradeCommand {
    Upgrade,
    FullUpgrade,
}

pub fn update(host: &Host) -> Result<()> {
    host.require("APT update", "sudo", ["apt-get", "update", "-qq"])?;
    Ok(())
}

pub fn update_and_install(host: &Host, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        anyhow::bail!("APT update-and-install package sequence must not be empty");
    }
    let missing = missing_packages(host, packages)?;
    if missing.is_empty() {
        return Ok(());
    }
    host.require("APT update before install", "sudo", ["apt-get", "update", "-qq"])?;
    install(host, "APT package install", missing)
}

pub fn packages(host: &Host, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let missing = missing_packages(host, packages)?;
    if missing.is_empty() {
        return Ok(());
    }
    install(host, "APT package installation", missing)
}

pub fn purge_then_install(host: &Host, purge_packages: &[String], install_packages: &[String]) -> Result<()> {
    purge(host, purge_packages)?;
    self::packages(host, install_packages)
}

fn missing_packages(host: &Host, packages: &[String]) -> Result<Vec<String>> {
    packages
        .iter()
        .filter_map(|package| match package_is_installed(host, package) {
            Ok(true) => None,
            Ok(false) => Some(Ok(package.clone())),
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn package_is_installed(host: &Host, package: &str) -> Result<bool> {
    let output = host.run("dpkg-query", ["-W", "-f=${db:Status-Status}\\n", "--", package])?;
    if !output.status.success() {
        if output.status.code() == Some(1) {
            return Ok(false);
        }
        anyhow::bail!(
            "APT package inspection failed for {package:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    match output.stdout.as_slice() {
        b"installed\n" => Ok(true),
        b"not-installed\n"
        | b"config-files\n"
        | b"half-installed\n"
        | b"unpacked\n"
        | b"half-configured\n"
        | b"triggers-awaited\n"
        | b"triggers-pending\n" => Ok(false),
        _ => anyhow::bail!("APT package inspection returned malformed state for {package:?}"),
    }
}

fn install(host: &Host, operation: &str, packages: Vec<String>) -> Result<()> {
    let mut args = vec![
        "DEBIAN_FRONTEND=noninteractive".to_owned(),
        "apt-get".to_owned(),
        "install".into(),
        "-y".into(),
        "-qq".into(),
        "--".into(),
    ];
    args.extend(packages.into_iter().map(|package| format!("{package}+")));
    host.require(operation, "sudo", args)?;
    Ok(())
}

pub fn purge(host: &Host, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let installed = packages
        .iter()
        .map(|package| Ok((package, package_is_installed(host, package)?)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, installed)| *installed)
        .map(|(package, _)| package.clone())
        .collect::<Vec<_>>();
    if installed.is_empty() {
        return Ok(());
    }
    let mut args = vec![
        "DEBIAN_FRONTEND=noninteractive".to_owned(),
        "apt-get".to_owned(),
        "purge".into(),
        "-y".into(),
        "-qq".into(),
        "--".into(),
    ];
    args.extend(installed);
    host.require("APT package purge", "sudo", args)?;
    Ok(())
}

pub fn upgrade(host: &Host, command: AptUpgradeCommand) -> Result<()> {
    match command {
        AptUpgradeCommand::Upgrade => {
            host.require(
                "APT upgrade",
                "sudo",
                ["DEBIAN_FRONTEND=noninteractive", "apt-get", "upgrade", "-y", "-qq", "--"],
            )?;
        }
        AptUpgradeCommand::FullUpgrade => {
            host.require(
                "APT full-upgrade",
                "sudo",
                ["DEBIAN_FRONTEND=noninteractive", "apt-get", "full-upgrade", "-y", "-qq", "--"],
            )?;
            host.require(
                "APT purge autoremove",
                "sudo",
                ["DEBIAN_FRONTEND=noninteractive", "apt-get", "autoremove", "--purge", "-y", "-qq", "--"],
            )?;
        }
    }
    Ok(())
}
