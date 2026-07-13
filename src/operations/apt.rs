use super::Host;
use anyhow::{Context, Result};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AptUpgradePolicy {
    Standard,
    Full,
}

pub fn metadata_refresh(host: &Host<'_>) -> Result<()> {
    host.require("APT metadata refresh", "sudo", ["apt-get", "update", "-qq"])?;
    Ok(())
}

pub fn bootstrap_packages(host: &Host<'_>, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        anyhow::bail!("APT bootstrap package sequence must not be empty");
    }
    let missing = select_packages(host, packages, false)?;
    if missing.is_empty() {
        return Ok(());
    }
    host.require(
        "APT bootstrap metadata refresh",
        "sudo",
        ["apt-get", "update", "-qq"],
    )?;
    install(host, "APT bootstrap package installation", missing)
}

pub fn packages(host: &Host<'_>, packages: &[String]) -> Result<()> {
    let missing = select_packages(host, packages, false)?;
    if missing.is_empty() {
        return Ok(());
    }
    install(host, "APT package installation", missing)
}

fn install(host: &Host<'_>, operation: &str, packages: Vec<String>) -> Result<()> {
    let mut args = vec![
        "DEBIAN_FRONTEND=noninteractive".to_owned(),
        "apt-get".to_owned(),
        "install".into(),
        "-y".into(),
        "-qq".into(),
        "--".into(),
    ];
    args.extend(packages);
    host.require(operation, "sudo", args)?;
    Ok(())
}

pub fn purge(host: &Host<'_>, packages: &[String]) -> Result<()> {
    let installed = select_packages(host, packages, true)?;
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

pub fn upgrade(host: &Host<'_>, policy: AptUpgradePolicy) -> Result<()> {
    match policy {
        AptUpgradePolicy::Standard => {
            host.require(
                "APT standard upgrade",
                "sudo",
                [
                    "DEBIAN_FRONTEND=noninteractive",
                    "apt-get",
                    "upgrade",
                    "-y",
                    "-qq",
                    "--",
                ],
            )?;
        }
        AptUpgradePolicy::Full => {
            host.require(
                "APT full upgrade",
                "sudo",
                [
                    "DEBIAN_FRONTEND=noninteractive",
                    "apt-get",
                    "full-upgrade",
                    "-y",
                    "-qq",
                    "--",
                ],
            )?;
            host.require(
                "APT purge autoremove",
                "sudo",
                [
                    "DEBIAN_FRONTEND=noninteractive",
                    "apt-get",
                    "autoremove",
                    "--purge",
                    "-y",
                    "-qq",
                    "--",
                ],
            )?;
        }
    }
    Ok(())
}

fn select_packages(
    host: &Host<'_>,
    packages: &[String],
    select_installed: bool,
) -> Result<Vec<String>> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    let mut requested = BTreeSet::new();
    for package in packages {
        validate_package_name(package)?;
        if !requested.insert(package.as_str()) {
            anyhow::bail!("APT package state query has duplicate requested package: {package:?}");
        }
    }
    let mut args = vec![
        "-W".to_owned(),
        "-f=${Package}\\t${db:Status-Abbrev}\\n".into(),
        "--".into(),
    ];
    args.extend(packages.iter().cloned());
    let output = host.run("dpkg-query", args)?;
    if !output.status.success() && output.status.code() != Some(1) {
        anyhow::bail!(
            "APT package state query: dpkg-query failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let installed = installed_packages(&output.stdout, &requested, output.status.success())?;
    Ok(packages
        .iter()
        .filter(|package| installed.contains(package.as_str()) == select_installed)
        .cloned()
        .collect())
}

fn validate_package_name(package: &str) -> Result<()> {
    let mut bytes = package.bytes();
    let valid = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'.' | b'-')
        });
    if !valid {
        anyhow::bail!("invalid canonical Debian package name: {package:?}");
    }
    Ok(())
}

fn installed_packages<'a>(
    output: &'a [u8],
    requested: &BTreeSet<&str>,
    require_complete: bool,
) -> Result<BTreeSet<&'a str>> {
    let output =
        std::str::from_utf8(output).context("dpkg-query returned non-UTF-8 package state")?;
    let mut installed = BTreeSet::new();
    let mut returned = BTreeSet::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let Some((package, status)) = line.split_once('\t') else {
            anyhow::bail!("dpkg-query returned malformed package state: {line:?}");
        };
        let status = status.as_bytes();
        if package.is_empty()
            || status.len() != 3
            || !matches!(status[0], b'u' | b'i' | b'h' | b'r' | b'p')
            || !matches!(
                status[1],
                b'n' | b'c' | b'H' | b'U' | b'F' | b'W' | b't' | b'i'
            )
            || !matches!(status[2], b' ' | b'R')
        {
            anyhow::bail!("dpkg-query returned malformed package state: {line:?}");
        }
        if !requested.contains(package) {
            anyhow::bail!("dpkg-query returned unrequested package record: {package:?}");
        }
        if !returned.insert(package) {
            anyhow::bail!("dpkg-query returned duplicate package record: {package:?}");
        }
        if status[1] == b'i' {
            installed.insert(package);
        }
    }
    if require_complete && returned.len() != requested.len() {
        let missing = requested
            .iter()
            .filter(|package| !returned.contains(**package))
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "dpkg-query returned incomplete package state; missing records for: {missing}"
        );
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::installed_packages;
    use std::collections::BTreeSet;

    #[test]
    fn status_accepts_documented_bytes_and_classifies_only_installed_status() {
        let desired = [b'u', b'i', b'h', b'r', b'p'];
        let states = [b'n', b'c', b'H', b'U', b'F', b'W', b't', b'i'];
        let errors = [b' ', b'R'];
        let mut output = Vec::new();
        let mut packages = Vec::new();
        let mut expected = BTreeSet::new();
        for desired in desired {
            for state in states {
                for error in errors {
                    let package = format!("package-{}", packages.len());
                    output.extend_from_slice(package.as_bytes());
                    output.extend_from_slice(&[b'\t', desired, state, error, b'\n']);
                    if state == b'i' {
                        expected.insert(package.clone());
                    }
                    packages.push(package);
                }
            }
        }
        let requested = packages.iter().map(String::as_str).collect();
        let installed = installed_packages(&output, &requested, true).unwrap();
        assert_eq!(installed, expected.iter().map(String::as_str).collect());
    }

    #[test]
    fn status_distinguishes_installed_held_residual_and_reinstall_entries() {
        let requested = ["installed", "held", "residual", "reinstall"]
            .into_iter()
            .collect();
        let installed = installed_packages(
            b"installed\tii \nheld\thi \nresidual\trc \nreinstall\tiiR\n",
            &requested,
            true,
        )
        .unwrap();
        assert!(installed.contains("installed"));
        assert!(installed.contains("held"));
        assert!(installed.contains("reinstall"));
        assert!(!installed.contains("residual"));
    }
}
