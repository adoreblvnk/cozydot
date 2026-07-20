use super::{Host, TempPath};
use crate::{config::HttpsUrl, platform::Architecture};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryPackageFormat {
    Deb,
    AppImage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryPackageMode {
    EnsurePresent,
    Update,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GithubRepository(String);
impl GithubRepository {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_repository(&value)?;
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryPackageSelector {
    pattern: String,
}
impl BinaryPackageSelector {
    pub fn new(pattern: impl Into<String>) -> Result<Self> {
        let value = Self { pattern: pattern.into() };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        validate_asset_regex(&self.pattern)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BinarySha256([u8; 32]);
impl BinarySha256 {
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        Ok(Self(parse_hex(value.as_ref())?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BinarySourceOperation {
    GithubLatest { repository: GithubRepository, selector: BinaryPackageSelector, sha256: Option<BinarySha256> },
    ChecksummedUrl { url: HttpsUrl, sha256: BinarySha256 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryPackageOperation {
    name: String,
    format: BinaryPackageFormat,
    commands: Vec<String>,
    architecture: Architecture,
    source: BinarySourceOperation,
    mode: BinaryPackageMode,
}
impl BinaryPackageOperation {
    pub fn new(
        name: impl Into<String>,
        format: BinaryPackageFormat,
        commands: Vec<String>,
        architecture: Architecture,
        source: BinarySourceOperation,
        mode: BinaryPackageMode,
    ) -> Result<Self> {
        let value = Self { name: name.into(), format, commands, architecture, source, mode };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<()> {
        validate_definition_name(&self.name)?;
        match self.format {
            BinaryPackageFormat::Deb if self.commands.is_empty() => {
                bail!("binary Debian package {:?} must declare at least one command", self.name)
            }
            BinaryPackageFormat::AppImage if !self.commands.is_empty() => {
                bail!("binary AppImage {:?} must not declare commands", self.name)
            }
            _ => {}
        }
        let mut seen = HashSet::new();
        for command in &self.commands {
            validate_executable(command)?;
            if !seen.insert(command) {
                bail!("binary package commands must be unique");
            }
        }
        match &self.source {
            BinarySourceOperation::GithubLatest { repository, selector, .. } => {
                validate_repository(repository.as_str())?;
                selector.validate()?;
            }
            BinarySourceOperation::ChecksummedUrl { url, .. } => {
                if HttpsUrl::parse(url.as_str())? != *url {
                    bail!("binary fixed URL is not canonical");
                }
                if self.mode == BinaryPackageMode::Update {
                    bail!("fixed checksummed URL binaries do not support update mode");
                }
            }
        }
        Ok(())
    }
}

struct Candidate {
    url: HttpsUrl,
    effective: Option<BinarySha256>,
}
struct Downloaded {
    temporary: TempPath,
}

pub(crate) fn execute(host: &Host, operation: &BinaryPackageOperation) -> Result<()> {
    if is_acceptable_live_state(host, operation)? {
        return Ok(());
    }

    let candidate = resolve(host, operation)?;
    let downloaded = download_candidate(host, operation, candidate)?;
    match operation.format {
        BinaryPackageFormat::Deb => {
            preflight_deb(host, operation, &downloaded)?;
            install_deb(host, operation, &downloaded)
        }
        BinaryPackageFormat::AppImage => install_appimage(host, operation, &downloaded),
    }
}

fn is_acceptable_live_state(host: &Host, operation: &BinaryPackageOperation) -> Result<bool> {
    if operation.mode != BinaryPackageMode::EnsurePresent {
        return Ok(false);
    }
    match operation.format {
        BinaryPackageFormat::Deb => Ok(operation.commands.iter().all(|name| executable_on_path(host, name))),
        BinaryPackageFormat::AppImage => Ok(valid_appimage(&appimage_destination(host, operation))),
    }
}

fn appimage_destination(host: &Host, operation: &BinaryPackageOperation) -> PathBuf {
    host.home().join("Applications").join(format!("{}.AppImage", operation.name))
}

fn valid_appimage(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && metadata.len() > 0
            && metadata.permissions().mode() & 0o111 != 0
            && has_elf_magic(path)
    })
}

fn install_appimage(host: &Host, operation: &BinaryPackageOperation, downloaded: &Downloaded) -> Result<()> {
    require_elf(downloaded.temporary.path(), &operation.name)?;
    fs::set_permissions(downloaded.temporary.path(), fs::Permissions::from_mode(0o755))?;
    fs::rename(downloaded.temporary.path(), appimage_destination(host, operation))
        .context("publish AppImage into Applications")?;
    Ok(())
}

fn verify_commands(host: &Host, operation: &BinaryPackageOperation) -> Result<()> {
    let missing = operation.commands.iter().filter(|name| !executable_on_path(host, name)).collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!("binary package installed but commands remain unavailable: {missing:?}");
    }
    Ok(())
}

fn executable_on_path(host: &Host, name: &str) -> bool {
    host.value("PATH")
        .and_then(|path| {
            std::env::split_paths(&path).find(|dir| {
                fs::metadata(dir.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
        })
        .is_some()
}

fn require_elf(path: &Path, name: &str) -> Result<()> {
    if !has_elf_magic(path) {
        bail!("binary package {name:?} AppImage does not have ELF magic");
    }
    Ok(())
}

fn has_elf_magic(path: &Path) -> bool {
    let mut magic = [0; 4];
    fs::File::open(path).and_then(|mut file| file.read_exact(&mut magic)).is_ok() && magic == *b"\x7fELF"
}

mod source {
    use super::*;

    pub(super) fn resolve(host: &Host, operation: &BinaryPackageOperation) -> Result<Candidate> {
        match &operation.source {
            BinarySourceOperation::ChecksummedUrl { url, sha256 } => {
                Ok(Candidate { url: url.clone(), effective: Some(*sha256) })
            }
            BinarySourceOperation::GithubLatest { repository, selector, sha256 } => {
                let endpoint = format!("https://api.github.com/repos/{}/releases/latest", repository.as_str());
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
                select_asset(
                    std::str::from_utf8(&output.stdout).context("GitHub release metadata is not UTF-8")?,
                    selector,
                    *sha256,
                    operation,
                )
            }
        }
    }
    fn select_asset(
        input: &str,
        selector: &BinaryPackageSelector,
        declared: Option<BinarySha256>,
        operation: &BinaryPackageOperation,
    ) -> Result<Candidate> {
        let value: Value = serde_json::from_str(input).context("parse GitHub release JSON")?;
        let object = value.as_object().context("GitHub release JSON must be an object")?;
        for field in ["draft", "prerelease"] {
            match object.get(field) {
                Some(Value::Bool(false)) => {}
                Some(Value::Bool(true)) => bail!("GitHub release {field} must be false"),
                Some(_) => bail!("GitHub release {field} must be boolean false"),
                None => bail!("GitHub release is missing {field}"),
            }
        }
        let tag = object
            .get("tag_name")
            .context("GitHub release is missing tag_name")?
            .as_str()
            .context("GitHub release tag_name must be a string")?;
        validate_safe_scalar(tag, "release tag")?;
        let assets = object
            .get("assets")
            .context("GitHub release is missing assets")?
            .as_array()
            .context("GitHub release assets must be an array")?;
        let mut named = Vec::new();
        for (index, value) in assets.iter().enumerate() {
            named.push((index, value, parse_asset_name(value, index)?));
        }
        let pattern = Regex::new(&selector.pattern).context("compile binary asset regex")?;
        let matches = named.into_iter().filter(|(_, _, name)| pattern.is_match(name)).collect::<Vec<_>>();
        if matches.len() != 1 {
            bail!(
                "binary package {:?} ({}) selector matched {} assets",
                operation.name,
                operation.architecture.canonical(),
                matches.len()
            );
        }
        let (index, asset, _) = matches[0];
        let object = asset.as_object().context("selected GitHub release asset must be an object")?;
        let url = HttpsUrl::parse(
            object
                .get("browser_download_url")
                .with_context(|| format!("GitHub release asset {index} is missing browser_download_url"))?
                .as_str()
                .with_context(|| format!("GitHub release asset {index} browser_download_url must be a string"))?,
        )?;
        let api = match object.get("digest") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(BinarySha256(parse_digest(value)?)),
            Some(_) => bail!("GitHub release asset {index} digest must be a string or null"),
        };
        if declared.is_some() && api.is_some() && declared != api {
            bail!("declared and GitHub API SHA-256 checksums differ");
        }
        Ok(Candidate { url, effective: declared.or(api) })
    }
    fn parse_asset_name(value: &Value, index: usize) -> Result<&str> {
        let name = value
            .as_object()
            .with_context(|| format!("GitHub release asset {index} must be an object"))?
            .get("name")
            .with_context(|| format!("GitHub release asset {index} is missing name"))?
            .as_str()
            .with_context(|| format!("GitHub release asset {index} name must be a string"))?;
        validate_asset_name(name)?;
        Ok(name)
    }
    pub(super) fn download_candidate(
        host: &Host,
        operation: &BinaryPackageOperation,
        candidate: Candidate,
    ) -> Result<Downloaded> {
        let suffix = match operation.format {
            BinaryPackageFormat::Deb => ".deb",
            BinaryPackageFormat::AppImage => ".part",
        };
        let temporary = if operation.format == BinaryPackageFormat::AppImage {
            let applications = host.home().join("Applications");
            fs::create_dir_all(&applications).context("create Applications directory")?;
            TempPath::new_in_with_suffix(&applications, &format!("{}-", operation.name), suffix)?
        } else {
            TempPath::new_with_suffix(host, &operation.name, suffix)?
        };
        host.require(
            "download binary package",
            "curl",
            [
                "--proto".as_ref(),
                "=https".as_ref(),
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
                candidate.url.as_str().as_ref(),
            ],
        )?;
        let metadata = fs::symlink_metadata(temporary.path())?;
        if !metadata.file_type().is_file() || metadata.len() == 0 {
            bail!("binary package downloaded an empty or non-regular artifact");
        }
        if let Some(expected) = candidate.effective
            && expected != BinarySha256(sha256_file(temporary.path())?)
        {
            bail!("binary package SHA-256 checksum mismatch");
        }
        Ok(Downloaded { temporary })
    }
}

use source::*;

fn install_deb(host: &Host, operation: &BinaryPackageOperation, downloaded: &Downloaded) -> Result<()> {
    let path = downloaded.temporary.path().as_os_str();
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
            path,
        ],
    )?;
    verify_commands(host, operation)
}

fn preflight_deb(host: &Host, operation: &BinaryPackageOperation, downloaded: &Downloaded) -> Result<()> {
    let path = downloaded.temporary.path().as_os_str();
    host.require("binary Debian preflight", "dpkg-deb", ["--info".as_ref(), "--".as_ref(), path])?;
    let fields = host.require(
        "binary Debian metadata",
        "dpkg-deb",
        ["--field".as_ref(), "--".as_ref(), path, "Package".as_ref(), "Architecture".as_ref()],
    )?;
    let text = std::str::from_utf8(&fields.stdout).context("dpkg-deb metadata is not UTF-8")?;
    let lines = text.strip_suffix('\n').unwrap_or(text).split('\n').collect::<Vec<_>>();
    let package = lines.first().and_then(|line| line.strip_prefix("Package: "));
    let architecture = lines.get(1).and_then(|line| line.strip_prefix("Architecture: "));
    if lines.len() != 2
        || !package.is_some_and(valid_debian_package)
        || !architecture
            .is_some_and(|architecture| architecture == "all" || architecture == operation.architecture.debian())
    {
        bail!("dpkg-deb Package/Architecture output is malformed or does not match native architecture");
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<[u8; 32]> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(hash.finalize().into())
}
fn parse_digest(value: &str) -> Result<[u8; 32]> {
    parse_hex(value.strip_prefix("sha256:").context("digest must use sha256:<64-lowercase-hex>")?)
}
fn parse_hex(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        bail!("SHA-256 must be exactly 64 lowercase hexadecimal characters");
    }
    let mut out = [0; 32];
    for (i, p) in value.as_bytes().chunks_exact(2).enumerate() {
        out[i] = (hex_value(p[0]) << 4) | hex_value(p[1]);
    }
    Ok(out)
}
fn hex_value(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        _ => unreachable!(),
    }
}
fn validate_safe_scalar(value: &str, field: &str) -> Result<()> {
    if value.is_empty() || value.chars().any(char::is_control) {
        bail!("GitHub {field} must be a non-empty safe scalar");
    }
    Ok(())
}
fn validate_repository(value: &str) -> Result<()> {
    let mut p = value.split('/');
    let owner = p.next().unwrap_or_default();
    let repo = p.next().unwrap_or_default();
    if p.next().is_some()
        || owner.is_empty()
        || repo.is_empty()
        || !owner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !owner.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        || !owner.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        || !repo.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || repo.bytes().all(|b| b == b'.')
    {
        bail!("GitHub repository must be an owner/repository coordinate");
    }
    Ok(())
}
fn validate_definition_name(value: &str) -> Result<()> {
    if !valid_definition(value) {
        bail!("binary package name must be a safe ASCII definition name");
    }
    Ok(())
}
fn valid_definition(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn validate_executable(value: &str) -> Result<()> {
    let mut b = value.bytes();
    if b.next().is_none_or(|x| !x.is_ascii_alphanumeric())
        || !b.all(|x| x.is_ascii_alphanumeric() || b"._+-".contains(&x))
    {
        bail!("binary package commands must be safe executable basenames");
    }
    Ok(())
}
fn valid_debian_package(value: &str) -> bool {
    value.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b))
}
fn validate_asset_regex(value: &str) -> Result<()> {
    if value.is_empty() || !value.starts_with('^') || !value.ends_with('$') {
        bail!("binary package asset regex must be non-empty and anchored with '^' and '$'");
    }
    Regex::new(value).with_context(|| format!("invalid binary package asset regex {value:?}"))?;
    Ok(())
}
fn validate_asset_name(value: &str) -> Result<()> {
    if !valid_asset_name(value) {
        bail!("asset name must be a safe basename");
    }
    Ok(())
}
fn valid_asset_name(value: &str) -> bool {
    !value.is_empty()
        && !value.bytes().all(|byte| byte == b'.')
        && !value.contains(['/', '\\', '\0'])
        && !value.chars().any(char::is_control)
}

pub(crate) mod cargo_binstall {

    use super::super::Host;
    use anyhow::{Context, Result, bail};
    use std::path::PathBuf;

    pub(crate) fn execute(host: &Host) -> Result<()> {
        let cargo_home = host.value("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".cargo"));
        if !cargo_home.is_absolute() {
            bail!("cargo-binstall managed CARGO_HOME must be absolute");
        }
        let cargo_bin = cargo_home.join("bin/cargo");
        let program = cargo_bin
            .to_str()
            .with_context(|| format!("Cargo executable path is not UTF-8: {}", cargo_bin.display()))?;

        host.require("cargo-binstall-bootstrap", program, ["install", "cargo-binstall", "--locked"])?;
        Ok(())
    }
}
