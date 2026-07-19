use super::{managed_state::ManagedState, Host, TempPath};
use crate::{config::HttpsUrl, platform::Architecture};
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

    fn validate(&self) -> Result<()> {
        validate_definition_name(&self.name)?;
        if self.commands.is_empty() {
            bail!("binary package {:?} must declare at least one command", self.name);
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
                repository, selector, ..
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
    operation.validate().context("validate binary package operation")?;
    let declaration = declaration(operation);
    let state = ManagedState::open(host, "binaries", &operation.name, "binary package")?;
    let lock = state.acquire_lock()?;
    let record = state.read()?.map(|bytes| parse_record(&bytes)).transpose()?;
    state.validate_lock_entry(&lock)?;
    if let Some(record) = &record {
        if record.version != VERSION {
            bail!("unsupported binary managed record version {}", record.version);
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
        let value = download_candidate(host, operation, candidate_for_retry(operation, &record.resolved)?)?;
        let result = value.resolved.clone();
        downloaded = Some(value);
        result
    } else if let Some(record) = pending {
        if operation.format == BinaryPackageFormat::AppImage && artifact_matches(host, operation, &record.resolved)? {
            record.resolved.clone()
        } else {
            let value = download_candidate(host, operation, candidate_for_retry(operation, &record.resolved)?)?;
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
    let status = if record.is_none() || record.as_ref().is_some_and(|r| r.status == Status::PendingInitial) {
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
            downloaded.as_ref().context("Debian retry requires a staged download")?,
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
        bail!("unsupported binary managed record version {}", record.version);
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
        SourceDeclaration::Github { pattern, sha256, .. } => {
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

mod appimage {
    use super::*;

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
        operation.commands.iter().map(|name| root.join(name)).collect()
    }
    pub(super) fn preflight_appimage(
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
            Ok(_) => bail!("binary AppImage artifact conflict at {}", artifact.display()),
            Err(error) => return Err(error.into()),
        }
        for (command, link) in operation.commands.iter().zip(command_links(host, operation)) {
            let retry_may_own_link = record.is_some_and(|record| match record.status {
                Status::PendingInitial | Status::PendingUpdate => record.declaration.commands.contains(command),
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
                let link = command_link_for(host, command);
                preflight_stale_link(
                    &link,
                    &artifact,
                    record.is_some_and(|record| record.status == Status::PendingUpdate),
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn install_appimage(
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
                            && valid_artifact(&artifact, &previous.resolved.actual_sha256).unwrap_or(false)
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
                let link = command_link_for(host, command);
                match fs::symlink_metadata(&link) {
                    Ok(_) if managed_link(&link, &artifact) => {
                        fs::remove_file(&link)
                            .with_context(|| format!("remove stale owned command {}", link.display()))?;
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
    fn command_link_for(host: &Host<'_>, command: &str) -> PathBuf {
        let root = host
            .value("XDG_BIN_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".local/bin"));
        root.join(command)
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
        validate_owned_directory(&root).context("binary command directory has unsafe type, owner, or permissions")
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
        let parent = destination
            .parent()
            .context("binary artifact destination has no parent")?;
        let mut source_file = fs::File::open(source)?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        std::io::copy(&mut source_file, staged.as_file_mut())?;
        staged
            .as_file_mut()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
        staged.as_file_mut().sync_all()?;
        if no_replace {
            let path = staged.into_temp_path();
            fs::hard_link(&path, destination).context("publish initial binary artifact without replacement")?;
            fs::remove_file(&path)?;
        } else {
            staged.into_temp_path().persist(destination).map_err(|e| e.error)?;
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
        fs::create_dir_all(link.parent().context("binary command link has no parent")?)?;
        match symlink(artifact, link) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && managed_link(link, artifact) => Ok(()),
            Err(e) => Err(e).context("publish binary AppImage command link"),
        }
    }
    fn managed_link(link: &Path, artifact: &Path) -> bool {
        fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_symlink())
            && fs::read_link(link).is_ok_and(|target| target == artifact)
    }
    fn verify_appimage(artifact: &Path, links: &[PathBuf], resolved: &Resolved) -> Result<()> {
        if !valid_artifact(artifact, &resolved.actual_sha256)? || links.iter().any(|link| !managed_link(link, artifact))
        {
            bail!("binary AppImage verification failed");
        }
        Ok(())
    }
    pub(super) fn artifact_matches(
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
    pub(super) fn postconditions(
        host: &Host<'_>,
        operation: &BinaryPackageOperation,
        resolved: &Resolved,
    ) -> Result<bool> {
        match operation.format {
            BinaryPackageFormat::Deb => Ok(operation.commands.iter().all(|name| executable_on_path(host, name))),
            BinaryPackageFormat::AppImage => {
                let artifact = data_artifact(host, operation);
                Ok(valid_artifact(&artifact, &resolved.actual_sha256)?
                    && command_links(host, operation)
                        .iter()
                        .all(|link| managed_link(link, &artifact)))
            }
        }
    }
    pub(super) fn verify_commands(host: &Host<'_>, operation: &BinaryPackageOperation) -> Result<()> {
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
                    fs::metadata(dir.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                })
            })
            .is_some()
    }

    pub(super) fn require_elf(path: &Path, name: &str) -> Result<()> {
        if !has_elf_magic(path) {
            bail!("binary package {name:?} AppImage does not have ELF magic");
        }
        Ok(())
    }
    fn has_elf_magic(path: &Path) -> bool {
        let mut magic = [0; 4];
        fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"\x7fELF"
    }
}

mod source {
    use super::*;

    pub(super) fn resolve(host: &Host<'_>, operation: &BinaryPackageOperation) -> Result<Candidate> {
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
    pub(super) fn select_asset(
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
        let matches = named
            .into_iter()
            .filter(|(_, _, name)| pattern.is_match(name))
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
        let object = asset
            .as_object()
            .context("selected GitHub release asset must be an object")?;
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
    pub(super) fn candidate_for_retry(operation: &BinaryPackageOperation, resolved: &Resolved) -> Result<Candidate> {
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
    pub(super) fn same_remote_identity(candidate: &Candidate, resolved: &Resolved) -> bool {
        candidate.tag == resolved.tag
            && candidate.asset_name == resolved.asset_name
            && candidate.url.as_str() == resolved.url
            && candidate.effective.map(BinarySha256::as_hex) == resolved.effective_sha256
    }

    pub(super) fn download_candidate(
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
        if candidate.effective.is_some_and(|expected| expected != actual)
            || candidate.retry_actual.is_some_and(|expected| expected != actual)
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

    pub(super) fn fixed_asset_name(url: &str) -> String {
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
}

use appimage::*;
use source::*;

fn install_deb(host: &Host<'_>, operation: &BinaryPackageOperation, downloaded: &Downloaded) -> Result<()> {
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

fn preflight_deb(host: &Host<'_>, operation: &BinaryPackageOperation, downloaded: &Downloaded) -> Result<()> {
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
    parse_hex(
        value
            .strip_prefix("sha256:")
            .context("digest must use sha256:<64-lowercase-hex>")?,
    )
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

pub(crate) mod cargo_binstall {

    use super::super::{managed_state::ManagedState, Host, TempDir, TempPath};
    use crate::{config::HttpsUrl, platform::Architecture};
    use anyhow::{bail, Context, Result};
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
    };

    const VERSION: u64 = 1;
    const RELEASE_ENDPOINT: &str = "https://api.github.com/repos/cargo-bins/cargo-binstall/releases/latest";
    const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
    const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
    const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct CargoBinstallBootstrapOperation {
        architecture: Architecture,
    }

    impl CargoBinstallBootstrapOperation {
        pub fn new(architecture: Architecture) -> Self {
            Self { architecture }
        }

        pub(crate) fn display_args(&self) -> Vec<String> {
            vec!["cargo-binstall-bootstrap".into(), self.architecture.canonical().into()]
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Status {
        Pending,
        Completed,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Record {
        version: u64,
        status: Status,
        architecture: String,
        target: String,
        tag: String,
        asset_name: String,
        url: String,
        archive_sha256: String,
        executable_sha256: String,
    }

    #[derive(Clone, Debug)]
    struct Release {
        target: String,
        tag: String,
        asset_name: String,
        url: HttpsUrl,
        sha256: String,
    }

    pub(crate) fn execute(host: &Host<'_>, operation: &CargoBinstallBootstrapOperation) -> Result<()> {
        let cargo_home = host
            .value("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".cargo"));
        if !cargo_home.is_absolute() {
            bail!("cargo-binstall managed CARGO_HOME must be absolute");
        }
        let bin = cargo_home.join("bin");
        ensure_managed_directory(&cargo_home)?;
        ensure_managed_directory(&bin)?;
        let destination = bin.join("cargo-binstall");

        let state = ManagedState::open(host, "managers", "cargo-binstall", "cargo-binstall")?;
        let lock = state.acquire_lock()?;
        let record = state
            .read()?
            .map(|bytes| -> Result<Record> {
                let record: Record = super::super::managed_state::parse_strict_json(&bytes)
                    .context("parse strict cargo-binstall managed record")?;
                validate_record(&record)?;
                Ok(record)
            })
            .transpose()?;
        state.validate_lock_entry(&lock)?;
        if let Some(record) = &record {
            if record.architecture != operation.architecture.canonical() {
                bail!("cargo-binstall has managed state for a different architecture");
            }
            if valid_installed(host, &destination, record)? {
                if record.status != Status::Completed {
                    publish_record(&state, &lock, record, Status::Completed)?;
                }
                return Ok(());
            }
            if fs::symlink_metadata(&destination).is_ok() {
                bail!("cargo-binstall managed executable changed at {}", destination.display());
            }
        } else if fs::symlink_metadata(&destination).is_ok() {
            bail!(
                "cargo-binstall executable conflict at {}; refusing to adopt it",
                destination.display()
            );
        }

        let release = match &record {
            Some(record) => release_from_record(record)?,
            None => resolve_release(host, operation.architecture)?,
        };
        let archive = TempPath::new_with_suffix(host, "cargo-binstall", ".tgz")?;
        host.require(
            "cargo-binstall archive download",
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
                archive.path().as_os_str(),
                "--".as_ref(),
                release.url.as_str().as_ref(),
            ],
        )?;
        if sha256_file(archive.path())? != release.sha256 {
            bail!("cargo-binstall archive SHA-256 checksum mismatch");
        }
        let listing = host.require(
            "cargo-binstall archive preflight",
            "tar",
            ["--list", "--gzip", "--file", &archive.path().to_string_lossy()],
        )?;
        validate_archive_listing(&listing.stdout)?;
        let stage = TempDir::new(host, "cargo-binstall-stage")?;
        host.require(
            "cargo-binstall archive extraction",
            "tar",
            [
                "--extract",
                "--gzip",
                "--directory",
                &stage.path().to_string_lossy(),
                "--file",
                &archive.path().to_string_lossy(),
            ],
        )?;
        let staged = stage.path().join("cargo-binstall");
        validate_executable(&staged, "staged cargo-binstall executable")?;
        verify_version(host, &staged, &release.tag)?;
        let executable_sha256 = sha256_file(&staged)?;
        if record
            .as_ref()
            .is_some_and(|record| record.executable_sha256 != executable_sha256)
        {
            bail!("cargo-binstall staged executable changed from its managed record");
        }
        let pending = Record {
            version: VERSION,
            status: Status::Pending,
            architecture: operation.architecture.canonical().into(),
            target: release.target.clone(),
            tag: release.tag.clone(),
            asset_name: release.asset_name.clone(),
            url: release.url.as_str().into(),
            archive_sha256: release.sha256.clone(),
            executable_sha256,
        };
        validate_record(&pending)?;
        publish_record(&state, &lock, &pending, Status::Pending)?;
        publish_executable(&staged, &destination)?;
        if !valid_installed(host, &destination, &pending)? {
            bail!("cargo-binstall publication failed its exact postcondition");
        }
        publish_record(&state, &lock, &pending, Status::Completed)
    }

    fn resolve_release(host: &Host<'_>, architecture: Architecture) -> Result<Release> {
        let target = target(architecture);
        let asset_name = format!("cargo-binstall-{target}.tgz");
        let output = host.require(
            "cargo-binstall release resolution",
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
                "--",
                RELEASE_ENDPOINT,
            ],
        )?;
        let value: Value =
            serde_json::from_slice(&output.stdout).context("parse cargo-binstall GitHub release JSON")?;
        let object = value
            .as_object()
            .context("cargo-binstall GitHub release must be an object")?;
        if object.get("draft") != Some(&Value::Bool(false)) || object.get("prerelease") != Some(&Value::Bool(false)) {
            bail!("cargo-binstall GitHub release must be stable");
        }
        let tag = object
            .get("tag_name")
            .and_then(Value::as_str)
            .context("cargo-binstall GitHub release is missing tag_name")?;
        validate_version_tag(tag)?;
        let assets = object
            .get("assets")
            .and_then(Value::as_array)
            .context("cargo-binstall GitHub release is missing assets")?;
        let mut selected = None;
        for (index, value) in assets.iter().enumerate() {
            let asset = value
                .as_object()
                .with_context(|| format!("cargo-binstall GitHub asset {index} must be an object"))?;
            let name = asset
                .get("name")
                .and_then(Value::as_str)
                .with_context(|| format!("cargo-binstall GitHub asset {index} is missing name"))?;
            validate_asset_name(name)?;
            if name == asset_name {
                if selected.is_some() {
                    bail!("cargo-binstall release contains duplicate target assets");
                }
                let url = HttpsUrl::parse(
                    asset
                        .get("browser_download_url")
                        .and_then(Value::as_str)
                        .context("cargo-binstall target asset is missing browser_download_url")?,
                )?;
                let sha256 = asset
                    .get("digest")
                    .and_then(Value::as_str)
                    .and_then(|value| value.strip_prefix("sha256:"))
                    .context("cargo-binstall target asset is missing its SHA-256 digest")?;
                validate_sha256(sha256)?;
                selected = Some((url, sha256.to_owned()));
            }
        }
        let (url, sha256) = selected.context("cargo-binstall release has no native target asset")?;
        let expected_url = format!("https://github.com/cargo-bins/cargo-binstall/releases/download/{tag}/{asset_name}");
        if url.as_str() != expected_url {
            bail!("cargo-binstall target asset URL does not match its release identity");
        }
        Ok(Release {
            target: target.into(),
            tag: tag.into(),
            asset_name,
            url,
            sha256,
        })
    }

    fn release_from_record(record: &Record) -> Result<Release> {
        validate_record(record)?;
        Ok(Release {
            target: record.target.clone(),
            tag: record.tag.clone(),
            asset_name: record.asset_name.clone(),
            url: HttpsUrl::parse(&record.url)?,
            sha256: record.archive_sha256.clone(),
        })
    }

    fn publish_record(state: &ManagedState, lock: &fs::File, record: &Record, status: Status) -> Result<()> {
        let mut record = record.clone();
        record.status = status;
        validate_record(&record)?;
        state.validate_lock_entry(lock)?;
        state.publish(&serde_json::to_vec(&record).context("serialize cargo-binstall record")?)
    }

    fn validate_record(record: &Record) -> Result<()> {
        if record.version != VERSION {
            bail!("unsupported cargo-binstall managed record version {}", record.version);
        }
        let architecture = Architecture::normalize(&record.architecture)?;
        if architecture.canonical() != record.architecture || target(architecture) != record.target {
            bail!("cargo-binstall managed record architecture or target is not canonical");
        }
        validate_version_tag(&record.tag)?;
        if record.asset_name != format!("cargo-binstall-{}.tgz", record.target) {
            bail!("cargo-binstall managed record asset name is inconsistent");
        }
        let url = HttpsUrl::parse(&record.url)?;
        let expected_url = format!(
            "https://github.com/cargo-bins/cargo-binstall/releases/download/{}/{}",
            record.tag, record.asset_name
        );
        if url.as_str() != record.url || record.url != expected_url {
            bail!("cargo-binstall managed record URL is not canonical");
        }
        validate_sha256(&record.archive_sha256)?;
        validate_sha256(&record.executable_sha256)
    }

    fn target(architecture: Architecture) -> &'static str {
        match architecture {
            Architecture::Amd64 => "x86_64-unknown-linux-musl",
            Architecture::Arm64 => "aarch64-unknown-linux-musl",
            Architecture::Arm32 => "armv7-unknown-linux-musleabihf",
        }
    }

    fn ensure_managed_directory(path: &Path) -> Result<()> {
        let existed = fs::symlink_metadata(path).is_ok();
        fs::create_dir_all(path).with_context(|| format!("create managed directory {}", path.display()))?;
        if !existed {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        let mut metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode() & 0o022 != 0
        {
            let mode = metadata.permissions().mode() & 0o7777 & !0o022;
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
            metadata = fs::symlink_metadata(path)?;
        }
        if !metadata.file_type().is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o022 != 0
        {
            bail!("managed directory {} is unsafe", path.display());
        }
        Ok(())
    }

    fn validate_archive_listing(output: &[u8]) -> Result<()> {
        let output = std::str::from_utf8(output).context("cargo-binstall archive listing is not UTF-8")?;
        let mut executable = 0;
        for entry in output.lines() {
            if entry.is_empty()
                || entry.starts_with('/')
                || entry.split('/').any(|component| component == "..")
                || entry.chars().any(char::is_control)
            {
                bail!("cargo-binstall archive contains an unsafe path");
            }
            executable += usize::from(entry == "cargo-binstall");
        }
        if executable != 1 {
            bail!("cargo-binstall archive must contain one root executable");
        }
        Ok(())
    }

    fn publish_executable(source: &Path, destination: &Path) -> Result<()> {
        let parent = destination
            .parent()
            .context("cargo-binstall destination has no parent")?;
        let mut source = fs::File::open(source)?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        std::io::copy(&mut source, staged.as_file_mut())?;
        staged
            .as_file_mut()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
        staged.as_file_mut().sync_all()?;
        let (file, staged_path) = staged.keep().map_err(|error| error.error)?;
        // Linux rejects execution while any process still holds the inode open for writing.
        drop(file);
        let publication = (|| -> Result<()> {
            fs::hard_link(&staged_path, destination)
                .context("publish cargo-binstall executable without replacement")?;
            fs::remove_file(&staged_path)?;
            fs::File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = fs::remove_file(&staged_path);
        }
        publication
    }

    fn valid_installed(host: &Host<'_>, path: &Path, record: &Record) -> Result<bool> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o7777 != 0o755
            || sha256_file(path)? != record.executable_sha256
        {
            return Ok(false);
        }
        let program = path
            .to_str()
            .with_context(|| format!("cargo-binstall path is not UTF-8: {}", path.display()))?;
        let output = host.require("managed cargo-binstall version postcondition", program, ["-V"])?;
        Ok(parse_version_output(&output.stdout)? == record.tag.trim_start_matches('v'))
    }

    fn verify_version(host: &Host<'_>, path: &Path, tag: &str) -> Result<()> {
        let program = path
            .to_str()
            .with_context(|| format!("cargo-binstall path is not UTF-8: {}", path.display()))?;
        let output = host.require("staged cargo-binstall version", program, ["-V"])?;
        if parse_version_output(&output.stdout)? != tag.trim_start_matches('v') {
            bail!("staged cargo-binstall version does not match its release tag");
        }
        Ok(())
    }

    fn parse_version_output(output: &[u8]) -> Result<String> {
        let text = std::str::from_utf8(output).context("cargo-binstall version output is not UTF-8")?;
        let output = text.strip_suffix('\n').unwrap_or(text);
        if !numeric_version(output) {
            bail!("cargo-binstall returned malformed version output");
        }
        Ok(output.into())
    }

    fn validate_version_tag(value: &str) -> Result<()> {
        if !value.strip_prefix('v').is_some_and(numeric_version) {
            bail!("cargo-binstall release tag must be v followed by a semantic version");
        }
        Ok(())
    }

    fn numeric_version(value: &str) -> bool {
        let parts = value.split('.').collect::<Vec<_>>();
        parts.len() == 3
            && parts.iter().all(|part| {
                !part.is_empty()
                    && part.bytes().all(|byte| byte.is_ascii_digit())
                    && (*part == "0" || !part.starts_with('0'))
            })
    }

    fn validate_asset_name(value: &str) -> Result<()> {
        if value.is_empty()
            || value.bytes().all(|byte| byte == b'.')
            || value.contains(['/', '\\', '\0'])
            || value.chars().any(char::is_control)
        {
            bail!("cargo-binstall asset name must be a safe basename");
        }
        Ok(())
    }

    fn validate_sha256(value: &str) -> Result<()> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("cargo-binstall SHA-256 must be 64 lowercase hexadecimal characters");
        }
        Ok(())
    }

    fn sha256_file(path: &Path) -> Result<String> {
        let mut file = fs::File::open(path)?;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash)?;
        Ok(format!("{:x}", hash.finalize()))
    }

    fn validate_executable(path: &Path, label: &str) -> Result<()> {
        let metadata = fs::symlink_metadata(path).with_context(|| format!("inspect {label}"))?;
        if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.permissions().mode() & 0o111 == 0 {
            bail!("{label} is not a nonempty regular executable");
        }
        Ok(())
    }
}
