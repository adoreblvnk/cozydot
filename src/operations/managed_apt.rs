use super::{repository, Host};
use crate::platform::{Architecture, ManagedAptSources, Platform};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fmt::Write as _,
    path::{Path, PathBuf},
};
use url::Url;

const APT_ROOT: &str = "/etc/apt";
const OWNED_SOURCE: &str = "/etc/apt/sources.list.d/cozydot-base.sources";
const BACKUP_ROOT: &str = "/var/lib/cozydot/apt-source-backups";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedAptSourcesOperation {
    policy: ManagedAptSources,
}

impl ManagedAptSourcesOperation {
    pub fn new(
        distro: String,
        release: String,
        architecture: Architecture,
        components: Vec<String>,
    ) -> Result<Self> {
        let upstream = if distro == "ubuntu" {
            "ubuntu"
        } else {
            "debian"
        };
        let platform = Platform::from_parts(
            distro,
            upstream.into(),
            release,
            "none".into(),
            architecture.canonical(),
        )?;
        let component_refs = components.iter().map(String::as_str).collect::<Vec<_>>();
        let policy = platform.managed_apt_sources(&component_refs)?;
        Ok(Self { policy })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "managed-apt-sources".into(),
            self.policy.distro.clone(),
            self.policy.release.clone(),
            self.policy.architecture.canonical().into(),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceFile {
    path: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceChange {
    path: PathBuf,
    existed: bool,
    original: Vec<u8>,
    replacement: Vec<u8>,
}

pub(crate) fn execute(host: &Host<'_>, operation: &ManagedAptSourcesOperation) -> Result<()> {
    validate_policy(&operation.policy)?;
    preflight_keyring(host, &operation.policy)?;
    let files = inspect_sources(host)?;
    let changes = reconcile(&operation.policy, &files)?;

    for change in &changes {
        backup(host, change)?;
    }
    for change in changes
        .iter()
        .filter(|change| change.path != Path::new(OWNED_SOURCE))
    {
        require_unchanged(host, change)?;
        repository::publish_bytes(
            host,
            &change.path,
            &change.replacement,
            "managed APT migration",
        )?;
    }
    if let Some(change) = changes
        .iter()
        .find(|change| change.path == Path::new(OWNED_SOURCE))
    {
        require_unchanged(host, change)?;
        repository::publish_bytes(
            host,
            &change.path,
            &change.replacement,
            "managed APT publication",
        )?;
    } else {
        repository::sync_parent(host, Path::new(OWNED_SOURCE), "managed APT publication")?;
    }

    let remaining = reconcile(&operation.policy, &inspect_sources(host)?)?;
    if !remaining.is_empty() {
        bail!("managed APT publication did not establish the exact source postcondition");
    }
    Ok(())
}

fn validate_policy(policy: &ManagedAptSources) -> Result<()> {
    let operation = ManagedAptSourcesOperation::new(
        policy.distro.clone(),
        policy.release.clone(),
        policy.architecture,
        policy.components.clone(),
    )?;
    if operation.policy != *policy {
        bail!("managed APT operation policy is not canonical");
    }
    Ok(())
}

fn preflight_keyring(host: &Host<'_>, policy: &ManagedAptSources) -> Result<()> {
    let Some(keyring) = policy
        .stanzas
        .first()
        .map(|stanza| stanza.signed_by.as_str())
    else {
        bail!("managed APT policy has no source stanzas");
    };
    if policy
        .stanzas
        .iter()
        .any(|stanza| stanza.signed_by != keyring)
    {
        bail!("managed APT policy has inconsistent keyrings");
    }
    let output = host.require(
        "managed APT keyring preflight",
        "sudo",
        ["stat", "--format=%f:%s", "--", keyring],
    )?;
    let state = std::str::from_utf8(&output.stdout)
        .context("managed APT keyring stat returned non-UTF-8 output")?
        .trim_end();
    let Some((mode, size)) = state.split_once(':') else {
        bail!("managed APT keyring stat returned malformed output");
    };
    let mode = u32::from_str_radix(mode, 16)
        .context("managed APT keyring stat returned malformed mode")?;
    let size = size
        .parse::<u64>()
        .context("managed APT keyring stat returned malformed size")?;
    if mode & 0o170000 != 0o100000 || size == 0 {
        bail!("managed APT keyring must be a nonempty regular file");
    }
    Ok(())
}

fn inspect_sources(host: &Host<'_>) -> Result<Vec<SourceFile>> {
    for directory in [APT_ROOT, "/etc/apt/sources.list.d"] {
        host.require(
            "managed APT source directory symlink check",
            "sudo",
            ["test", "!", "-L", directory],
        )?;
    }
    let output = host.require(
        "managed APT source discovery",
        "sudo",
        [
            "find",
            APT_ROOT,
            "-xdev",
            "-maxdepth",
            "2",
            "(",
            "-path",
            "/etc/apt/sources.list",
            "-o",
            "-path",
            "/etc/apt/sources.list.d/*.list",
            "-o",
            "-path",
            "/etc/apt/sources.list.d/*.sources",
            ")",
            "-print0",
        ],
    )?;
    let mut paths = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = std::str::from_utf8(raw)
            .context("managed APT source discovery returned a non-UTF-8 path")?;
        let path = validate_source_path(path)?;
        if paths.iter().any(|existing| existing == &path) {
            bail!("managed APT source discovery returned a duplicate path");
        }
        paths.push(path);
    }
    paths.sort();

    let mut files = Vec::new();
    for path in paths {
        let state = host.require(
            "managed APT source inspection",
            "sudo",
            [
                OsStr::new("stat"),
                OsStr::new("--format=%f"),
                OsStr::new("--"),
                path.as_os_str(),
            ],
        )?;
        let mode = std::str::from_utf8(&state.stdout)
            .context("managed APT source stat returned non-UTF-8 output")?
            .trim_end();
        let mode = u32::from_str_radix(mode, 16)
            .context("managed APT source stat returned malformed mode")?;
        if mode & 0o170000 != 0o100000 {
            bail!(
                "managed APT source path is not a regular file: {}",
                path.display()
            );
        }
        let bytes = host
            .require(
                "managed APT source inspection",
                "sudo",
                [OsStr::new("cat"), OsStr::new("--"), path.as_os_str()],
            )?
            .stdout;
        std::str::from_utf8(&bytes)
            .with_context(|| format!("managed APT source is not UTF-8: {}", path.display()))?;
        files.push(SourceFile { path, bytes });
    }
    Ok(files)
}

fn validate_source_path(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path == Path::new("/etc/apt/sources.list") {
        return Ok(path.to_owned());
    }
    if path.parent() != Some(Path::new("/etc/apt/sources.list.d")) {
        bail!("managed APT discovery returned a path outside the source directories");
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        bail!("managed APT discovery returned an invalid source filename");
    };
    if !name.ends_with(".list") && !name.ends_with(".sources")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        bail!("managed APT discovery returned an invalid source filename");
    }
    Ok(path.to_owned())
}

fn reconcile(policy: &ManagedAptSources, files: &[SourceFile]) -> Result<Vec<SourceChange>> {
    let expected = policy.render_deb822().into_bytes();
    let mut changes = Vec::new();
    let mut saw_owned = false;
    for file in files {
        if file.path == Path::new(OWNED_SOURCE) {
            saw_owned = true;
            if file.bytes != expected {
                changes.push(SourceChange {
                    path: file.path.clone(),
                    existed: true,
                    original: file.bytes.clone(),
                    replacement: expected.clone(),
                });
            }
            continue;
        }
        let text = std::str::from_utf8(&file.bytes).context("managed APT source is not UTF-8")?;
        let replacement = match file
            .path
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some("list") | None => reconcile_list(policy, text)?,
            Some("sources") => reconcile_deb822(policy, text)?,
            _ => bail!("managed APT source has an unsupported extension"),
        };
        if replacement.as_bytes() != file.bytes {
            changes.push(SourceChange {
                path: file.path.clone(),
                existed: true,
                original: file.bytes.clone(),
                replacement: replacement.into_bytes(),
            });
        }
    }
    if !saw_owned {
        changes.push(SourceChange {
            path: PathBuf::from(OWNED_SOURCE),
            existed: false,
            original: Vec::new(),
            replacement: expected,
        });
    }
    Ok(changes)
}

