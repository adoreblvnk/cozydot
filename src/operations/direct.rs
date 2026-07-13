use super::{publish_file, Host, TempPath};
use crate::{config::v1::HttpsUrl, platform::Architecture};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
};

const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectPackageFormat {
    Deb,
    AppImage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectPackageMode {
    EnsurePresent,
    Update,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubRepository(String);

impl GithubRepository {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_repository(&value)?;
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectPackageSelector {
    include: String,
    excludes: Vec<String>,
}

impl DirectPackageSelector {
    pub fn new(include: impl Into<String>, excludes: Vec<String>) -> Result<Self> {
        let selector = Self {
            include: include.into(),
            excludes,
        };
        selector.validate()?;
        Ok(selector)
    }

    fn validate(&self) -> Result<()> {
        validate_wildcard(&self.include, "include selector")?;
        let mut seen = HashSet::new();
        for exclude in &self.excludes {
            validate_wildcard(exclude, "exclude selector")?;
            if !seen.insert(exclude) {
                bail!("direct package exclude selectors must be unique");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectPackageOperation {
    name: String,
    format: DirectPackageFormat,
    provides: Vec<String>,
    repository: GithubRepository,
    architecture: Architecture,
    selector: DirectPackageSelector,
    mode: DirectPackageMode,
}

impl DirectPackageOperation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        format: DirectPackageFormat,
        provides: Vec<String>,
        repository: GithubRepository,
        architecture: Architecture,
        selector: DirectPackageSelector,
        mode: DirectPackageMode,
    ) -> Result<Self> {
        let operation = Self {
            name: name.into(),
            format,
            provides,
            repository,
            architecture,
            selector,
            mode,
        };
        operation.validate()?;
        Ok(operation)
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "direct-package".into(),
            self.name.clone(),
            match self.mode {
                DirectPackageMode::EnsurePresent => "ensure-present",
                DirectPackageMode::Update => "update",
            }
            .into(),
        ]
    }

    fn validate(&self) -> Result<()> {
        validate_definition_name(&self.name)?;
        if self.provides.is_empty() {
            bail!(
                "direct package {:?} must provide at least one executable",
                self.name
            );
        }
        let mut seen = HashSet::new();
        for provide in &self.provides {
            validate_executable(provide)?;
            if !seen.insert(provide) {
                bail!("direct package {:?} provides must be unique", self.name);
            }
        }
        validate_repository(self.repository.as_str())?;
        self.selector.validate()
    }
}

#[derive(Debug)]
struct ReleaseAsset {
    url: HttpsUrl,
    digest: Option<[u8; 32]>,
}

pub(crate) fn execute(host: &Host<'_>, package: &DirectPackageOperation) -> Result<()> {
    package
        .validate()
        .context("validate direct package operation")?;
    match package.format {
        DirectPackageFormat::Deb => install_deb(host, package),
        DirectPackageFormat::AppImage => install_appimage(host, package),
    }
}

fn install_deb(host: &Host<'_>, package: &DirectPackageOperation) -> Result<()> {
    if package.mode == DirectPackageMode::EnsurePresent
        && package
            .provides
            .iter()
            .all(|provide| executable_on_path(host, provide))
    {
        return Ok(());
    }
    let asset = resolve_asset(host, package)?;
    let temporary = download(host, package, &asset, ".deb")?;
    let path = temporary.path().as_os_str();
    host.require(
        "direct Debian preflight",
        "dpkg-deb",
        ["--info".as_ref(), "--".as_ref(), path],
    )?;
    host.require(
        "direct Debian install",
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
    verify_provides(host, package)
}

fn install_appimage(host: &Host<'_>, package: &DirectPackageOperation) -> Result<()> {
    let artifact = managed_artifact(host, package);
    let links = package
        .provides
        .iter()
        .map(|provide| host.home().join(".local/bin").join(provide))
        .collect::<Vec<_>>();
    if package.mode == DirectPackageMode::EnsurePresent
        && valid_managed_artifact(&artifact)
        && package.provides.iter().zip(&links).all(|(provide, link)| {
            executable_on_path(host, provide) || managed_link(link, &artifact)
        })
    {
        return Ok(());
    }

    preflight_links(&links, &artifact)?;
    let asset = resolve_asset(host, package)?;
    let temporary = download(host, package, &asset, ".AppImage")?;
    require_elf(temporary.path(), &package.name)?;
    publish_file(temporary.path(), &artifact, 0o755)
        .with_context(|| format!("publish direct AppImage {:?}", package.name))?;
    for link in &links {
        publish_link(link, &artifact)?;
    }
    verify_appimage(&artifact, &links, package)
}

fn resolve_asset(host: &Host<'_>, package: &DirectPackageOperation) -> Result<ReleaseAsset> {
    let endpoint = format!(
        "https://api.github.com/repos/{}/releases/latest",
        package.repository.as_str()
    );
    let output = host.require(
        "resolve direct package release",
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
    let json = String::from_utf8(output.stdout).context("GitHub release metadata is not UTF-8")?;
    select_asset(&json, package)
}

fn select_asset(input: &str, package: &DirectPackageOperation) -> Result<ReleaseAsset> {
    let value: Value = serde_json::from_str(input).context("parse GitHub release JSON")?;
    let assets = value
        .as_object()
        .context("GitHub release JSON must be an object")?
        .get("assets")
        .context("GitHub release JSON is missing assets")?
        .as_array()
        .context("GitHub release assets must be an array")?;
    let mut named_assets = Vec::with_capacity(assets.len());
    for (index, value) in assets.iter().enumerate() {
        let name = parse_asset_name(value, index)?;
        named_assets.push((index, value, name));
    }
    let matches = named_assets
        .into_iter()
        .filter(|(_, _, name)| {
            wildcard_match(&package.selector.include, name)
                && !package
                    .selector
                    .excludes
                    .iter()
                    .any(|exclude| wildcard_match(exclude, name))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        let names = matches.iter().map(|(_, _, name)| *name).collect::<Vec<_>>();
        bail!(
            "direct package {:?} ({}) selector include {:?}, excludes {:?} matched {} assets: {:?}",
            package.name,
            package.architecture.canonical(),
            package.selector.include,
            package.selector.excludes,
            matches.len(),
            names
        );
    }
    let (index, value, _) = matches[0];
    parse_asset(value, index)
}

fn parse_asset_name(value: &Value, index: usize) -> Result<&str> {
    let asset = value
        .as_object()
        .with_context(|| format!("GitHub release asset {index} must be an object"))?;
    let name = asset
        .get("name")
        .with_context(|| format!("GitHub release asset {index} is missing name"))?
        .as_str()
        .with_context(|| format!("GitHub release asset {index} name must be a string"))?;
    validate_asset_name(name)
        .with_context(|| format!("GitHub release asset {index} has an unsafe name"))?;
    Ok(name)
}

fn parse_asset(value: &Value, index: usize) -> Result<ReleaseAsset> {
    let asset = value
        .as_object()
        .with_context(|| format!("GitHub release asset {index} must be an object"))?;
    let raw_url = asset
        .get("browser_download_url")
        .with_context(|| format!("GitHub release asset {index} is missing browser_download_url"))?
        .as_str()
        .with_context(|| {
            format!("GitHub release asset {index} browser_download_url must be a string")
        })?;
    let url = HttpsUrl::parse(raw_url).with_context(|| {
        format!("GitHub release asset {index} has an invalid browser_download_url")
    })?;
    let digest = match asset.get("digest") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(
            parse_digest(value)
                .with_context(|| format!("GitHub release asset {index} has a malformed digest"))?,
        ),
        Some(_) => bail!("GitHub release asset {index} digest must be a string or null"),
    };
    Ok(ReleaseAsset { url, digest })
}

fn download(
    host: &Host<'_>,
    package: &DirectPackageOperation,
    asset: &ReleaseAsset,
    suffix: &str,
) -> Result<TempPath> {
    let temporary = TempPath::new_with_suffix(host, &package.name, suffix)?;
    host.require(
        "download direct package",
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
            asset.url.as_str().as_ref(),
        ],
    )?;
    let metadata = fs::metadata(temporary.path()).context("inspect direct package download")?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!(
            "direct package {:?} downloaded an empty artifact",
            package.name
        );
    }
    if let Some(expected) = asset.digest {
        let actual = sha256_file(temporary.path())?;
        if actual != expected {
            bail!(
                "direct package {:?} SHA-256 checksum mismatch",
                package.name
            );
        }
    }
    Ok(temporary)
}

fn sha256_file(path: &Path) -> Result<[u8; 32]> {
    let mut file = fs::File::open(path).context("open direct package for checksum")?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).context("read direct package for checksum")?;
    Ok(hash.finalize().into())
}

fn parse_digest(value: &str) -> Result<[u8; 32]> {
    let hex = value
        .strip_prefix("sha256:")
        .context("digest must use sha256:<64-lowercase-hex>")?;
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("digest must use sha256:<64-lowercase-hex>");
    }
    let mut digest = [0; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = (hex_value(pair[0]) << 4) | hex_value(pair[1]);
    }
    Ok(digest)
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!(),
    }
}

fn managed_artifact(host: &Host<'_>, package: &DirectPackageOperation) -> PathBuf {
    host.home()
        .join(".local/share/cozydot/direct")
        .join(format!("{}.AppImage", package.name))
}

fn valid_managed_artifact(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && metadata.len() > 0
            && metadata.permissions().mode() & 0o7777 == 0o755
            && has_elf_magic(path)
    })
}

