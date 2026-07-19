use super::{Host, TempPath};
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

struct Candidate {
    url: HttpsUrl,
    effective: Option<BinarySha256>,
}
struct Downloaded {
    temporary: TempPath,
    actual_sha256: String,
}

pub(crate) fn execute(host: &Host, operation: &BinaryPackageOperation) -> Result<()> {
    let appimage_expectation = if operation.format == BinaryPackageFormat::AppImage {
        let artifact = appimage::data_artifact(host, operation);
        let expectation = capture_publication_expectation(&artifact)?;
        preflight_appimage(host, operation)?;
        if is_acceptable_live_state(host, operation)? {
            if expectation.verify_identity(&artifact)? {
                return Ok(());
            } else {
                bail!(
                    "TOCTOU conflict detected: live-state inspection passed but destination identity changed for {}",
                    artifact.display()
                );
            }
        }
        Some(expectation)
    } else {
        None
    };

    if operation.format == BinaryPackageFormat::Deb && is_acceptable_live_state(host, operation)? {
        return Ok(());
    }

    let candidate = resolve(host, operation)?;

    let downloaded = download_candidate(host, operation, candidate)?;

    if operation.format == BinaryPackageFormat::Deb {
        preflight_deb(host, operation, &downloaded)?;
        install_deb(host, operation, &downloaded)?;
    } else {
        require_elf(downloaded.temporary.path(), &operation.name)?;
        install_appimage(
            host,
            operation,
            &downloaded.actual_sha256,
            &downloaded,
            appimage_expectation.unwrap(),
        )?;
    }

    if !postconditions(host, operation, &downloaded.actual_sha256)? {
        bail!("binary package postconditions failed");
    }
    Ok(())
}

fn configured_checksum(source: &BinarySourceOperation) -> Option<String> {
    match source {
        BinarySourceOperation::GithubLatest { sha256, .. } => sha256.as_ref().map(|s| s.as_hex()),
        BinarySourceOperation::ChecksummedUrl { sha256, .. } => Some(sha256.as_hex()),
    }
}

fn is_acceptable_live_state(host: &Host, operation: &BinaryPackageOperation) -> Result<bool> {
    if operation.mode != BinaryPackageMode::EnsurePresent {
        return Ok(false);
    }
    match operation.format {
        BinaryPackageFormat::Deb => Ok(operation.commands.iter().all(|name| executable_on_path(host, name))),
        BinaryPackageFormat::AppImage => {
            let artifact = data_artifact(host, operation);
            let has_valid_artifact = if let Some(digest) = configured_checksum(&operation.source) {
                valid_artifact(&artifact, &digest)?
            } else {
                valid_artifact_unchecksummed(&artifact)?
            };
            if !has_valid_artifact {
                return Ok(false);
            }
            Ok(command_links(host, operation)
                .iter()
                .all(|link| managed_link(link, &artifact)))
        }
    }
}

fn valid_artifact_unchecksummed(path: &Path) -> Result<bool> {
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
        && has_elf_magic(path))
}

#[derive(Debug)]
pub(crate) enum PublicationExpectation {
    Absent,
    Existing(fs::File),
}

impl PublicationExpectation {
    pub(crate) fn verify_identity(&self, path: &Path) -> Result<bool> {
        match self {
            PublicationExpectation::Absent => match fs::symlink_metadata(path) {
                Ok(_) => Ok(false),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
                Err(e) => Err(e.into()),
            },
            PublicationExpectation::Existing(file) => match fs::symlink_metadata(path) {
                Ok(current_metadata) => {
                    let expected_metadata = file.metadata().context("inspect expected destination descriptor")?;
                    Ok(current_metadata.dev() == expected_metadata.dev()
                        && current_metadata.ino() == expected_metadata.ino())
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e.into()),
            },
        }
    }
}

pub(crate) fn capture_publication_expectation(destination: &Path) -> Result<PublicationExpectation> {
    let open_result = rustix::fs::open(
        destination,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    );

    match open_result {
        Ok(fd) => {
            let file = std::fs::File::from(fd);
            let metadata = file.metadata().context("inspect existing destination descriptor")?;
            let uid = rustix::process::geteuid().as_raw();
            if !metadata.file_type().is_file() {
                bail!("destination is not a regular file");
            }
            if metadata.uid() != uid {
                bail!("destination owner is not current user");
            }
            if metadata.nlink() != 1 {
                bail!("destination has multiple hard links");
            }
            if (metadata.permissions().mode() & 0o7777) != 0o755 {
                bail!("destination permissions are not exact 0755");
            }
            Ok(PublicationExpectation::Existing(file))
        }
        Err(rustix::io::Errno::NOENT) => Ok(PublicationExpectation::Absent),
        Err(other_err) => Err(other_err).context("open existing destination file"),
    }
}

