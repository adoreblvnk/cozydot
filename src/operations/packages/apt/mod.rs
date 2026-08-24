use anyhow::Result;
use std::path::Path;

use crate::config::AptUpgrade;
use crate::operations::host::{self, privileged_file::write_atomic, systemd};

pub(crate) mod repo;

const AUTO_UPGRADES: &str = "/etc/apt/apt.conf.d/20auto-upgrades";

pub fn update() -> Result<()> {
    host::run("APT update", "sudo", ["apt-get", "update", "-qq"])?;
    Ok(())
}

pub(crate) fn set_unattended_upgrades(enabled: bool) -> Result<()> {
    let contents = if enabled {
        b"APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n".as_slice()
    } else {
        b"APT::Periodic::Update-Package-Lists \"0\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n".as_slice()
    };
    if enabled {
        install(&["unattended-upgrades".into()])?;
        write_atomic(Path::new(AUTO_UPGRADES), contents, "unattended-upgrades periodic configuration")?;
        let args = ["systemctl", "enable", "--now", "unattended-upgrades.service"];
        host::run("unattended-upgrades service enablement", "sudo", args)?;
    } else {
        write_atomic(Path::new(AUTO_UPGRADES), contents, "unattended-upgrades periodic configuration")?;
        if systemd::is_enabled_or_active("unattended-upgrades.service")? {
            let args = ["systemctl", "disable", "--now", "unattended-upgrades.service"];
            host::run("unattended-upgrades service disablement", "sudo", args)?;
        }
        purge(&["unattended-upgrades".into()])?;
    }
    Ok(())
}

pub fn install(packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let packages = packages.iter().map(|package| format!("{package}+"));
    change_packages("APT package install", &["install", "--no-upgrade"], packages)
}

pub fn purge(packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }
    let mut installed = Vec::new();
    for package in packages {
        if is_installed(package)? {
            installed.push(package.clone());
        }
    }
    if installed.is_empty() {
        return Ok(());
    }
    change_packages("APT package purge", &["purge"], installed)
}

pub fn upgrade(command: AptUpgrade) -> Result<()> {
    let (label, apt_command) = match command {
        AptUpgrade::Upgrade => ("APT upgrade", "upgrade"),
        AptUpgrade::FullUpgrade => ("APT full-upgrade", "full-upgrade"),
    };
    host::run(label, "sudo", ["DEBIAN_FRONTEND=noninteractive", "apt-get", apt_command, "-y", "-qq"])?;
    if command == AptUpgrade::FullUpgrade {
        let args = ["DEBIAN_FRONTEND=noninteractive", "apt-get", "autoremove", "--purge", "-y", "-qq"];
        host::run("APT purge autoremove", "sudo", args)?;
    }
    Ok(())
}

fn change_packages(label: &str, command: &[&str], packages: impl IntoIterator<Item = String>) -> Result<()> {
    let mut args = vec!["DEBIAN_FRONTEND=noninteractive".to_owned(), "apt-get".to_owned()];
    args.extend(command.iter().map(|value| (*value).to_owned()));
    args.extend(["-y".into(), "-qq".into(), "--".into()]);
    args.extend(packages);
    host::run(label, "sudo", args)?;
    Ok(())
}

fn is_installed(package: &str) -> Result<bool> {
    let output = host::output("dpkg-query", ["-W", "-f=${db:Status-Status}\\n", "--", package])?;
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