fn preflight_links(links: &[PathBuf], artifact: &Path) -> Result<()> {
    for link in links {
        match fs::symlink_metadata(link) {
            Ok(metadata) if metadata.file_type().is_symlink() && managed_link(link, artifact) => {}
            Ok(_) => bail!(
                "direct AppImage link conflict at {}; refusing to overwrite it",
                link.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect direct AppImage link"),
        }
    }
    Ok(())
}

fn publish_link(link: &Path, artifact: &Path) -> Result<()> {
    if managed_link(link, artifact) {
        return Ok(());
    }
    let parent = link
        .parent()
        .context("direct AppImage link has no parent")?;
    fs::create_dir_all(parent).context("create direct AppImage link directory")?;
    match symlink(artifact, link) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if managed_link(link, artifact) {
                Ok(())
            } else {
                bail!(
                    "direct AppImage link conflict at {}; refusing to overwrite it",
                    link.display()
                )
            }
        }
        Err(error) => Err(error).context("publish direct AppImage link"),
    }
}

fn managed_link(link: &Path, artifact: &Path) -> bool {
    fs::symlink_metadata(link).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && fs::read_link(link).is_ok_and(|target| target == artifact)
}

fn verify_appimage(
    artifact: &Path,
    links: &[PathBuf],
    package: &DirectPackageOperation,
) -> Result<()> {
    if !valid_managed_artifact(artifact) {
        bail!(
            "direct package {:?} managed AppImage verification failed",
            package.name
        );
    }
    require_elf(artifact, &package.name)?;
    for link in links {
        if !managed_link(link, artifact) {
            bail!(
                "direct package {:?} link verification failed at {}",
                package.name,
                link.display()
            );
        }
    }
    Ok(())
}

