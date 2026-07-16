use super::{managed_state::ManagedState, Host, TempPath};
use crate::{domain::HttpsUrl, platform::Architecture};
use anyhow::{bail, Context, Result};
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
    include: String,
    excludes: Vec<String>,
}
impl BinaryPackageSelector {
    pub fn new(include: impl Into<String>, excludes: Vec<String>) -> Result<Self> {
        let value = Self {
            include: include.into(),
            excludes,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        validate_wildcard(&self.include, "include selector")?;
        let mut seen = HashSet::new();
        for value in &self.excludes {
            validate_wildcard(value, "exclude selector")?;
            if !seen.insert(value) {
                bail!("binary package exclude selectors must be unique");
            }
        }
        Ok(())
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
        include: String,
        excludes: Vec<String>,
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
            include: selector.include.clone(),
            excludes: selector.excludes.clone(),
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
            include,
            excludes,
            sha256,
        } => BinarySourceOperation::GithubLatest {
            repository: GithubRepository::parse(repository)?,
            selector: BinaryPackageSelector::new(include, excludes.clone())?,
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
            include,
            excludes,
            sha256,
            ..
        } => {
            if value.tag.is_none()
                || !wildcard_match(include, &value.asset_name)
                || excludes
                    .iter()
                    .any(|pattern| wildcard_match(pattern, &value.asset_name))
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

fn resolve(host: &Host<'_>, operation: &BinaryPackageOperation) -> Result<Candidate> {
    match &operation.source {
        BinarySourceOperation::ChecksummedUrl { url, sha256 } => Ok(Candidate {
            tag: None,
            asset_name: fixed_asset_name(url.as_str()),
            url: url.clone(),
            effective: Some(*sha256),
            retry_actual: None,
        }),
        BinarySourceOperation::GithubLatest {
            repository,
            selector,
            sha256,
        } => {
            let endpoint = format!(
                "https://api.github.com/repos/{}/releases/latest",
                repository.as_str()
            );
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
                std::str::from_utf8(&output.stdout)
                    .context("GitHub release metadata is not UTF-8")?,
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
    let object = value
        .as_object()
        .context("GitHub release JSON must be an object")?;
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
    let matches = named
        .into_iter()
        .filter(|(_, _, name)| {
            wildcard_match(&selector.include, name)
                && !selector
                    .excludes
                    .iter()
                    .any(|pattern| wildcard_match(pattern, name))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!(
            "binary package {:?} ({}) selector matched {} assets",
            operation.name,
            operation.architecture.canonical(),
            matches.len()
        );
    }
    let (index, asset, name) = matches[0];
    let object = asset.as_object().unwrap();
    let url = HttpsUrl::parse(
        object
            .get("browser_download_url")
            .with_context(|| {
                format!("GitHub release asset {index} is missing browser_download_url")
            })?
            .as_str()
            .with_context(|| {
                format!("GitHub release asset {index} browser_download_url must be a string")
            })?,
    )?;
    let api = match object.get("digest") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(BinarySha256(parse_digest(value)?)),
        Some(_) => bail!("GitHub release asset {index} digest must be a string or null"),
    };
    if declared.is_some() && api.is_some() && declared != api {
        bail!("declared and GitHub API SHA-256 checksums differ");
    }
    Ok(Candidate {
        tag: Some(tag.into()),
        asset_name: name.into(),
        url,
        effective: declared.or(api),
        retry_actual: None,
    })
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
fn candidate_for_retry(
    operation: &BinaryPackageOperation,
    resolved: &Resolved,
) -> Result<Candidate> {
    Ok(Candidate {
        tag: resolved.tag.clone(),
        asset_name: resolved.asset_name.clone(),
        url: HttpsUrl::parse(&resolved.url)?,
        effective: resolved
            .effective_sha256
            .as_deref()
            .map(BinarySha256::parse)
            .transpose()?
            .or(match operation.source {
                BinarySourceOperation::ChecksummedUrl { sha256, .. } => Some(sha256),
                _ => None,
            }),
        retry_actual: Some(BinarySha256::parse(&resolved.actual_sha256)?),
    })
}
fn same_remote_identity(candidate: &Candidate, resolved: &Resolved) -> bool {
    candidate.tag == resolved.tag
        && candidate.asset_name == resolved.asset_name
        && candidate.url.as_str() == resolved.url
        && candidate.effective.map(BinarySha256::as_hex) == resolved.effective_sha256
}

fn download_candidate(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    candidate: Candidate,
) -> Result<Downloaded> {
    let suffix = match operation.format {
        BinaryPackageFormat::Deb => ".deb",
        BinaryPackageFormat::AppImage => ".AppImage",
    };
    let temporary = TempPath::new_with_suffix(host, &operation.name, suffix)?;
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
    let actual = BinarySha256(sha256_file(temporary.path())?);
    if candidate
        .effective
        .is_some_and(|expected| expected != actual)
        || candidate
            .retry_actual
            .is_some_and(|expected| expected != actual)
    {
        bail!("binary package SHA-256 checksum mismatch");
    }
    Ok(Downloaded {
        temporary,
        resolved: Resolved {
            tag: candidate.tag,
            asset_name: candidate.asset_name,
            url: candidate.url.as_str().into(),
            actual_sha256: actual.as_hex(),
            effective_sha256: candidate.effective.map(BinarySha256::as_hex),
        },
    })
}

fn fixed_asset_name(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .rev()
                .find(|segment| !segment.is_empty() && valid_asset_name(segment))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "artifact".into())
}

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
    if lines.len() != 2
        || !valid_debian_package(lines[0])
        || lines[1].is_empty()
        || lines[1] != "all" && lines[1] != operation.architecture.debian()
    {
        bail!("dpkg-deb Package/Architecture output is malformed or does not match native architecture");
    }
    Ok(())
}

fn data_artifact(host: &Host<'_>, operation: &BinaryPackageOperation) -> PathBuf {
    host.value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"))
        .join("cozydot/binaries")
        .join(format!("{}.AppImage", operation.name))
}
fn command_links(host: &Host<'_>, operation: &BinaryPackageOperation) -> Vec<PathBuf> {
    let root = host
        .value("XDG_BIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/bin"));
    operation
        .commands
        .iter()
        .map(|name| root.join(name))
        .collect()
}
fn preflight_appimage(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    record: Option<&Record>,
) -> Result<()> {
    let artifact = data_artifact(host, operation);
    ensure_secure_data_parent(host, &artifact)?;
    ensure_secure_command_root(host)?;
    match fs::symlink_metadata(&artifact) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) if record.is_some_and(|record| record_owns_artifact(&artifact, record)) => {}
        Ok(_) => bail!(
            "binary AppImage artifact conflict at {}",
            artifact.display()
        ),
        Err(error) => return Err(error.into()),
    }
    for (command, link) in operation
        .commands
        .iter()
        .zip(command_links(host, operation))
    {
        let retry_may_own_link = record.is_some_and(|record| match record.status {
            Status::PendingInitial | Status::PendingUpdate => {
                record.declaration.commands.contains(command)
            }
            Status::Completed => record.declaration.commands.contains(command),
        });
        preflight_link(&link, &artifact, !retry_may_own_link)?;
    }
    if let Some(previous) = record.and_then(|r| {
        if r.status == Status::Completed {
            Some(Previous {
                declaration: r.declaration.clone(),
                resolved: r.resolved.clone(),
            })
        } else {
            r.previous.clone()
        }
    }) {
        for command in previous
            .declaration
            .commands
            .iter()
            .filter(|name| !operation.commands.contains(name))
        {
            let link = command_links_for(host, std::slice::from_ref(command))
                .pop()
                .unwrap();
            preflight_stale_link(
                &link,
                &artifact,
                record.is_some_and(|record| record.status == Status::PendingUpdate),
            )?;
        }
    }
    Ok(())
}
fn install_appimage(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    resolved: &Resolved,
    downloaded: Option<&Downloaded>,
    previous: Option<&Previous>,
    retrying_update: bool,
) -> Result<()> {
    let artifact = data_artifact(host, operation);
    ensure_secure_data_parent(host, &artifact)?;
    if !artifact_matches(host, operation, resolved)? {
        let source = downloaded
            .context("AppImage publication requires staged bytes")?
            .temporary
            .path();
        require_elf(source, &operation.name)?;
        let destination_absent = match fs::symlink_metadata(&artifact) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Ok(_)
                if previous.is_some_and(|previous| {
                    previous.declaration.format == BinaryPackageFormat::AppImage
                        && valid_artifact(&artifact, &previous.resolved.actual_sha256)
                            .unwrap_or(false)
                }) =>
            {
                false
            }
            Ok(_) | Err(_) => bail!(
                "binary AppImage artifact ownership changed before publication at {}",
                artifact.display()
            ),
        };
        publish_artifact(source, &artifact, previous.is_none() || destination_absent)?;
    }
    let links = command_links(host, operation);
    for link in &links {
        publish_link(link, &artifact)?;
    }
    verify_appimage(&artifact, &links, resolved)?;
    if let Some(previous) = previous {
        for command in previous
            .declaration
            .commands
            .iter()
            .filter(|name| !operation.commands.contains(name))
        {
            let link = command_links_for(host, std::slice::from_ref(command))
                .pop()
                .unwrap();
            match fs::symlink_metadata(&link) {
                Ok(_) if managed_link(&link, &artifact) => {
                    fs::remove_file(&link).with_context(|| {
                        format!("remove stale owned command {}", link.display())
                    })?;
                    if fs::symlink_metadata(&link).is_ok() {
                        bail!("stale binary command removal failed");
                    }
                }
                Err(error) if retrying_update && error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) | Err(_) => bail!(
                    "stale owned binary command at {} was changed or is missing",
                    link.display()
                ),
            }
        }
    }
    Ok(())
}
fn command_links_for(host: &Host<'_>, commands: &[String]) -> Vec<PathBuf> {
    let root = host
        .value("XDG_BIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/bin"));
    commands.iter().map(|name| root.join(name)).collect()
}
fn ensure_secure_data_parent(host: &Host<'_>, artifact: &Path) -> Result<()> {
    let data = host
        .value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"));
    if !data.is_absolute() {
        bail!("binary data directory must be absolute");
    }
    let existed = fs::symlink_metadata(&data).is_ok();
    fs::create_dir_all(&data)?;
    if !existed {
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700))?;
    }
    validate_owned_directory(&data)?;
    let cozy = data.join("cozydot");
    create_owned_directory(&cozy)?;
    let binaries = cozy.join("binaries");
    create_owned_directory(&binaries)?;
    if artifact.parent() != Some(binaries.as_path()) {
        bail!("binary artifact path escaped managed directory");
    }
    Ok(())
}
fn ensure_secure_command_root(host: &Host<'_>) -> Result<()> {
    let root = host
        .value("XDG_BIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/bin"));
    if !root.is_absolute() {
        bail!("binary command directory must be absolute");
    }
    let existed = fs::symlink_metadata(&root).is_ok();
    fs::create_dir_all(&root).context("create binary command directory")?;
    if !existed {
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    validate_owned_directory(&root)
        .context("binary command directory has unsafe type, owner, or permissions")
}
fn create_owned_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    validate_owned_directory(path)
}
fn validate_owned_directory(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_dir()
        || m.uid() != rustix::process::geteuid().as_raw()
        || m.permissions().mode() & 0o022 != 0
    {
        bail!("binary managed data directory has unsafe type, owner, or permissions");
    }
    Ok(())
}
fn publish_artifact(source: &Path, destination: &Path, no_replace: bool) -> Result<()> {
    let parent = destination.parent().unwrap();
    let mut source_file = fs::File::open(source)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut source_file, staged.as_file_mut())?;
    staged
        .as_file_mut()
        .set_permissions(fs::Permissions::from_mode(0o755))?;
    staged.as_file_mut().sync_all()?;
    if no_replace {
        let path = staged.into_temp_path();
        fs::hard_link(&path, destination)
            .context("publish initial binary artifact without replacement")?;
        fs::remove_file(&path)?;
    } else {
        staged
            .into_temp_path()
            .persist(destination)
            .map_err(|e| e.error)?;
    }
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn preflight_link(link: &Path, artifact: &Path, require_absent: bool) -> Result<()> {
    match fs::symlink_metadata(link) {
        Ok(_) if require_absent => {
            bail!("binary AppImage command conflict at {}", link.display())
        }
        Ok(m) if m.file_type().is_symlink() && managed_link(link, artifact) => Ok(()),
        Ok(_) => bail!("binary AppImage command conflict at {}", link.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn preflight_stale_link(link: &Path, artifact: &Path, allow_absent: bool) -> Result<()> {
    match fs::symlink_metadata(link) {
        Ok(_) if managed_link(link, artifact) => Ok(()),
        Err(error) if allow_absent && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) | Err(_) => bail!(
            "stale owned binary command at {} was changed or is missing",
            link.display()
        ),
    }
}
fn publish_link(link: &Path, artifact: &Path) -> Result<()> {
    if managed_link(link, artifact) {
        return Ok(());
    }
    fs::create_dir_all(link.parent().unwrap())?;
    match symlink(artifact, link) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && managed_link(link, artifact) => {
            Ok(())
        }
        Err(e) => Err(e).context("publish binary AppImage command link"),
    }
}
fn managed_link(link: &Path, artifact: &Path) -> bool {
    fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_symlink())
        && fs::read_link(link).is_ok_and(|target| target == artifact)
}
fn verify_appimage(artifact: &Path, links: &[PathBuf], resolved: &Resolved) -> Result<()> {
    if !valid_artifact(artifact, &resolved.actual_sha256)?
        || links.iter().any(|link| !managed_link(link, artifact))
    {
        bail!("binary AppImage verification failed");
    }
    Ok(())
}
fn artifact_matches(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    resolved: &Resolved,
) -> Result<bool> {
    let _ = host;
    valid_artifact(&data_artifact(host, operation), &resolved.actual_sha256)
}
fn record_owns_artifact(artifact: &Path, record: &Record) -> bool {
    record.declaration.format == BinaryPackageFormat::AppImage
        && valid_artifact(artifact, &record.resolved.actual_sha256).unwrap_or(false)
        || record.previous.as_ref().is_some_and(|previous| {
            previous.declaration.format == BinaryPackageFormat::AppImage
                && valid_artifact(artifact, &previous.resolved.actual_sha256).unwrap_or(false)
        })
}
fn valid_artifact(path: &Path, digest: &str) -> Result<bool> {
    let m = match fs::symlink_metadata(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    Ok(m.file_type().is_file()
        && m.uid() == rustix::process::geteuid().as_raw()
        && m.nlink() == 1
        && m.len() > 0
        && m.permissions().mode() & 0o7777 == 0o755
        && has_elf_magic(path)
        && sha256_file(path)? == BinarySha256::parse(digest)?.0)
}
fn postconditions(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    resolved: &Resolved,
) -> Result<bool> {
    match operation.format {
        BinaryPackageFormat::Deb => Ok(operation
            .commands
            .iter()
            .all(|name| executable_on_path(host, name))),
        BinaryPackageFormat::AppImage => {
            let artifact = data_artifact(host, operation);
            Ok(valid_artifact(&artifact, &resolved.actual_sha256)?
                && command_links(host, operation)
                    .iter()
                    .all(|link| managed_link(link, &artifact)))
        }
    }
}
fn verify_commands(host: &Host<'_>, operation: &BinaryPackageOperation) -> Result<()> {
    let missing = operation
        .commands
        .iter()
        .filter(|name| !executable_on_path(host, name))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!("binary package installed but commands remain unavailable: {missing:?}");
    }
    Ok(())
}
fn executable_on_path(host: &Host<'_>, name: &str) -> bool {
    host.value("PATH")
        .and_then(|path| {
            std::env::split_paths(&path).find(|dir| {
                fs::metadata(dir.join(name))
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
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
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == *b"\x7fELF"
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
fn validate_wildcard(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || !value.contains(['*', '?'])
        || value.contains(['/', '\\', '[', ']', '{', '}', '$', '(', ')', '`'])
        || value.chars().any(char::is_control)
    {
        bail!("binary package {field} must be an anchored filename wildcard using only '*' and '?' operators");
    }
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
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let text = text.chars().collect::<Vec<_>>();
    let mut previous = vec![false; text.len() + 1];
    previous[0] = true;
    for token in pattern.chars() {
        let mut current = vec![false; text.len() + 1];
        match token {
            '*' => {
                current[0] = previous[0];
                for i in 1..=text.len() {
                    current[i] = previous[i] || current[i - 1];
                }
            }
            '?' => current[1..].copy_from_slice(&previous[..text.len()]),
            literal => {
                for i in 1..=text.len() {
                    current[i] = previous[i - 1] && text[i - 1] == literal;
                }
            }
        }
        previous = current;
    }
    previous[text.len()]
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
                selector: BinaryPackageSelector::new("sample-?.AppImage", vec![]).unwrap(),
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
    fn wildcard_matches_unicode_scalar() {
        assert!(wildcard_match("x-?.deb", "x-é.deb"));
        assert!(!wildcard_match("x-?.deb", "x-ab.deb"));
    }
}
