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

pub fn packages(host: &Host<'_>, packages: &[String]) -> Result<()> {
    let missing = select_packages(host, packages, false)?;
    if missing.is_empty() {
        return Ok(());
    }
    let mut args = vec![
        "apt-get".to_owned(),
        "install".into(),
        "-y".into(),
        "-qq".into(),
        "--".into(),
    ];
    args.extend(missing);
    host.require("APT package installation", "sudo", args)?;
    Ok(())
}

pub fn purge(host: &Host<'_>, packages: &[String]) -> Result<()> {
    let installed = select_packages(host, packages, true)?;
    if installed.is_empty() {
        return Ok(());
    }
    let mut args = vec![
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
                ["apt-get", "upgrade", "-y", "-qq"],
            )?;
        }
        AptUpgradePolicy::Full => {
            host.require(
                "APT full upgrade",
                "sudo",
                ["apt-get", "full-upgrade", "-y", "-qq"],
            )?;
            host.require(
                "APT purge autoremove",
                "sudo",
                ["apt-get", "autoremove", "--purge", "-y", "-qq"],
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
    let installed = installed_packages(&output.stdout)?;
    Ok(packages
        .iter()
        .filter(|package| installed.contains(package.as_str()) == select_installed)
        .cloned()
        .collect())
}

fn installed_packages(output: &[u8]) -> Result<BTreeSet<&str>> {
    let output =
        std::str::from_utf8(output).context("dpkg-query returned non-UTF-8 package state")?;
    let mut installed = BTreeSet::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let Some((package, status)) = line.split_once('\t') else {
            anyhow::bail!("dpkg-query returned malformed package state: {line:?}");
        };
        if package.is_empty() || status.len() < 2 || status.contains('\t') {
            anyhow::bail!("dpkg-query returned malformed package state: {line:?}");
        }
        if status.as_bytes()[1] == b'i' {
            installed.insert(package);
        }
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::installed_packages;

    #[test]
    fn status_distinguishes_installed_held_and_residual_entries() {
        let installed = installed_packages(
            b"installed\tii \nheld\thi \nresidual\trc \nreinstalled\trc \nreinstalled\tii \n",
        )
        .unwrap();
        assert!(installed.contains("installed"));
        assert!(installed.contains("held"));
        assert!(installed.contains("reinstalled"));
        assert!(!installed.contains("residual"));
    }
}