fn reconcile_list(policy: &ManagedAptSources, text: &str) -> Result<String> {
    let mut output = String::new();
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let active = body
            .split_once('#')
            .map_or(body, |(before, _)| before)
            .trim();
        if active.split_ascii_whitespace().next() != Some("deb") {
            output.push_str(line);
            continue;
        }
        let entry = parse_list_entry(active).context("parse active one-line APT source")?;
        if official_uri(policy, &entry.uri) && entry.architecture_modifiers {
            bail!("managed APT cannot safely migrate an official one-line source with architecture add/remove modifiers");
        }
        if entry.applies_to(policy.architecture) && official_uri(policy, &entry.uri) {
            validate_official_suites(policy, &entry.suites)?;
            continue;
        }
        output.push_str(line);
    }
    Ok(output)
}

#[derive(Debug)]
struct ListEntry {
    uri: String,
    suites: Vec<String>,
    architectures: Option<Vec<String>>,
    architecture_modifiers: bool,
}

impl ListEntry {
    fn applies_to(&self, architecture: Architecture) -> bool {
        self.architectures
            .as_ref()
            .is_none_or(|values| values.iter().any(|value| value == architecture.debian()))
    }
}

fn parse_list_entry(line: &str) -> Result<ListEntry> {
    let mut rest = line
        .strip_prefix("deb")
        .context("APT source does not start with deb")?
        .trim_start();
    let mut architectures = None;
    let mut architecture_modifiers = false;
    if rest.starts_with('[') {
        let end = rest
            .find(']')
            .context("APT source has unterminated options")?;
        let options = &rest[1..end];
        for option in options.split_ascii_whitespace() {
            if let Some(value) = option.strip_prefix("arch=") {
                architectures = Some(parse_architectures(value)?);
            } else if option.starts_with("arch+=") || option.starts_with("arch-=") {
                architecture_modifiers = true;
            }
        }
        rest = rest[end + 1..].trim_start();
    }
    let fields = rest.split_ascii_whitespace().collect::<Vec<_>>();
    if fields.len() < 3 {
        bail!("APT source is missing URI, suite, or components");
    }
    let uri = normalized_uri(fields[0])?;
    Ok(ListEntry {
        uri,
        suites: vec![fields[1].to_owned()],
        architectures,
        architecture_modifiers,
    })
}

