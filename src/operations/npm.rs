use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};
use std::{
    collections::BTreeSet,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::Host;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpmPackageMode {
    EnsurePresent,
    UpdateCurrent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpmPackageOperation {
    packages: Vec<String>,
    mode: NpmPackageMode,
}

impl NpmPackageOperation {
    pub fn new(packages: Vec<String>, mode: NpmPackageMode) -> Result<Self> {
        validate_packages(&packages)?;
        Ok(Self { packages, mode })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        std::iter::once("npm-package-set".into())
            .chain(std::iter::once(
                match self.mode {
                    NpmPackageMode::EnsurePresent => "ensure-present",
                    NpmPackageMode::UpdateCurrent => "update-current",
                }
                .into(),
            ))
            .chain(self.packages.iter().cloned())
            .collect()
    }
}

pub(crate) fn execute(host: &Host<'_>, operation: &NpmPackageOperation) -> Result<()> {
    validate_packages(&operation.packages).context("validate npm package operation")?;
    let fnm = resolve_fnm(host)?;
    let version = selected_version(host, &fnm)?;
    let installed = inspect_installed(host, &fnm, &version)?;
    let selected = match operation.mode {
        NpmPackageMode::EnsurePresent => operation
            .packages
            .iter()
            .filter(|package| !installed.contains(package.as_str()))
            .cloned()
            .collect::<Vec<_>>(),
        NpmPackageMode::UpdateCurrent => operation.packages.clone(),
    };
    if selected.is_empty() {
        return Ok(());
    }

    let mut npm_args = vec!["install".to_owned(), "--global".into(), "--".into()];
    npm_args.extend(selected);
    run_npm_required(host, &fnm, &version, "npm package mutation", npm_args)?;

    let installed = inspect_installed(host, &fnm, &version)?;
    require_packages(&operation.packages, &installed)
}

fn resolve_fnm(host: &Host<'_>) -> Result<String> {
    let data_home = host
        .value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"));
    if !data_home.is_absolute() {
        bail!("npm package operation requires an absolute managed FNM data directory");
    }
    let managed = data_home.join("fnm/fnm");
    if executable_file(&managed) {
        return managed
            .to_str()
            .map(str::to_owned)
            .context("managed fnm executable path is not UTF-8");
    }
    bail!("npm package operation: managed fnm is unavailable after bootstrap")
}

fn selected_version(host: &Host<'_>, fnm: &str) -> Result<String> {
    let output = host.require("fnm default Node query", fnm, ["default"])?;
    let output = std::str::from_utf8(&output.stdout)
        .context("fnm returned non-UTF-8 default Node version")?;
    let version = output.strip_suffix('\n').unwrap_or(output);
    if version.contains(['\n', '\r']) || !valid_node_version(version) {
        bail!("fnm returned invalid default Node version: {version:?}");
    }
    Ok(version.to_owned())
}

fn inspect_installed(host: &Host<'_>, fnm: &str, version: &str) -> Result<BTreeSet<String>> {
    let output = run_npm_required(
        host,
        fnm,
        version,
        "npm global package query",
        ["list", "--global", "--depth=0", "--json"],
    )?;
    installed_packages(&output.stdout)
}

fn run_npm_required<I, S>(
    host: &Host<'_>,
    fnm: &str,
    version: &str,
    operation: &str,
    npm_args: I,
) -> Result<std::process::Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = vec![
        "exec".to_owned(),
        "--using".into(),
        version.to_owned(),
        "--".into(),
        "npm".into(),
    ];
    args.extend(npm_args.into_iter().map(|arg| arg.as_ref().to_owned()));
    host.require(operation, fnm, args)
}

fn installed_packages(output: &[u8]) -> Result<BTreeSet<String>> {
    let output =
        std::str::from_utf8(output).context("npm returned non-UTF-8 global package state")?;
    let root: Value = serde_json::from_str(output).context("npm returned malformed JSON state")?;
    let root = root
        .as_object()
        .context("npm global package state must be a JSON object")?;
    reject_problem_state(root, "npm global package state")?;
    if root.contains_key("error") {
        bail!("npm global package state reported an error");
    }
    let dependencies = match root.get("dependencies") {
        Some(dependencies) => dependencies
            .as_object()
            .context("npm global package state dependencies must be a JSON object")?,
        None => return Ok(BTreeSet::new()),
    };
    let mut installed = BTreeSet::new();
    for (package, metadata) in dependencies {
        validate_package(package).map_err(|_| {
            anyhow::anyhow!("npm returned invalid global package name: {package:?}")
        })?;
        let metadata = metadata.as_object().with_context(|| {
            format!("npm global package metadata for {package:?} must be a JSON object")
        })?;
        reject_problem_state(metadata, &format!("npm global package {package:?}"))?;
        if metadata.contains_key("error") {
            bail!("npm global package {package:?} reported an error");
        }
        let version = metadata
            .get("version")
            .and_then(Value::as_str)
            .with_context(|| {
                format!("npm global package {package:?} must report a string version")
            })?;
        if version.is_empty() || version.chars().any(char::is_control) {
            bail!("npm global package {package:?} reported an invalid version");
        }
        for flag in ["invalid", "missing"] {
            if metadata
                .get(flag)
                .is_some_and(|value| value != &Value::Bool(false))
            {
                bail!("npm global package {package:?} reported {flag} state");
            }
        }
        installed.insert(package.clone());
    }
    Ok(installed)
}

fn reject_problem_state(object: &Map<String, Value>, description: &str) -> Result<()> {
    if let Some(problems) = object.get("problems") {
        let problems = problems
            .as_array()
            .with_context(|| format!("{description} problems must be a JSON array"))?;
        if !problems.is_empty() {
            bail!("{description} reported problems");
        }
    }
    Ok(())
}

fn require_packages(packages: &[String], installed: &BTreeSet<String>) -> Result<()> {
    let missing = packages
        .iter()
        .filter(|package| !installed.contains(package.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "npm package mutation did not install configured packages: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

fn validate_packages(packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        bail!("npm package sequence must not be empty");
    }
    let mut seen = BTreeSet::new();
    for package in packages {
        validate_package(package)?;
        if !seen.insert(package.as_str()) {
            bail!("duplicate npm package name: {package:?}");
        }
    }
    Ok(())
}

fn validate_package(package: &str) -> Result<()> {
    let valid_part = |part: &str| {
        let mut bytes = part.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
    };
    let valid = if let Some(scoped) = package.strip_prefix('@') {
        let mut parts = scoped.split('/');
        valid_part(parts.next().unwrap_or_default())
            && valid_part(parts.next().unwrap_or_default())
            && parts.next().is_none()
    } else {
        !package.contains('/') && valid_part(package)
    };
    if !valid {
        bail!("invalid unversioned lowercase npm package name: {package:?}");
    }
    Ok(())
}

fn valid_node_version(version: &str) -> bool {
    let Some(version) = version.strip_prefix('v') else {
        return false;
    };
    let parts = version.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        })
}

fn executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::installed_packages;

    #[test]
    fn npm_state_accepts_clean_scoped_and_unscoped_dependencies() {
        let installed = installed_packages(
            br#"{"dependencies":{"opencode-ai":{"version":"1.0.0"},"@scope/tool":{"version":"2.0.0"}},"problems":[]}"#,
        )
        .unwrap();
        assert_eq!(
            installed.into_iter().collect::<Vec<_>>(),
            ["@scope/tool", "opencode-ai"]
        );
    }

    #[test]
    fn npm_state_accepts_real_empty_global_root() {
        assert!(installed_packages(br#"{"name":"lib"}"#).unwrap().is_empty());
    }

    #[test]
    fn npm_state_rejects_reported_and_malformed_dependency_state() {
        for output in [
            br#"[]"#.as_slice(),
            br#"{"dependencies":null}"#.as_slice(),
            br#"{"dependencies":[]}"#.as_slice(),
            br#"{"dependencies":{"BAD":{}}}"#.as_slice(),
            br#"{"dependencies":{"tool":{}}}"#.as_slice(),
            br#"{"dependencies":{"tool":null}}"#.as_slice(),
            br#"{"dependencies":{"tool":{"version":"1.0.0","problems":["broken"]}}}"#.as_slice(),
            br#"{"dependencies":{"tool":{"version":"1.0.0","invalid":true}}}"#.as_slice(),
            br#"{"dependencies":{"tool":{"version":"1.0.0","missing":true}}}"#.as_slice(),
            br#"{"dependencies":{"tool":{"version":"1.0.0","error":{"code":"EFAIL"}}}}"#.as_slice(),
            br#"{"dependencies":{"tool":{"version":"1.0.0","error":null}}}"#.as_slice(),
            br#"{"dependencies":{"tool":{"version":"1.0.0","error":false}}}"#.as_slice(),
            br#"{"dependencies":{"tool":{"version":"1.0.0","error":""}}}"#.as_slice(),
            br#"{"dependencies":{},"problems":["broken"]}"#.as_slice(),
            br#"{"dependencies":{},"error":{"code":"EFAIL"}}"#.as_slice(),
        ] {
            assert!(installed_packages(output).is_err());
        }
    }
}
