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
    let mut args = vec!["apt-get".to_owned(), "install".into(), "-qq".into()];
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
    ];
    args.extend(packages.iter().cloned());
    let output = host.run("dpkg-query", args)?;
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
    Ok(output
        .lines()
        .filter_map(|line| {
            let (package, status) = line.split_once('\t')?;
            (status.as_bytes().get(1) == Some(&b'i')).then_some(package)
        })
        .collect())
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
