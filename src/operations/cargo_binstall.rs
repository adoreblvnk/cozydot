use super::{managed_state::ManagedState, Host, TempDir, TempPath};
use crate::{domain::HttpsUrl, platform::Architecture};
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
const RELEASE_ENDPOINT: &str =
    "https://api.github.com/repos/cargo-bins/cargo-binstall/releases/latest";
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
        vec![
            "cargo-binstall-bootstrap".into(),
            self.architecture.canonical().into(),
        ]
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
            let record: Record = super::managed_state::parse_strict_json(&bytes)
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
            bail!(
                "cargo-binstall managed executable changed at {}",
                destination.display()
            );
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
        [
            "--list",
            "--gzip",
            "--file",
            &archive.path().to_string_lossy(),
        ],
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
    let value: Value = serde_json::from_slice(&output.stdout)
        .context("parse cargo-binstall GitHub release JSON")?;
    let object = value
        .as_object()
        .context("cargo-binstall GitHub release must be an object")?;
    if object.get("draft") != Some(&Value::Bool(false))
        || object.get("prerelease") != Some(&Value::Bool(false))
    {
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
    let expected_url = format!(
        "https://github.com/cargo-bins/cargo-binstall/releases/download/{tag}/{asset_name}"
    );
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

fn publish_record(
    state: &ManagedState,
    lock: &fs::File,
    record: &Record,
    status: Status,
) -> Result<()> {
    let mut record = record.clone();
    record.status = status;
    validate_record(&record)?;
    state.validate_lock_entry(lock)?;
    state.publish(&serde_json::to_vec(&record).context("serialize cargo-binstall record")?)
}

fn validate_record(record: &Record) -> Result<()> {
    if record.version != VERSION {
        bail!(
            "unsupported cargo-binstall managed record version {}",
            record.version
        );
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
        Architecture::Riscv64 => "riscv64gc-unknown-linux-musl",
    }
}

fn ensure_managed_directory(path: &Path) -> Result<()> {
    let existed = fs::symlink_metadata(path).is_ok();
    fs::create_dir_all(path)
        .with_context(|| format!("create managed directory {}", path.display()))?;
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
    let output =
        std::str::from_utf8(output).context("cargo-binstall archive listing is not UTF-8")?;
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
    let output = host.require(
        "managed cargo-binstall version postcondition",
        program,
        ["-V"],
    )?;
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
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        bail!("{label} is not a nonempty regular executable");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustup_created_group_writable_directory_is_hardened() {
        let root = tempfile::tempdir().unwrap();
        let cargo_home = root.path().join(".cargo");
        fs::create_dir(&cargo_home).unwrap();
        fs::set_permissions(&cargo_home, fs::Permissions::from_mode(0o775)).unwrap();

        ensure_managed_directory(&cargo_home).unwrap();

        let mode = fs::symlink_metadata(&cargo_home)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn target_matrix_is_explicit() {
        assert_eq!(target(Architecture::Amd64), "x86_64-unknown-linux-musl");
        assert_eq!(target(Architecture::Arm64), "aarch64-unknown-linux-musl");
        assert_eq!(
            target(Architecture::Arm32),
            "armv7-unknown-linux-musleabihf"
        );
        assert_eq!(
            target(Architecture::Riscv64),
            "riscv64gc-unknown-linux-musl"
        );
    }

    #[test]
    fn archive_and_version_parsers_fail_closed() {
        validate_archive_listing(b"LICENSE\ncargo-binstall\n").unwrap();
        for listing in [
            b"../cargo-binstall\n".as_slice(),
            b"/cargo-binstall\n".as_slice(),
            b"cargo-binstall\ncargo-binstall\n".as_slice(),
            b"LICENSE\n".as_slice(),
        ] {
            assert!(validate_archive_listing(listing).is_err());
        }
        assert_eq!(parse_version_output(b"1.21.0\n").unwrap(), "1.21.0");
        assert!(parse_version_output(b"cargo-binstall 1.21.0\n").is_err());
    }
}