fn require_elf(path: &Path, name: &str) -> Result<()> {
    if !has_elf_magic(path) {
        bail!("direct package {name:?} AppImage does not have ELF magic");
    }
    Ok(())
}

fn has_elf_magic(path: &Path) -> bool {
    let mut magic = [0; 4];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && magic == *b"\x7fELF"
}

fn verify_provides(host: &Host<'_>, package: &DirectPackageOperation) -> Result<()> {
    let missing = package
        .provides
        .iter()
        .filter(|provide| !executable_on_path(host, provide))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "direct package {:?} installed but executables remain unavailable: {:?}",
            package.name,
            missing
        );
    }
    Ok(())
}

fn executable_on_path(host: &Host<'_>, name: &str) -> bool {
    host.value("PATH")
        .and_then(|path| {
            std::env::split_paths(&path).find(|directory| {
                fs::metadata(directory.join(name)).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
        })
        .is_some()
}

fn validate_repository(value: &str) -> Result<()> {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    let valid_owner = !owner.is_empty()
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && owner
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && owner
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric);
    let valid_repository = !repository.is_empty()
        && repository
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        && repository
            .bytes()
            .any(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte));
    if parts.next().is_some() || !valid_owner || !valid_repository {
        bail!("GitHub repository must be an owner/repository coordinate");
    }
    Ok(())
}

fn validate_definition_name(value: &str) -> Result<()> {
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        bail!("direct package name must be a safe ASCII definition name");
    }
    Ok(())
}

fn validate_executable(value: &str) -> Result<()> {
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        bail!("direct package provides must contain safe executable basenames");
    }
    Ok(())
}

fn validate_wildcard(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || !value.contains(['*', '?'])
        || value.contains(['/', '\\', '[', ']', '{', '}', '$', '(', ')', '`'])
        || value.chars().any(char::is_control)
    {
        bail!(
            "direct package {field} must be an anchored filename wildcard using only '*' and '?' operators"
        );
    }
    Ok(())
}

