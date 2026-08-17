use anyhow::Result;
use std::path::Path;

use super::{Host, privileged_file::write_atomic, systemd};
use crate::config::AptUpgradeCommand;

const AUTO_UPGRADES: &str = "/etc/apt/apt.conf.d/20auto-upgrades";

pub fn update(host: &Host) -> Result<()> {
    host.run("APT update", "sudo", ["apt-get", "update", "-qq"])?;
    Ok(())
}

pub(crate) fn set_unattended_upgrades(host: &Host, enabled: bool) -> Result<()> {
    let contents = if enabled {
        b"APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n".as_slice()
    } else {
        b"APT::Periodic::Update-Package-Lists \"0\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n".as_slice()
    };
    if enabled {
        install_packages(host, &["unattended-upgrades".into()])?;
        write_atomic(host, Path::new(AUTO_UPGRADES), contents, "unattended-upgrades periodic configuration")?;
        host.run(
            "unattended-upgrades service enablement",
            "sudo",
            ["systemctl", "enable", "--now", "unattended-upgrades.service"],
        )?;
    } else {
        write_atomic(host, Path::new(AUTO_UPGRADES), contents, "unattended-upgrades periodic configuration")?;
        if systemd::enabled_or_active(host, "unattended-upgrades.service")? {
            host.run(
                "unattended-upgrades service disablement",
                "sudo",
                ["systemctl", "disable", "--now", "unattended-upgrades.service"],
            )?;
        }
        purge(host, &["unattended-upgrades".into()])?;
    }
    Ok(())
}

pub fn install_packages(host: &Host, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let missing = missing_packages(host, packages)?;
    if missing.is_empty() {
        return Ok(());
    }
    install(host, "APT package install", missing)
}

fn missing_packages(host: &Host, packages: &[String]) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for package in packages {
        if !is_package_installed(host, package)? {
            missing.push(package.clone());
        }
    }
    Ok(missing)
}

fn installed_packages(host: &Host, packages: &[String]) -> Result<Vec<String>> {
    let mut installed = Vec::new();
    for package in packages {
        if is_package_installed(host, package)? {
            installed.push(package.clone());
        }
    }
    Ok(installed)
}

fn is_package_installed(host: &Host, package: &str) -> Result<bool> {
    let output = host.output("dpkg-query", ["-W", "-f=${db:Status-Status}\\n", "--", package])?;
    if !output.status.success() {
        if output.status.code() == Some(1) {
            return Ok(false);
        }
        anyhow::bail!("dpkg-query failed for {package:?}: {}", String::from_utf8_lossy(&output.stderr).trim());
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
        _ => anyhow::bail!("dpkg-query returned unrecognized package status for {package:?}"),
    }
}

fn install(host: &Host, label: &str, packages: Vec<String>) -> Result<()> {
    change_packages(host, label, "install", packages.into_iter().map(|package| format!("{package}+")))
}

fn change_packages(host: &Host, label: &str, command: &str, packages: impl IntoIterator<Item = String>) -> Result<()> {
    let mut args = vec![
        "DEBIAN_FRONTEND=noninteractive".to_owned(),
        "apt-get".to_owned(),
        command.to_owned(),
        "-y".into(),
        "-qq".into(),
        "--".into(),
    ];
    args.extend(packages);
    host.run(label, "sudo", args)?;
    Ok(())
}

pub fn purge(host: &Host, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let installed = installed_packages(host, packages)?;
    if installed.is_empty() {
        return Ok(());
    }
    change_packages(host, "APT package purge", "purge", installed)
}

pub fn upgrade(host: &Host, command: AptUpgradeCommand) -> Result<()> {
    let (label, apt_command) = match command {
        AptUpgradeCommand::Upgrade => ("APT upgrade", "upgrade"),
        AptUpgradeCommand::FullUpgrade => ("APT full-upgrade", "full-upgrade"),
    };
    host.run(label, "sudo", ["DEBIAN_FRONTEND=noninteractive", "apt-get", apt_command, "-y", "-qq"])?;
    if command == AptUpgradeCommand::FullUpgrade {
        host.run(
            "APT purge autoremove",
            "sudo",
            ["DEBIAN_FRONTEND=noninteractive", "apt-get", "autoremove", "--purge", "-y", "-qq"],
        )?;
    }
    Ok(())
}