fn parse_architectures(value: &str) -> Result<Vec<String>> {
    let values = value.split(',').map(str::to_owned).collect::<Vec<_>>();
    if values.is_empty()
        || values.iter().any(|value| {
            value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        bail!("APT source has malformed architectures");
    }
    Ok(values)
}

fn reconcile_deb822(policy: &ManagedAptSources, text: &str) -> Result<String> {
    let trailing_newline = text.ends_with('\n');
    let mut output = Vec::new();
    for paragraph in text.split("\n\n") {
        if paragraph.trim().is_empty() {
            output.push(paragraph.to_owned());
            continue;
        }
        let fields = parse_deb822_fields(paragraph)?;
        let enabled = match fields.get("enabled").map(String::as_str) {
            None => true,
            Some(value) if value.eq_ignore_ascii_case("yes") => true,
            Some(value) if value.eq_ignore_ascii_case("no") => false,
            Some(_) => bail!("deb822 source has an invalid Enabled value"),
        };
        let types = fields
            .get("types")
            .map(|value| value.split_ascii_whitespace().collect::<Vec<_>>())
            .unwrap_or_default();
        if !enabled || !types.contains(&"deb") {
            output.push(paragraph.to_owned());
            continue;
        }
        let uris = fields
            .get("uris")
            .context("active deb822 APT source is missing URIs")?
            .split_ascii_whitespace()
            .map(normalized_uri)
            .collect::<Result<Vec<_>>>()?;
        let suites = fields
            .get("suites")
            .context("active deb822 APT source is missing Suites")?
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let architectures = fields.get("architectures").map(|value| {
            value
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
        let applies = architectures.as_ref().is_none_or(|values| {
            values
                .iter()
                .any(|value| value == policy.architecture.debian())
        });
        let official = uris.iter().filter(|uri| official_uri(policy, uri)).count();
        if applies && official != 0 {
            if fields.contains_key("architectures-add")
                || fields.contains_key("architectures-remove")
            {
                bail!("managed APT cannot safely migrate an official deb822 source with architecture add/remove fields");
            }
            if official != uris.len() {
                bail!("managed APT cannot safely split a deb822 stanza mixing official and unrelated URIs");
            }
            validate_official_suites(policy, &suites)?;
            if types.contains(&"deb-src") {
                output.push(replace_deb822_types(paragraph, "deb-src")?);
            }
            continue;
        }
        output.push(paragraph.to_owned());
    }
    let mut result = output.join("\n\n");
    if trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

fn parse_deb822_fields(paragraph: &str) -> Result<BTreeMap<String, String>> {
    let mut fields = BTreeMap::<String, String>::new();
    let mut current: Option<String> = None;
    for line in paragraph.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            let key = current
                .as_ref()
                .context("deb822 continuation has no field")?;
            fields.get_mut(key).unwrap().push(' ');
            fields.get_mut(key).unwrap().push_str(line.trim());
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .context("deb822 source has malformed field")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            bail!("deb822 source has malformed field name");
        }
        let key = name.to_ascii_lowercase();
        if fields
            .insert(key.clone(), value.trim().to_owned())
            .is_some()
        {
            bail!("deb822 source has a duplicate field");
        }
        current = Some(key);
    }
    Ok(fields)
}

fn replace_deb822_types(paragraph: &str, replacement: &str) -> Result<String> {
    let mut result = Vec::new();
    let mut replacing = false;
    let mut found = false;
    for line in paragraph.lines() {
        if line.starts_with([' ', '\t']) && replacing {
            continue;
        }
        replacing = false;
        if let Some((name, _)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("Types") {
                if found {
                    bail!("deb822 source has duplicate Types fields");
                }
                result.push(format!("{name}: {replacement}"));
                found = true;
                replacing = true;
                continue;
            }
        }
        result.push(line.to_owned());
    }
    if !found {
        bail!("deb822 source is missing Types");
    }
    Ok(result.join("\n"))
}

fn normalized_uri(value: &str) -> Result<String> {
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return Ok(value.to_owned());
    }
    let mut url = Url::parse(value).context("APT source URI is malformed")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("APT source URI is unsupported");
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn official_uri(policy: &ManagedAptSources, uri: &str) -> bool {
    let aliases: &[&str] = match policy.distro.as_str() {
        "ubuntu" => &[
            "http://archive.ubuntu.com/ubuntu",
            "https://archive.ubuntu.com/ubuntu",
            "http://security.ubuntu.com/ubuntu",
            "https://security.ubuntu.com/ubuntu",
            "http://ports.ubuntu.com/ubuntu-ports",
            "https://ports.ubuntu.com/ubuntu-ports",
        ],
        "debian" => &[
            "http://deb.debian.org/debian",
            "https://deb.debian.org/debian",
            "http://deb.debian.org/debian-security",
            "https://deb.debian.org/debian-security",
            "http://security.debian.org/debian-security",
            "https://security.debian.org/debian-security",
        ],
        "kali" => &["http://http.kali.org/kali", "https://http.kali.org/kali"],
        _ => &[],
    };
    aliases.contains(&uri)
}

fn validate_official_suites(policy: &ManagedAptSources, suites: &[String]) -> Result<()> {
    let expected = policy
        .stanzas
        .iter()
        .flat_map(|stanza| stanza.suites.iter().cloned())
        .collect::<BTreeSet<_>>();
    if suites.is_empty() || suites.iter().any(|suite| !expected.contains(suite)) {
        bail!("managed APT found an official base source for an unexpected release or pocket");
    }
    Ok(())
}

fn backup(host: &Host<'_>, change: &SourceChange) -> Result<()> {
    if !change.existed {
        return Ok(());
    }
    let digest = Sha256::digest(&change.original);
    let mut digest_hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(digest_hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    let relative = change
        .path
        .strip_prefix("/")
        .context("managed APT source path is not absolute")?;
    let destination = Path::new(BACKUP_ROOT).join(digest_hex).join(relative);
    repository::publish_bytes_with_mode(
        host,
        &destination,
        &change.original,
        "managed APT source backup",
        "0600",
    )
}

fn require_unchanged(host: &Host<'_>, change: &SourceChange) -> Result<()> {
    if !change.existed {
        host.require(
            "managed APT owned source absence check",
            "sudo",
            [
                OsStr::new("test"),
                OsStr::new("!"),
                OsStr::new("-e"),
                change.path.as_os_str(),
            ],
        )?;
        host.require(
            "managed APT owned source symlink check",
            "sudo",
            [
                OsStr::new("test"),
                OsStr::new("!"),
                OsStr::new("-L"),
                change.path.as_os_str(),
            ],
        )?;
        return Ok(());
    }
    let current = host.require(
        "managed APT source prepublication check",
        "sudo",
        [OsStr::new("cat"), OsStr::new("--"), change.path.as_os_str()],
    )?;
    if current.stdout != change.original {
        bail!("managed APT source changed concurrently before publication");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(distro: &str, release: &str, architecture: Architecture) -> ManagedAptSources {
        ManagedAptSourcesOperation::new(
            distro.into(),
            release.into(),
            architecture,
            vec!["main".into()],
        )
        .unwrap()
        .policy
    }

    fn file(path: &str, text: &str) -> SourceFile {
        SourceFile {
            path: path.into(),
            bytes: text.as_bytes().to_vec(),
        }
    }

    #[test]
    fn list_migration_removes_only_active_official_binary_entries() {
        let policy = policy("ubuntu", "noble", Architecture::Amd64);
        let input = "# base\ndeb http://archive.ubuntu.com/ubuntu noble main universe\ndeb-src http://archive.ubuntu.com/ubuntu noble main\ndeb [arch=arm64] http://archive.ubuntu.com/ubuntu noble main\ndeb [signed-by=/vendor.gpg] https://vendor.example/ubuntu noble main\n";
        let output = reconcile_list(&policy, input).unwrap();
        assert_eq!(
            output,
            "# base\ndeb-src http://archive.ubuntu.com/ubuntu noble main\ndeb [arch=arm64] http://archive.ubuntu.com/ubuntu noble main\ndeb [signed-by=/vendor.gpg] https://vendor.example/ubuntu noble main\n"
        );
    }

    #[test]
    fn list_migration_handles_tabs_and_preserves_local_and_transport_sources() {
        let policy = policy("ubuntu", "noble", Architecture::Amd64);
        let input = "deb\thttps://archive.ubuntu.com/ubuntu\tnoble\tmain\ndeb file:/srv/mirror noble main\ndeb tor+http://mirror.example/ubuntu noble main\n";
        assert_eq!(
            reconcile_list(&policy, input).unwrap(),
            "deb file:/srv/mirror noble main\ndeb tor+http://mirror.example/ubuntu noble main\n"
        );
    }

    #[test]
    fn deb822_migration_preserves_source_types_unknown_fields_and_unrelated_stanzas() {
        let policy = policy("debian", "trixie", Architecture::Amd64);
        let input = "Types: deb deb-src\nURIs: https://deb.debian.org/debian\nSuites: trixie trixie-updates\nComponents: main\nX-Vendor: keep me\n\nTypes: deb\nURIs: https://vendor.example/apt\nSuites: trixie\nComponents: main\nX-Repolib-Name: Vendor\n";
        let output = reconcile_deb822(&policy, input).unwrap();
        assert!(output.contains("Types: deb-src"));
        assert!(output.contains("X-Vendor: keep me"));
        assert!(output.contains("https://vendor.example/apt"));
        assert!(output.contains("X-Repolib-Name: Vendor"));
    }

    #[test]
    fn mixed_official_stanzas_and_old_release_sources_fail_before_changes() {
        let policy = policy("ubuntu", "noble", Architecture::Amd64);
        assert!(reconcile_deb822(
            &policy,
            "Types: deb\nURIs: https://archive.ubuntu.com/ubuntu https://mirror.local/ubuntu\nSuites: noble\nComponents: main\n"
        )
        .unwrap_err()
        .to_string()
        .contains("mixing official"));
        assert!(reconcile_list(
            &policy,
            "deb https://archive.ubuntu.com/ubuntu jammy main\n"
        )
        .unwrap_err()
        .to_string()
        .contains("unexpected release"));
        assert!(reconcile_list(
            &policy,
            "deb [arch-=amd64] https://archive.ubuntu.com/ubuntu noble main\n"
        )
        .unwrap_err()
        .to_string()
        .contains("architecture add/remove"));
    }

    #[test]
    fn reconciliation_is_retry_safe_and_publishes_owned_source_last() {
        let policy = policy("ubuntu", "noble", Architecture::Amd64);
        let files = vec![
            file(
                "/etc/apt/sources.list",
                "deb http://archive.ubuntu.com/ubuntu noble main\n",
            ),
            file(
                "/etc/apt/sources.list.d/vendor.sources",
                "Types: deb\nURIs: https://vendor.example/apt\nSuites: stable\nComponents: main\nX-Unknown: preserved\n",
            ),
        ];
        let changes = reconcile(&policy, &files).unwrap();
        assert_eq!(changes.last().unwrap().path, Path::new(OWNED_SOURCE));
        let converged = vec![
            SourceFile {
                path: changes[0].path.clone(),
                bytes: changes[0].replacement.clone(),
            },
            files[1].clone(),
            SourceFile {
                path: PathBuf::from(OWNED_SOURCE),
                bytes: policy.render_deb822().into_bytes(),
            },
        ];
        assert!(reconcile(&policy, &converged).unwrap().is_empty());
    }
}
