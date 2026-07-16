use super::{managed_state::ManagedState, Host, TempPath};
use crate::{domain::HttpsUrl, platform::Architecture};
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

const VERSION: u64 = 1;
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
        let value = Self {
            pattern: pattern.into(),
        };
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
    pub fn as_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut value = String::with_capacity(64);
        for byte in self.0 {
            value.push(HEX[(byte >> 4) as usize] as char);
            value.push(HEX[(byte & 0x0f) as usize] as char);
        }
        value
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BinarySourceOperation {
    GithubLatest {
        repository: GithubRepository,
        selector: BinaryPackageSelector,
        sha256: Option<BinarySha256>,
    },
    ChecksummedUrl {
        url: HttpsUrl,
        sha256: BinarySha256,
    },
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
        let value = Self {
            name: name.into(),
            format,
            commands,
            architecture,
            source,
            mode,
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "binary-package".into(),
            self.name.clone(),
            match self.mode {
                BinaryPackageMode::EnsurePresent => "ensure-present",
                BinaryPackageMode::Update => "update",
            }
            .into(),
        ]
    }

    #[cfg(test)]
    pub fn source(&self) -> &BinarySourceOperation {
        &self.source
    }

    #[cfg(test)]
    pub fn mode(&self) -> BinaryPackageMode {
        self.mode
    }
    fn validate(&self) -> Result<()> {
        validate_definition_name(&self.name)?;
        if self.commands.is_empty() {
            bail!(
                "binary package {:?} must declare at least one command",
                self.name
            );
        }
        let mut seen = HashSet::new();
        for command in &self.commands {
            validate_executable(command)?;
            if !seen.insert(command) {
                bail!("binary package commands must be unique");
            }
        }
        match &self.source {
            BinarySourceOperation::GithubLatest {
                repository,
                selector,
                ..
            } => {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u64,
    status: Status,
    declaration: Declaration,
    resolved: Resolved,
    previous: Option<Previous>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    PendingInitial,
    PendingUpdate,
    Completed,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Previous {
    declaration: Declaration,
    resolved: Resolved,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    name: String,
    format: BinaryPackageFormat,
    architecture: String,
    commands: Vec<String>,
    source: SourceDeclaration,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
enum SourceDeclaration {
    Github {
        repository: String,
        pattern: String,
        sha256: Option<String>,
    },
    Url {
        url: String,
        sha256: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resolved {
    tag: Option<String>,
    asset_name: String,
    url: String,
    actual_sha256: String,
    effective_sha256: Option<String>,
}

#[derive(Debug)]
struct Candidate {
    tag: Option<String>,
    asset_name: String,
    url: HttpsUrl,
    effective: Option<BinarySha256>,
    retry_actual: Option<BinarySha256>,
}
struct Downloaded {
    temporary: TempPath,
    resolved: Resolved,
}

pub(crate) fn execute(host: &Host<'_>, operation: &BinaryPackageOperation) -> Result<()> {
    operation
        .validate()
        .context("validate binary package operation")?;
    let declaration = declaration(operation);
    let state = ManagedState::open(host, "binaries", &operation.name, "binary package")?;
    let lock = state.acquire_lock()?;
    let record = state
        .read()?
        .map(|bytes| parse_record(&bytes))
        .transpose()?;
    state.validate_lock_entry(&lock)?;
    if let Some(record) = &record {
        if record.version != VERSION {
            bail!(
                "unsupported binary managed record version {}",
                record.version
            );
        }
        validate_record(record)?;
        if record.status != Status::Completed && record.declaration != declaration {
            bail!("binary package has a pending managed record for a different declaration");
        }
        if record.status == Status::Completed
            && (record.declaration.format != declaration.format
                || record.declaration.architecture != declaration.architecture)
        {
            bail!("binary package format and architecture cannot change across managed state");
        }
    }

    if let Some(record) = &record {
        if record.status == Status::Completed
            && record.declaration == declaration
            && operation.mode == BinaryPackageMode::EnsurePresent
            && postconditions(host, operation, &record.resolved)?
        {
            return Ok(());
        }
    }

    if operation.format == BinaryPackageFormat::AppImage {
        preflight_appimage(host, operation, record.as_ref())?;
    }

    let retrying_update = record
        .as_ref()
        .is_some_and(|record| record.status == Status::PendingUpdate);
    let pending = record.as_ref().filter(|r| r.status != Status::Completed);
    let mut downloaded = None;
    let completed_appimage_repair = record.as_ref().filter(|record| {
        let artifact_ok = artifact_matches(host, operation, &record.resolved).unwrap_or(false);
        record.status == Status::Completed
            && record.declaration == declaration
            && operation.mode == BinaryPackageMode::EnsurePresent
            && operation.format == BinaryPackageFormat::AppImage
            && artifact_ok
    });
    let resolved = if let Some(record) = completed_appimage_repair {
        record.resolved.clone()
    } else if let Some(record) = record.as_ref().filter(|record| {
        record.status == Status::Completed
            && record.declaration == declaration
            && operation.mode == BinaryPackageMode::EnsurePresent
    }) {
        let value = download_candidate(
            host,
            operation,
            candidate_for_retry(operation, &record.resolved)?,
        )?;
        let result = value.resolved.clone();
        downloaded = Some(value);
        result
    } else if let Some(record) = pending {
        if operation.format == BinaryPackageFormat::AppImage
            && artifact_matches(host, operation, &record.resolved)?
        {
            record.resolved.clone()
        } else {
            let value = download_candidate(
                host,
                operation,
                candidate_for_retry(operation, &record.resolved)?,
            )?;
            let result = value.resolved.clone();
            downloaded = Some(value);
            result
        }
    } else {
        let candidate = resolve(host, operation)?;
        if let Some(record) = &record {
            if record.status == Status::Completed
                && record.declaration == declaration
                && same_remote_identity(&candidate, &record.resolved)
                && postconditions(host, operation, &record.resolved)?
            {
                return Ok(());
            }
        }
        let value = download_candidate(host, operation, candidate)?;
        let result = value.resolved.clone();
        downloaded = Some(value);
        result
    };

    let previous = match &record {
        Some(record) if record.status == Status::Completed => Some(Previous {
            declaration: record.declaration.clone(),
            resolved: record.resolved.clone(),
        }),
        Some(record) => record.previous.clone(),
        None => None,
    };
    let status = if record.is_none()
        || record
            .as_ref()
            .is_some_and(|r| r.status == Status::PendingInitial)
    {
        Status::PendingInitial
    } else {
        Status::PendingUpdate
    };
    if operation.format == BinaryPackageFormat::Deb {
        preflight_deb(
            host,
            operation,
            downloaded
                .as_ref()
                .context("Debian convergence requires a staged download")?,
        )?;
    } else if let Some(downloaded) = downloaded.as_ref() {
        require_elf(downloaded.temporary.path(), &operation.name)?;
    }
    publish_record(
        &state,
        &Record {
            version: VERSION,
            status,
            declaration: declaration.clone(),
            resolved: resolved.clone(),
            previous: previous.clone(),
        },
    )?;

    match operation.format {
        BinaryPackageFormat::Deb => install_deb(
            host,
            operation,
            downloaded
                .as_ref()
                .context("Debian retry requires a staged download")?,
        )?,
        BinaryPackageFormat::AppImage => install_appimage(
            host,
            operation,
            &resolved,
            downloaded.as_ref(),
            previous.as_ref(),
            retrying_update,
        )?,
    }
    if !postconditions(host, operation, &resolved)? {
        bail!("binary package postconditions failed");
    }
    state.validate_lock_entry(&lock)?;
    publish_record(
        &state,
        &Record {
            version: VERSION,
            status: Status::Completed,
            declaration,
            resolved,
            previous: None,
        },
    )
}

fn declaration(operation: &BinaryPackageOperation) -> Declaration {
    let source = match &operation.source {
        BinarySourceOperation::GithubLatest {
            repository,
            selector,
            sha256,
        } => SourceDeclaration::Github {
            repository: repository.0.clone(),
            pattern: selector.pattern.clone(),
            sha256: sha256.map(BinarySha256::as_hex),
        },
        BinarySourceOperation::ChecksummedUrl { url, sha256 } => SourceDeclaration::Url {
            url: url.as_str().into(),
            sha256: sha256.as_hex(),
        },
    };
    Declaration {
        name: operation.name.clone(),
        format: operation.format,
        architecture: operation.architecture.canonical().into(),
        commands: operation.commands.clone(),
        source,
    }
}
fn publish_record(state: &ManagedState, record: &Record) -> Result<()> {
    state.publish(&serde_json::to_vec(record).context("serialize binary managed record")?)
}
fn parse_record(bytes: &[u8]) -> Result<Record> {
    super::managed_state::parse_strict_json(bytes).context("parse strict binary managed record")
}
fn validate_record(record: &Record) -> Result<()> {
    if record.version != VERSION {
        bail!(
            "unsupported binary managed record version {}",
            record.version
        );
    }
    validate_declaration(&record.declaration)?;
    validate_resolved_for_declaration(&record.resolved, &record.declaration)?;
    if let Some(previous) = &record.previous {
        validate_declaration(&previous.declaration)?;
        validate_resolved_for_declaration(&previous.resolved, &previous.declaration)?;
        if previous.declaration.name != record.declaration.name
            || previous.declaration.format != record.declaration.format
            || previous.declaration.architecture != record.declaration.architecture
        {
            bail!("binary pending-update previous ownership has mismatched identity");
        }
    }
    match record.status {
        Status::Completed if record.previous.is_some() => {
            bail!("completed binary record must not retain previous state")
        }
        Status::PendingInitial if record.previous.is_some() => {
            bail!("pending initial binary record must not retain previous state")
        }
        Status::PendingUpdate if record.previous.is_none() => {
            bail!("pending update binary record must retain previous state")
        }
        _ => Ok(()),
    }
}

fn validate_declaration(stored: &Declaration) -> Result<()> {
    validate_definition_name(&stored.name)?;
    if Architecture::normalize(&stored.architecture)?.canonical() != stored.architecture {
        bail!("binary record architecture is not canonical");
    }
    let source = match &stored.source {
        SourceDeclaration::Github {
            repository,
            pattern,
            sha256,
        } => BinarySourceOperation::GithubLatest {
            repository: GithubRepository::parse(repository)?,
            selector: BinaryPackageSelector::new(pattern)?,
            sha256: sha256.as_deref().map(BinarySha256::parse).transpose()?,
        },
        SourceDeclaration::Url { url, sha256 } => BinarySourceOperation::ChecksummedUrl {
            url: HttpsUrl::parse(url)?,
            sha256: BinarySha256::parse(sha256)?,
        },
    };
    let operation = BinaryPackageOperation::new(
        &stored.name,
        stored.format,
        stored.commands.clone(),
        Architecture::normalize(&stored.architecture)?,
        source,
        BinaryPackageMode::EnsurePresent,
    )?;
    if declaration(&operation) != *stored {
        bail!("binary declaration does not match its canonical operation identity");
    }
    Ok(())
}

fn validate_resolved(value: &Resolved) -> Result<()> {
    if let Some(tag) = &value.tag {
        validate_safe_scalar(tag, "release tag")?;
    }
    validate_asset_name(&value.asset_name)?;
    let url = HttpsUrl::parse(&value.url)?;
    if url.as_str() != value.url {
        bail!("binary resolved URL is not canonical");
    }
    let actual = BinarySha256::parse(&value.actual_sha256)?;
    if let Some(value) = &value.effective_sha256 {
        if BinarySha256::parse(value)? != actual {
            bail!("binary resolved actual and effective SHA-256 checksums differ");
        }
    }
    Ok(())
}

fn validate_resolved_for_declaration(value: &Resolved, declaration: &Declaration) -> Result<()> {
    validate_resolved(value)?;
    match &declaration.source {
        SourceDeclaration::Github {
            pattern, sha256, ..
        } => {
            if value.tag.is_none()
                || !regex_match(pattern, &value.asset_name)?
                || sha256
                    .as_ref()
                    .is_some_and(|declared| value.effective_sha256.as_ref() != Some(declared))
            {
                bail!("binary GitHub resolved identity does not match its declaration");
            }
        }
        SourceDeclaration::Url { url, sha256 } => {
            if value.tag.is_some()
                || value.url != *url
                || value.asset_name != fixed_asset_name(url)
                || value.effective_sha256.as_ref() != Some(sha256)
            {
                bail!("binary fixed-URL resolved identity does not match its declaration");
            }
        }
    }
    Ok(())
}

mod appimage;
mod source;

use appimage::*;
use source::*;

fn install_deb(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    downloaded: &Downloaded,
) -> Result<()> {
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

fn preflight_deb(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    downloaded: &Downloaded,
) -> Result<()> {
    let path = downloaded.temporary.path().as_os_str();
    host.require(
        "binary Debian preflight",
        "dpkg-deb",
        ["--info".as_ref(), "--".as_ref(), path],
    )?;
    let fields = host.require(
        "binary Debian metadata",
        "dpkg-deb",
        [
            "--field".as_ref(),
            "--".as_ref(),
            path,
            "Package".as_ref(),
            "Architecture".as_ref(),
        ],
    )?;
    let text = std::str::from_utf8(&fields.stdout).context("dpkg-deb metadata is not UTF-8")?;
    let lines = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .collect::<Vec<_>>();
    let package = lines
        .first()
        .and_then(|line| line.strip_prefix("Package: "));
    let architecture = lines
        .get(1)
        .and_then(|line| line.strip_prefix("Architecture: "));
    if lines.len() != 2
        || !package.is_some_and(valid_debian_package)
        || !architecture.is_some_and(|architecture| {
            architecture == "all" || architecture == operation.architecture.debian()
        })
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
    parse_hex(
        value
            .strip_prefix("sha256:")
            .context("digest must use sha256:<64-lowercase-hex>")?,
    )
}
fn parse_hex(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
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
        || !owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !owner
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !owner
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
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
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
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
    value
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b))
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
fn regex_match(pattern: &str, text: &str) -> Result<bool> {
    Ok(Regex::new(pattern)?.is_match(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operation() -> BinaryPackageOperation {
        operation_for(Architecture::Amd64)
    }
    fn operation_for(architecture: Architecture) -> BinaryPackageOperation {
        BinaryPackageOperation::new(
            "sample",
            BinaryPackageFormat::AppImage,
            vec!["sample".into()],
            architecture,
            BinarySourceOperation::GithubLatest {
                repository: GithubRepository::parse("owner/repo").unwrap(),
                selector: BinaryPackageSelector::new(r"^sample-.\.AppImage$").unwrap(),
                sha256: None,
            },
            BinaryPackageMode::EnsurePresent,
        )
        .unwrap()
    }
    fn selector(operation: &BinaryPackageOperation) -> &BinaryPackageSelector {
        match &operation.source {
            BinarySourceOperation::GithubLatest { selector, .. } => selector,
            BinarySourceOperation::ChecksummedUrl { .. } => unreachable!(),
        }
    }
    fn release(asset_fields: &str) -> String {
        format!(
            r#"{{"draft":false,"prerelease":false,"tag_name":"v1","assets":[{{"name":"sample-a.AppImage","browser_download_url":"https://example.test/a",{asset_fields}}}]}}"#
        )
    }
    #[test]
    fn stable_release_fields_are_strict() {
        let op = operation();
        for input in [
            r#"{"assets":[]}"#,
            r#"{"draft":false,"prerelease":false,"tag_name":"","assets":[]}"#,
            r#"{"draft":0,"prerelease":false,"tag_name":"v1","assets":[]}"#,
            r#"{"draft":false,"prerelease":true,"tag_name":"v1","assets":[]}"#,
            "{\"draft\":false,\"prerelease\":false,\"tag_name\":\"v1\\nunsafe\",\"assets\":[]}",
        ] {
            assert!(select_asset(input, selector(&op), None, &op).is_err());
        }
    }
    #[test]
    fn fixed_update_is_rejected() {
        assert!(BinaryPackageOperation::new(
            "sample",
            BinaryPackageFormat::Deb,
            vec!["sample".into()],
            Architecture::Amd64,
            BinarySourceOperation::ChecksummedUrl {
                url: HttpsUrl::parse("https://example.test/a.deb").unwrap(),
                sha256: BinarySha256::parse("00".repeat(32)).unwrap()
            },
            BinaryPackageMode::Update
        )
        .is_err());
    }
    #[test]
    fn checksum_composition_rejects_mismatch() {
        let op = operation();
        let json = format!(
            r#"{{"draft":false,"prerelease":false,"tag_name":"v1","assets":[{{"name":"sample-a.AppImage","browser_download_url":"https://example.test/a","digest":"sha256:{}"}}]}}"#,
            "11".repeat(32)
        );
        assert!(select_asset(
            &json,
            selector(&op),
            Some(BinarySha256::parse("22".repeat(32)).unwrap()),
            &op
        )
        .is_err());
    }
    #[test]
    fn checksum_composition_accepts_api_declaration_both_and_neither() {
        let op = operation();
        let checksum = "11".repeat(32);
        for (declared, asset_fields, expected) in [
            (
                None,
                format!(r#""digest":"sha256:{checksum}""#),
                Some(checksum.as_str()),
            ),
            (
                Some(checksum.as_str()),
                r#""digest":null"#.into(),
                Some(checksum.as_str()),
            ),
            (
                Some(checksum.as_str()),
                format!(r#""digest":"sha256:{checksum}""#),
                Some(checksum.as_str()),
            ),
            (None, r#""digest":null"#.into(), None),
        ] {
            let candidate = select_asset(
                &release(&asset_fields),
                selector(&op),
                declared.map(|value| BinarySha256::parse(value).unwrap()),
                &op,
            )
            .unwrap();
            assert_eq!(
                candidate.effective.map(BinarySha256::as_hex).as_deref(),
                expected
            );
        }
    }
    #[test]
    fn selector_cardinality_and_diagnostics_cover_every_architecture() {
        for architecture in [
            Architecture::Amd64,
            Architecture::Arm64,
            Architecture::Arm32,
            Architecture::Riscv64,
        ] {
            let operation = operation_for(architecture);
            for (assets, count) in [
                (r#"[]"#, 0),
                (
                    r#"[{"name":"sample-a.AppImage"},{"name":"sample-b.AppImage"}]"#,
                    2,
                ),
            ] {
                let input = format!(
                    r#"{{"draft":false,"prerelease":false,"tag_name":"v1","assets":{assets}}}"#
                );
                let error = select_asset(&input, selector(&operation), None, &operation)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(architecture.canonical()), "{error}");
                assert!(
                    error.contains(&format!("matched {count} assets")),
                    "{error}"
                );
            }
        }
    }
    #[test]
    fn fixed_urls_always_derive_a_safe_deterministic_asset_name() {
        for (url, expected) in [
            (
                "https://example.test/downloads/app.AppImage",
                "app.AppImage",
            ),
            ("https://example.test/downloads/", "downloads"),
            ("https://example.test/", "artifact"),
            ("https://example.test/...", "artifact"),
        ] {
            let name = fixed_asset_name(url);
            assert_eq!(name, expected);
            validate_asset_name(&name).unwrap();
        }
    }
    #[test]
    fn managed_records_require_canonical_resolved_identity_and_status() {
        let operation = operation();
        let declaration = declaration(&operation);
        let resolved = Resolved {
            tag: Some("v1".into()),
            asset_name: "sample-a.AppImage".into(),
            url: "https://example.test/a".into(),
            actual_sha256: "11".repeat(32),
            effective_sha256: None,
        };
        let completed = Record {
            version: VERSION,
            status: Status::Completed,
            declaration: declaration.clone(),
            resolved: resolved.clone(),
            previous: None,
        };
        validate_record(&completed).unwrap();

        let previous = Previous {
            declaration: declaration.clone(),
            resolved: resolved.clone(),
        };
        let mut invalid = vec![
            Record {
                version: 2,
                ..completed.clone()
            },
            Record {
                status: Status::Completed,
                previous: Some(previous.clone()),
                ..completed.clone()
            },
            Record {
                status: Status::PendingInitial,
                previous: Some(previous.clone()),
                ..completed.clone()
            },
            Record {
                status: Status::PendingUpdate,
                ..completed.clone()
            },
        ];
        let mut bad_asset = completed.clone();
        bad_asset.resolved.asset_name = "other.AppImage".into();
        invalid.push(bad_asset);
        let mut missing_tag = completed.clone();
        missing_tag.resolved.tag = None;
        invalid.push(missing_tag);
        let mut noncanonical_url = completed.clone();
        noncanonical_url.resolved.url = "https://EXAMPLE.test/a".into();
        invalid.push(noncanonical_url);
        let mut mismatched_checksums = completed.clone();
        mismatched_checksums.resolved.effective_sha256 = Some("22".repeat(32));
        invalid.push(mismatched_checksums);
        for record in invalid {
            assert!(validate_record(&record).is_err(), "{record:?}");
        }

        let bytes = serde_json::to_vec(&completed).unwrap();
        let duplicate = String::from_utf8(bytes.clone()).unwrap().replacen(
            "\"version\":1",
            "\"version\":1,\"version\":1",
            1,
        );
        assert!(parse_record(duplicate.as_bytes()).is_err());
        let unknown = String::from_utf8(bytes)
            .unwrap()
            .replacen('{', "{\"unknown\":true,", 1);
        assert!(parse_record(unknown.as_bytes()).is_err());
    }
    #[test]
    fn obsidian_regexes_distinguish_unmarked_amd64_from_arm64() {
        let amd64 = r"^Obsidian-[0-9]+(?:\.[0-9]+)+\.AppImage$";
        let arm64 = r"^Obsidian-[0-9]+(?:\.[0-9]+)+-arm64\.AppImage$";

        assert!(regex_match(amd64, "Obsidian-1.12.7.AppImage").unwrap());
        assert!(!regex_match(amd64, "Obsidian-1.12.7-arm64.AppImage").unwrap());
        assert!(regex_match(arm64, "Obsidian-1.12.7-arm64.AppImage").unwrap());
        assert!(!regex_match(arm64, "Obsidian-1.12.7.AppImage").unwrap());
        assert!(BinaryPackageSelector::new(amd64).is_ok());
        assert!(BinaryPackageSelector::new("Obsidian-.*").is_err());
        assert!(BinaryPackageSelector::new("^Obsidian-($").is_err());
    }

    #[test]
    fn regex_dot_matches_one_unicode_scalar() {
        assert!(regex_match(r"^x-.\.deb$", "x-é.deb").unwrap());
        assert!(!regex_match(r"^x-.\.deb$", "x-ab.deb").unwrap());
    }
}
