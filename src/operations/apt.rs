use anyhow::Result;
use std::path::Path;

use super::{Host, privileged_file::write_atomic};
use crate::config::AptUpgradeCommand;

const AUTO_UPGRADES: &str = "/etc/apt/apt.conf.d/20auto-upgrades";

pub fn update(host: &Host) -> Result<()> {
    host.require("APT update", "sudo", ["apt-get", "update", "-qq"])?;
    Ok(())
}

pub(crate) fn unattended_upgrades(host: &Host, enabled: bool) -> Result<()> {
    let contents = if enabled {
        b"APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n".as_slice()
    } else {
        b"APT::Periodic::Update-Package-Lists \"0\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n".as_slice()
    };
    if enabled {
        packages(host, &["unattended-upgrades".into()])?;
        write_atomic(host, Path::new(AUTO_UPGRADES), contents, "unattended-upgrades periodic configuration")?;
        host.require(
            "unattended-upgrades service enablement",
            "sudo",
            ["systemctl", "enable", "--now", "unattended-upgrades.service"],
        )?;
    } else {
        write_atomic(host, Path::new(AUTO_UPGRADES), contents, "unattended-upgrades periodic configuration")?;
        let is_enabled = systemd_state(host, "is-enabled", "unattended-upgrades.service")?;
        let is_active = systemd_state(host, "is-active", "unattended-upgrades.service")?;
        if is_enabled || is_active {
            host.require(
                "unattended-upgrades service disablement",
                "sudo",
                ["systemctl", "disable", "--now", "unattended-upgrades.service"],
            )?;
        }
        purge(host, &["unattended-upgrades".into()])?;
    }
    Ok(())
}

fn systemd_state(host: &Host, query: &str, unit: &str) -> Result<bool> {
    Ok(host.run("systemctl", [query, unit])?.status.success())
}

pub fn update_and_install(host: &Host, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        anyhow::bail!("APT update-and-install package sequence must not be empty");
    }
    let missing = packages_with_state(host, packages, false)?;
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
    let missing = packages_with_state(host, packages, false)?;
    if missing.is_empty() {
        return Ok(());
    }
    install(host, "APT package installation", missing)
}

pub fn purge_then_install(host: &Host, purge_packages: &[String], install_packages: &[String]) -> Result<()> {
    purge(host, purge_packages)?;
    self::packages(host, install_packages)
}

fn packages_with_state(host: &Host, packages: &[String], installed: bool) -> Result<Vec<String>> {
    let mut matching = Vec::new();
    for package in packages {
        if package_is_installed(host, package)? == installed {
            matching.push(package.clone());
        }
    }
    Ok(matching)
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
    change_packages(host, operation, "install", packages.into_iter().map(|package| format!("{package}+")))
}

fn change_packages(
    host: &Host,
    operation: &str,
    command: &str,
    packages: impl IntoIterator<Item = String>,
) -> Result<()> {
    let mut args = vec![
        "DEBIAN_FRONTEND=noninteractive".to_owned(),
        "apt-get".to_owned(),
        command.to_owned(),
        "-y".into(),
        "-qq".into(),
        "--".into(),
    ];
    args.extend(packages);
    host.require(operation, "sudo", args)?;
    Ok(())
}

pub fn purge(host: &Host, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let installed = packages_with_state(host, packages, true)?;
    if installed.is_empty() {
        return Ok(());
    }
    change_packages(host, "APT package purge", "purge", installed)
}

pub fn upgrade(host: &Host, command: AptUpgradeCommand) -> Result<()> {
    let (operation, apt_command) = match command {
        AptUpgradeCommand::Upgrade => ("APT upgrade", "upgrade"),
        AptUpgradeCommand::FullUpgrade => ("APT full-upgrade", "full-upgrade"),
    };
    host.require(operation, "sudo", ["DEBIAN_FRONTEND=noninteractive", "apt-get", apt_command, "-y", "-qq", "--"])?;
    if command == AptUpgradeCommand::FullUpgrade {
        host.require(
            "APT purge autoremove",
            "sudo",
            ["DEBIAN_FRONTEND=noninteractive", "apt-get", "autoremove", "--purge", "-y", "-qq", "--"],
        )?;
    }
    Ok(())
}