fn validate_asset_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.bytes().all(|byte| byte == b'.')
        || value.contains(['/', '\\', '\0'])
        || value.chars().any(char::is_control)
    {
        bail!("asset name must be a safe basename");
    }
    Ok(())
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
                for index in 1..=text.len() {
                    current[index] = previous[index] || current[index - 1];
                }
            }
            '?' => {
                current[1..].copy_from_slice(&previous[..text.len()]);
            }
            literal => {
                for index in 1..=text.len() {
                    current[index] = previous[index - 1] && text[index - 1] == literal;
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

    fn package(architecture: Architecture) -> DirectPackageOperation {
        DirectPackageOperation::new(
            "sample",
            DirectPackageFormat::AppImage,
            vec!["sample".into()],
            GithubRepository::parse("owner/repo").unwrap(),
            architecture,
            DirectPackageSelector::new("sample-?.AppImage", vec!["sample-x*.AppImage".into()])
                .unwrap(),
            DirectPackageMode::EnsurePresent,
        )
        .unwrap()
    }

    #[test]
    fn wildcard_is_anchored_and_matches_unicode_scalars() {
        assert!(wildcard_match("app-*.deb", "app-.deb"));
        assert!(wildcard_match("app-??.deb", "app-ab.deb"));
        assert!(wildcard_match("app-?.deb", "app-é.deb"));
        assert!(wildcard_match("应用-?.deb", "应用-版.deb"));
        assert!(!wildcard_match("app-?.deb", "xapp-a.deb"));
        assert!(!wildcard_match("app-?.deb", "app-ab.deb"));
        assert!(!wildcard_match("app-?.deb", "app-é好.deb"));
    }

    #[test]
    fn selection_includes_then_excludes_and_requires_exactly_one() {
        let package = package(Architecture::Amd64);
        let json = r#"{"assets":[
            {"name":"sample-x.AppImage","browser_download_url":"https://example.test/x"},
            {"name":"sample-a.AppImage","browser_download_url":"https://example.test/a"}
        ]}"#;
        assert_eq!(
            select_asset(json, &package).unwrap().url.as_str(),
            "https://example.test/a"
        );

        let zero = select_asset(r#"{"assets":[]}"#, &package)
            .unwrap_err()
            .to_string();
        assert!(zero.contains("matched 0 assets"));
        let multiple = select_asset(
            r#"{"assets":[
                {"name":"sample-a.AppImage","browser_download_url":"https://example.test/a"},
                {"name":"sample-b.AppImage","browser_download_url":"https://example.test/b"}
            ]}"#,
            &package,
        )
        .unwrap_err()
        .to_string();
        assert!(multiple.contains("matched 2 assets"));
        assert!(multiple.contains("sample-a.AppImage"));
        assert!(multiple.contains("sample-b.AppImage"));

        let unicode_package = DirectPackageOperation::new(
            "sample",
            DirectPackageFormat::AppImage,
            vec!["sample".into()],
            GithubRepository::parse("owner/repo").unwrap(),
            Architecture::Amd64,
            DirectPackageSelector::new("sample-?.AppImage", vec!["sample-坏*.AppImage".into()])
                .unwrap(),
            DirectPackageMode::EnsurePresent,
        )
        .unwrap();
        let unicode = r#"{"assets":[
            {"name":"sample-坏.AppImage","browser_download_url":"https://example.test/bad"},
            {"name":"sample-é.AppImage","browser_download_url":"https://example.test/good"}
        ]}"#;
        assert_eq!(
            select_asset(unicode, &unicode_package)
                .unwrap()
                .url
                .as_str(),
            "https://example.test/good"
        );
    }

    #[test]
    fn selection_diagnostics_use_all_canonical_architectures() {
        for architecture in [
            Architecture::Amd64,
            Architecture::Arm64,
            Architecture::Arm32,
            Architecture::Riscv64,
        ] {
            let error = select_asset(r#"{"assets":[]}"#, &package(architecture))
                .unwrap_err()
                .to_string();
            assert!(error.contains(architecture.canonical()), "{error}");
            assert!(error.contains("sample-?.AppImage"), "{error}");
            assert!(error.contains("sample-x*.AppImage"), "{error}");
        }
    }

    #[test]
    fn malformed_assets_names_urls_and_digests_fail_closed() {
        let package = package(Architecture::Amd64);
        for (json, expected) in [
            ("not-json", "parse GitHub release JSON"),
            (r#"{}"#, "missing assets"),
            (r#"{"assets":{}}"#, "must be an array"),
            (r#"{"assets":[{}]}"#, "missing name"),
            (
                r#"{"assets":[{"name":"../sample-a.AppImage","browser_download_url":"https://example.test/a"}]}"#,
                "unsafe name",
            ),
            (
                r#"{"assets":[{"name":"sample-a.AppImage","browser_download_url":"http://example.test/a"}]}"#,
                "invalid browser_download_url",
            ),
            (
                r#"{"assets":[{"name":"sample-a.AppImage","browser_download_url":"https://user@example.test/a"}]}"#,
                "invalid browser_download_url",
            ),
            (
                r#"{"assets":[{"name":"sample-a.AppImage","browser_download_url":"https://example.test/a#fragment"}]}"#,
                "invalid browser_download_url",
            ),
            (
                r#"{"assets":[{"name":"sample-a.AppImage","browser_download_url":"https://example.test/a","digest":"SHA256:00"}]}"#,
                "malformed digest",
            ),
        ] {
            let error = select_asset(json, &package).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn multiple_matches_are_reported_before_candidate_payload_validation() {
        let package = package(Architecture::Amd64);
        for malformed_payload in [
            r#""browser_download_url":"http://example.test/a""#,
            r#""browser_download_url":"https://example.test/a","digest":"SHA256:00""#,
        ] {
            let json = format!(
                r#"{{"assets":[
                    {{"name":"sample-a.AppImage",{malformed_payload}}},
                    {{"name":"sample-b.AppImage","browser_download_url":"https://example.test/b"}}
                ]}}"#
            );
            let error = select_asset(&json, &package).unwrap_err().to_string();
            assert!(error.contains("matched 2 assets"), "{error}");
            assert!(error.contains("sample-a.AppImage"), "{error}");
            assert!(error.contains("sample-b.AppImage"), "{error}");
            assert!(!error.contains("browser_download_url"), "{error}");
            assert!(!error.contains("digest"), "{error}");
        }
    }

    #[test]
    fn unrelated_asset_payload_is_ignored_but_every_name_is_validated() {
        let package = package(Architecture::Amd64);
        let selected = select_asset(
            r#"{"assets":[
                {"name":"unrelated.txt","browser_download_url":7,"digest":{}},
                {"name":"also-unrelated.txt"},
                {"name":"sample-a.AppImage","browser_download_url":"https://example.test/a"}
            ]}"#,
            &package,
        )
        .unwrap();
        assert_eq!(selected.url.as_str(), "https://example.test/a");

        for (unrelated, expected) in [
            (r#"{}"#, "missing name"),
            (r#"{"name":7}"#, "name must be a string"),
            (r#"{"name":"../unrelated.txt"}"#, "unsafe name"),
        ] {
            let json = format!(
                r#"{{"assets":[{{"name":"sample-a.AppImage","browser_download_url":"https://example.test/a"}},{unrelated}]}}"#
            );
            let error = select_asset(&json, &package).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn digest_accepts_valid_lowercase_sha256_and_absence() {
        let package = package(Architecture::Amd64);
        let hex = "ab".repeat(32);
        let with_digest = format!(
            r#"{{"assets":[{{"name":"sample-a.AppImage","browser_download_url":"https://example.test/a","digest":"sha256:{hex}"}}]}}"#
        );
        assert_eq!(
            select_asset(&with_digest, &package).unwrap().digest,
            Some([0xab; 32])
        );
        assert!(select_asset(
            r#"{"assets":[{"name":"sample-a.AppImage","browser_download_url":"https://example.test/a"}]}"#,
            &package,
        )
        .unwrap()
        .digest
        .is_none());
    }

    #[test]
    fn operation_constructor_rejects_boundary_injections() {
        assert!(GithubRepository::parse("owner/repo/extra").is_err());
        assert!(DirectPackageSelector::new("../*.deb", vec![]).is_err());
        assert!(DirectPackageOperation::new(
            "sample;sh",
            DirectPackageFormat::Deb,
            vec!["sample".into()],
            GithubRepository::parse("owner/repo").unwrap(),
            Architecture::Amd64,
            DirectPackageSelector::new("*.deb", vec![]).unwrap(),
            DirectPackageMode::EnsurePresent,
        )
        .is_err());
    }
}
