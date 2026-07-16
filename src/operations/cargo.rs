use anyhow::{bail, Context, Result};
use semver::Version;
use std::{
    collections::BTreeSet,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::Host;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CargoPackageMode {
    EnsurePresent,
    UpdateCurrent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoPackageOperation {
    packages: Vec<String>,
    mode: CargoPackageMode,
}

impl CargoPackageOperation {
    pub fn new(packages: Vec<String>, mode: CargoPackageMode) -> Result<Self> {
        validate_packages(&packages)?;
        Ok(Self { packages, mode })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        std::iter::once("cargo-package-set".into())
            .chain(std::iter::once(
                match self.mode {
                    CargoPackageMode::EnsurePresent => "ensure-present",
                    CargoPackageMode::UpdateCurrent => "update-current",
                }
                .into(),
            ))
            .chain(self.packages.iter().cloned())
            .collect()
    }
}

pub(crate) fn execute(host: &Host<'_>, operation: &CargoPackageOperation) -> Result<()> {
    validate_packages(&operation.packages).context("validate Cargo package operation")?;
    let cargo_home = host
        .value("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".cargo"));
    if !cargo_home.is_absolute() {
        bail!("Cargo package operation requires an absolute CARGO_HOME");
    }
    let cargo = resolve_cargo(host, &cargo_home)?;
    let installed = inspect_installed(host, &cargo)?;
    let selected = match operation.mode {
        CargoPackageMode::EnsurePresent => operation
            .packages
            .iter()
            .filter(|package| !installed.contains(package.as_str()))
            .cloned()
            .collect::<Vec<_>>(),
        CargoPackageMode::UpdateCurrent => operation.packages.clone(),
    };
    if selected.is_empty() {
        return Ok(());
    }

    let binstall = resolve_binstall(&cargo_home)?.context(
        "Cargo package operation: managed cargo-binstall is unavailable after bootstrap",
    )?;
    let mut args = vec!["--no-confirm".to_owned()];
    if operation.mode == CargoPackageMode::UpdateCurrent {
        args.push("--force".into());
    }
    args.extend(selected);
    host.require("Cargo package mutation", &binstall, args)?;

    let installed = inspect_installed(host, &cargo)?;
    require_packages(&operation.packages, &installed)
}

fn resolve_cargo(_host: &Host<'_>, cargo_home: &Path) -> Result<String> {
    let managed = cargo_home.join("bin/cargo");
    if executable_file(&managed) {
        return path_program(&managed, "Cargo executable path");
    }
    bail!("Cargo package operation: managed Cargo is unavailable after Rust bootstrap")
}

fn resolve_binstall(cargo_home: &Path) -> Result<Option<String>> {
    let managed = cargo_home.join("bin/cargo-binstall");
    if executable_file(&managed) {
        return path_program(&managed, "cargo-binstall executable path").map(Some);
    }
    Ok(None)
}

fn inspect_installed(host: &Host<'_>, cargo: &str) -> Result<BTreeSet<String>> {
    let output = host.require(
        "Cargo installed package query",
        cargo,
        ["install", "--list"],
    )?;
    installed_packages(&output.stdout)
}

fn installed_packages(output: &[u8]) -> Result<BTreeSet<String>> {
    let output =
        std::str::from_utf8(output).context("cargo returned non-UTF-8 installed package state")?;
    let mut installed = BTreeSet::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((package, version_and_source)) = line.split_once(" v") else {
            bail!("cargo returned malformed installed package state: {line:?}");
        };
        validate_package(package).map_err(|_| {
            anyhow::anyhow!("cargo returned malformed installed package state: {line:?}")
        })?;
        let Some(record) = version_and_source.strip_suffix(':') else {
            bail!("cargo returned malformed installed package state: {line:?}");
        };
        let (version, source) = record
            .split_once(" (")
            .map_or((record, None), |parts| (parts.0, parts.1.strip_suffix(')')));
        if Version::parse(version).is_err()
            || record.contains(" (") && source.is_none()
            || source.is_some_and(|source| !valid_display_source(source))
        {
            bail!("cargo returned malformed installed package state: {line:?}");
        }
        if source.is_none() && !installed.insert(package.to_owned()) {
            bail!("cargo returned duplicate installed registry package: {package:?}");
        }
    }
    Ok(installed)
}

fn valid_display_source(source: &str) -> bool {
    if source.is_empty() || source.chars().any(char::is_control) {
        return false;
    }
    let mut depth = 0_u32;
    for character in source.chars() {
        match character {
            '(' => depth += 1,
            ')' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            _ => {}
        }
    }
    depth == 0
}

fn require_packages(packages: &[String], installed: &BTreeSet<String>) -> Result<()> {
    let missing = packages
        .iter()
        .filter(|package| !installed.contains(package.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "Cargo package mutation did not install configured packages: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

fn validate_packages(packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        bail!("Cargo package sequence must not be empty");
    }
    let mut seen = BTreeSet::new();
    for package in packages {
        validate_package(package)?;
        if !seen.insert(package.as_str()) {
            bail!("duplicate Cargo package name: {package:?}");
        }
    }
    Ok(())
}

fn validate_package(package: &str) -> Result<()> {
    let mut bytes = package.bytes();
    let valid = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if !valid {
        bail!("invalid unversioned Cargo package name: {package:?}");
    }
    Ok(())
}

fn executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn path_program(path: &Path, description: &str) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .with_context(|| format!("{description} is not UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::installed_packages;

    #[test]
    fn cargo_state_accepts_registry_records_and_ignores_binaries_and_other_sources() {
        let output = b"bat v0.25.0:\n    bat\npath-probe v1.2.3 (/tmp/hermes-cargo-probe):\n    path-probe\ngit-probe v2.3.4 (https://github.com/example/repo?rev=main):\n    git-probe\nnested-probe v3.4.5 (/tmp/probe (safe)):\n    nested-probe\nprerelease-probe v1.2.3-alpha.1+build.7 (/tmp/prerelease-probe):\n    prerelease-probe\n";
        let installed = installed_packages(output).unwrap();
        assert_eq!(installed.into_iter().collect::<Vec<_>>(), ["bat"]);
    }

    #[test]
    fn cargo_state_rejects_malformed_and_duplicate_registry_records() {
        for output in [
            b"bat\n".as_slice(),
            b"bat v01.2.3:\n".as_slice(),
            b"bat v1.2.3-01:\n".as_slice(),
            b"bat v1.2.3\n".as_slice(),
            b"bat v1.2.3 ():\n".as_slice(),
            b"bat v1.2.3 (/tmp/probe (broken):\n".as_slice(),
            b"bat v1.2.3 (/tmp/probe ) broken ():\n".as_slice(),
            b"bat v1.2.3 (/tmp/probe\tbroken):\n".as_slice(),
            b"bat v1.2.3:\nbat v1.2.4:\n".as_slice(),
        ] {
            assert!(installed_packages(output).is_err());
        }
    }
}
