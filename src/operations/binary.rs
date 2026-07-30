use super::{Host, TempPath};
use crate::{config::BinaryFormat, platform::Architecture};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BinarySourceOperation {
    GithubLatest { repository: String, selector: String },
    Url { url: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryPackageOperation {
    name: String,
    format: BinaryFormat,
    architecture: Architecture,
    source: BinarySourceOperation,
}

impl BinaryPackageOperation {
    pub fn new(name: String, format: BinaryFormat, architecture: Architecture, source: BinarySourceOperation) -> Self {
        Self { name, format, architecture, source }
    }
}

pub(crate) fn execute(host: &Host, operation: &BinaryPackageOperation) -> Result<()> {
    if installed(host, operation) {
        return Ok(());
    }
    let url = resolve(host, operation)?;
    let temporary = download(host, operation, &url)?;
    match operation.format {
        BinaryFormat::Deb => install_deb(host, temporary),
        BinaryFormat::Appimage => install_appimage(host, operation, temporary),
    }
}

fn installed(host: &Host, operation: &BinaryPackageOperation) -> bool {
    match operation.format {
        BinaryFormat::Deb => executable_on_path(host, &operation.name),
        BinaryFormat::Appimage => appimage_destination(host, operation).exists(),
    }
}

fn executable_on_path(host: &Host, name: &str) -> bool {
    host.value("PATH")
        .and_then(|path| {
            std::env::split_paths(&path).find(|dir| {
                fs::metadata(dir.join(name))
                    .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            })
        })
        .is_some()
}

fn appimage_destination(host: &Host, operation: &BinaryPackageOperation) -> PathBuf {
    host.home().join("Applications").join(format!("{}.AppImage", operation.name))
}

fn resolve(host: &Host, operation: &BinaryPackageOperation) -> Result<String> {
    match &operation.source {
        BinarySourceOperation::Url { url } => Ok(url.clone()),
        BinarySourceOperation::GithubLatest { repository, selector } => {
            let endpoint = format!("https://api.github.com/repos/{repository}/releases/latest");
            let output = host.require(
                "resolve binary package release",
                "curl",
                [
                    "--proto",
                    "=https",
                    "--location",
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--retry",
                    "3",
                    "--retry-all-errors",
                    "--header",
                    GITHUB_ACCEPT,
                    "--header",
                    GITHUB_API_VERSION,
                    "--header",
                    USER_AGENT,
                    &endpoint,
                ],
            )?;
            select_asset(&output.stdout, selector, operation)
        }
    }
}

fn select_asset(input: &[u8], selector: &str, operation: &BinaryPackageOperation) -> Result<String> {
    let release: Value = serde_json::from_slice(input).context("parse GitHub release JSON")?;
    let assets = release.get("assets").and_then(Value::as_array).context("GitHub release assets must be an array")?;
    let pattern = Regex::new(selector).context("compile binary asset regex")?;
    let matches = assets
        .iter()
        .filter(|asset| asset.get("name").and_then(Value::as_str).is_some_and(|name| pattern.is_match(name)))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!(
            "binary package {:?} ({}) selector matched {} assets",
            operation.name,
            operation.architecture.canonical(),
            matches.len()
        );
    }
    matches[0]
        .get("browser_download_url")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("selected GitHub release asset must have a browser_download_url")
}

fn download(host: &Host, operation: &BinaryPackageOperation, url: &str) -> Result<TempPath> {
    let temporary = match operation.format {
        BinaryFormat::Deb => TempPath::new_with_suffix(host, &operation.name, ".deb")?,
        BinaryFormat::Appimage => {
            let applications = host.home().join("Applications");
            fs::create_dir_all(&applications).context("create Applications directory")?;
            TempPath::new_in_with_suffix(&applications, &format!("{}-", operation.name), ".part")?
        }
    };
    host.require(
        "download binary package",
        "curl",
        [
            "--location".as_ref(),
            "--fail".as_ref(),
            "--silent".as_ref(),
            "--show-error".as_ref(),
            "--retry".as_ref(),
            "3".as_ref(),
            "--retry-all-errors".as_ref(),
            "--output".as_ref(),
            temporary.path().as_os_str(),
            "--".as_ref(),
            url.as_ref(),
        ],
    )?;
    Ok(temporary)
}

fn install_deb(host: &Host, temporary: TempPath) -> Result<()> {
    host.require(
        "binary Debian install",
        "sudo",
        [
            "DEBIAN_FRONTEND=noninteractive".as_ref(),
            "apt-get".as_ref(),
            "install".as_ref(),
            "-y".as_ref(),
            "-qq".as_ref(),
            "--".as_ref(),
            temporary.path().as_os_str(),
        ],
    )?;
    Ok(())
}

fn install_appimage(host: &Host, operation: &BinaryPackageOperation, temporary: TempPath) -> Result<()> {
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o755))?;
    fs::rename(temporary.path(), appimage_destination(host, operation)).context("publish AppImage into Applications")
}

pub(crate) mod cargo_binstall {
    use super::super::Host;
    use anyhow::{Context, Result, bail};
    use std::{os::unix::fs::PermissionsExt, path::PathBuf};

    pub(crate) fn execute(host: &Host) -> Result<()> {
        let cargo_home = host.value("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".cargo"));
        if !cargo_home.is_absolute() {
            bail!("cargo-binstall managed CARGO_HOME must be absolute");
        }
        let installed = cargo_home.join("bin/cargo-binstall");
        if std::fs::metadata(&installed)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        {
            return Ok(());
        }
        let cargo_bin = cargo_home.join("bin/cargo");
        let program = cargo_bin
            .to_str()
            .with_context(|| format!("Cargo executable path is not UTF-8: {}", cargo_bin.display()))?;

        host.require("cargo-binstall-bootstrap", program, ["install", "cargo-binstall", "--locked"])?;
        Ok(())
    }
}