pub(crate) fn publish_executable(source: &Path, destination: &Path, expectation: PublicationExpectation) -> Result<()> {
    let parent = destination.parent().context("destination has no parent")?;
    let mut source_file = fs::File::open(source)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut source_file, staged.as_file_mut())?;
    staged
        .as_file_mut()
        .set_permissions(fs::Permissions::from_mode(0o755))?;
    staged.as_file_mut().sync_all()?;
    let (file, staged_path) = staged.keep().map_err(|error| error.error)?;
    // Linux rejects execution while any process still holds the inode open for writing.
    drop(file);

    let result = (|| -> Result<()> {
        match expectation {
            PublicationExpectation::Existing(file) => {
                let metadata = file.metadata().context("inspect existing destination descriptor")?;
                let validated_dev = metadata.dev();
                let validated_ino = metadata.ino();

                rustix::fs::renameat_with(
                    rustix::fs::CWD,
                    &staged_path,
                    rustix::fs::CWD,
                    destination,
                    rustix::fs::RenameFlags::EXCHANGE,
                )
                .context("atomically exchange staged executable with destination")?;

                let displaced_metadata =
                    fs::symlink_metadata(&staged_path).context("inspect displaced destination file")?;
                let displaced_dev = displaced_metadata.dev();
                let displaced_ino = displaced_metadata.ino();

                if displaced_dev != validated_dev || displaced_ino != validated_ino {
                    let rollback_res = rustix::fs::renameat_with(
                        rustix::fs::CWD,
                        &staged_path,
                        rustix::fs::CWD,
                        destination,
                        rustix::fs::RenameFlags::EXCHANGE,
                    );
                    if let Err(rollback_err) = rollback_res {
                        bail!(
                            "TOCTOU conflict detected (device/inode mismatch) and rollback exchange failed: {rollback_err:#}"
                        );
                    }
                    bail!("TOCTOU conflict detected (device/inode mismatch), safely rolled back");
                }

                drop(file);

                fs::remove_file(&staged_path).context("remove displaced old file")?;
                fs::File::open(parent)?.sync_all().context("sync parent directory")?;
            }
            PublicationExpectation::Absent => {
                rustix::fs::renameat_with(
                    rustix::fs::CWD,
                    &staged_path,
                    rustix::fs::CWD,
                    destination,
                    rustix::fs::RenameFlags::NOREPLACE,
                )
                .context("atomically publish new executable to vacant destination")?;

                fs::File::open(parent)?.sync_all().context("sync parent directory")?;
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&staged_path);
    }
    result
}

mod appimage {
    use super::*;

    pub(super) fn data_artifact(host: &Host, operation: &BinaryPackageOperation) -> PathBuf {
        host.value("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".local/share"))
            .join("cozydot/binaries")
            .join(format!("{}.AppImage", operation.name))
    }
    pub(super) fn command_links(host: &Host, operation: &BinaryPackageOperation) -> Vec<PathBuf> {
        let root = host
            .value("XDG_BIN_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".local/bin"));
        operation.commands.iter().map(|name| root.join(name)).collect()
    }
    pub(super) fn preflight_appimage(host: &Host, operation: &BinaryPackageOperation) -> Result<()> {
        let artifact = data_artifact(host, operation);
        ensure_secure_data_parent(host, &artifact)?;
        ensure_secure_command_root(host)?;
        match fs::symlink_metadata(&artifact) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                if !valid_artifact_unchecksummed(&artifact)? {
                    bail!("binary AppImage artifact conflict at {}", artifact.display());
                }
            }
            Err(error) => return Err(error.into()),
        }
        for link in command_links(host, operation) {
            preflight_link(&link, &artifact, false)?;
        }
        Ok(())
    }
    pub(super) fn install_appimage(
        host: &Host,
        operation: &BinaryPackageOperation,
        actual_sha256: &str,
        downloaded: &Downloaded,
        expectation: PublicationExpectation,
    ) -> Result<()> {
        let artifact = data_artifact(host, operation);
        ensure_secure_data_parent(host, &artifact)?;
        if !valid_artifact(&artifact, actual_sha256)? {
            let source = downloaded.temporary.path();
            require_elf(source, &operation.name)?;
            super::publish_executable(source, &artifact, expectation)?;
        }
        let links = command_links(host, operation);
        for link in &links {
            publish_link(link, &artifact)?;
        }
        verify_appimage(&artifact, &links, actual_sha256)?;
        Ok(())
    }
    fn ensure_secure_data_parent(host: &Host, artifact: &Path) -> Result<()> {
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
    fn ensure_secure_command_root(host: &Host) -> Result<()> {
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
    pub(super) fn managed_link(link: &Path, artifact: &Path) -> bool {
        fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_symlink())
            && fs::read_link(link).is_ok_and(|target| target == artifact)
    }
    fn verify_appimage(artifact: &Path, links: &[PathBuf], actual_sha256: &str) -> Result<()> {
        if !valid_artifact(artifact, actual_sha256)? || links.iter().any(|link| !managed_link(link, artifact)) {
            bail!("binary AppImage verification failed");
        }
        Ok(())
    }
    pub(super) fn valid_artifact(path: &Path, digest: &str) -> Result<bool> {
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
    pub(super) fn postconditions(host: &Host, operation: &BinaryPackageOperation, actual_sha256: &str) -> Result<bool> {
        match operation.format {
            BinaryPackageFormat::Deb => Ok(operation.commands.iter().all(|name| executable_on_path(host, name))),
            BinaryPackageFormat::AppImage => {
                let artifact = data_artifact(host, operation);
                Ok(valid_artifact(&artifact, actual_sha256)?
                    && command_links(host, operation)
                        .iter()
                        .all(|link| managed_link(link, &artifact)))
            }
        }
    }
    pub(super) fn verify_commands(host: &Host, operation: &BinaryPackageOperation) -> Result<()> {
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
    pub(super) fn executable_on_path(host: &Host, name: &str) -> bool {
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
    pub(super) fn has_elf_magic(path: &Path) -> bool {
        let mut magic = [0; 4];
        fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"\x7fELF"
    }
}

mod source {
    use super::*;

    pub(super) fn resolve(host: &Host, operation: &BinaryPackageOperation) -> Result<Candidate> {
        match &operation.source {
            BinarySourceOperation::ChecksummedUrl { url, sha256 } => Ok(Candidate {
                url: url.clone(),
                effective: Some(*sha256),
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
        let (index, asset, _) = matches[0];
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
            url,
            effective: declared.or(api),
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
    pub(super) fn download_candidate(
        host: &Host,
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
        if candidate.effective.is_some_and(|expected| expected != actual) {
            bail!("binary package SHA-256 checksum mismatch");
        }
        Ok(Downloaded {
            temporary,
            actual_sha256: actual.as_hex(),
        })
    }
}

use appimage::*;
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

pub(crate) mod cargo_binstall {

    use super::super::{Host, TempDir, TempPath};
    use crate::{config::HttpsUrl, platform::Architecture};
    use anyhow::{bail, Context, Result};
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::{Path, PathBuf},
    };

    const RELEASE_ENDPOINT: &str = "https://api.github.com/repos/cargo-bins/cargo-binstall/releases/latest";
    const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
    const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
    const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

    #[derive(Clone, Debug)]
    struct Release {
        tag: String,
        url: HttpsUrl,
        sha256: String,
    }

    pub(crate) fn execute(host: &Host, architecture: Architecture) -> Result<()> {
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

        let release = resolve_release(host, architecture)?;

        let expectation = super::capture_publication_expectation(&destination)?;

        if valid_installed(host, &destination, &release.tag)? {
            if expectation.verify_identity(&destination)? {
                return Ok(());
            } else {
                bail!("TOCTOU conflict detected: cargo-binstall version/live-state inspection passed but destination identity changed for {}", destination.display());
            }
        }

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
        super::publish_executable(&staged, &destination, expectation)?;
        if !valid_installed(host, &destination, &release.tag)? {
            bail!("cargo-binstall publication failed its exact postcondition");
        }
        Ok(())
    }

    fn resolve_release(host: &Host, architecture: Architecture) -> Result<Release> {
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
            tag: tag.into(),
            url,
            sha256,
        })
    }

    fn valid_installed(host: &Host, path: &Path, tag: &str) -> Result<bool> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o7777 != 0o755
        {
            return Ok(false);
        }
        let program = path
            .to_str()
            .with_context(|| format!("cargo-binstall path is not UTF-8: {}", path.display()))?;
        let output = host.require("managed cargo-binstall version postcondition", program, ["-V"])?;
        Ok(parse_version_output(&output.stdout)? == tag.trim_start_matches('v'))
    }

    fn verify_version(host: &Host, path: &Path, tag: &str) -> Result<()> {
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
}
