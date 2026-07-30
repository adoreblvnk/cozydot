use super::{Host, TempPath};
use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

const SOURCES_DIRECTORY: &str = "/etc/apt/sources.list.d";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AptRepositoryOperation {
    name: String,
    key_url: String,
    source_url: String,
    architecture: Architecture,
    suite: Option<String>,
    components: Vec<String>,
    path: Option<String>,
    keyring_path: PathBuf,
    source_list_path: PathBuf,
}

impl AptRepositoryOperation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        key_url: String,
        source_url: String,
        architecture: Architecture,
        suite: Option<String>,
        components: Vec<String>,
        path: Option<String>,
        keyring_path: PathBuf,
    ) -> Result<Self> {
        let name = name.into();
        validate_keyring_path(&keyring_path)?;
        for value in std::iter::once(source_url.as_str())
            .chain(suite.as_deref())
            .chain(components.iter().map(String::as_str))
            .chain(path.as_deref())
        {
            if value.chars().any(char::is_control) {
                bail!("APT repository source values must fit on one line and contain no control characters");
            }
        }
        let source_list_path = PathBuf::from(format!("{SOURCES_DIRECTORY}/{name}.list"));
        Ok(Self { name, key_url, source_url, architecture, suite, components, path, keyring_path, source_list_path })
    }

    pub fn render_source(&self) -> String {
        let prefix = format!(
            "deb [arch={} signed-by={}] {} ",
            self.architecture.debian(),
            self.keyring_path.display(),
            self.source_url
        );
        match &self.path {
            None => format!(
                "{prefix}{} {}\n",
                self.suite.as_deref().expect("validated suite/components repository"),
                self.components.join(" ")
            ),
            Some(path) => format!("{prefix}{path}\n"),
        }
    }
}

fn validate_keyring_path(path: &Path) -> Result<()> {
    let parent = path.parent().context("APT repository key path has no parent")?;
    if parent != Path::new("/etc/apt/keyrings") && parent != Path::new("/usr/share/keyrings") {
        bail!("APT repository key path must be a direct child of /etc/apt/keyrings or /usr/share/keyrings");
    }
    let name = path.file_name().and_then(|name| name.to_str()).context("APT repository key path has no filename")?;
    if !matches!(path.extension().and_then(|extension| extension.to_str()), Some("asc" | "gpg"))
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        bail!("APT repository key path must name a safe .asc or .gpg file");
    }
    Ok(())
}

pub(crate) fn execute(host: &Host, operation: &AptRepositoryOperation) -> Result<()> {
    let keyring_path_str = operation.keyring_path.to_str().context("keyring path is not UTF-8")?;
    let preserve_armor = keyring_path_str.ends_with(".asc");

    let key = processed_key(host, &operation.key_url, preserve_armor)?;
    let source = operation.render_source().into_bytes();

    super::privileged_file::publish_bytes(host, &operation.keyring_path, &key, "APT repository key publication")?;
    super::privileged_file::publish_bytes(
        host,
        &operation.source_list_path,
        &source,
        "APT repository source publication",
    )?;

    Ok(())
}

fn processed_key(host: &Host, url: &str, preserve_armor: bool) -> Result<Vec<u8>> {
    let downloaded = TempPath::new(host, "repository-key-download")?;
    host.require(
        "repository key download",
        "curl",
        [
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--tlsv1.2",
            "--retry",
            "3",
            "--retry-all-errors",
            "--output",
            &downloaded.path().to_string_lossy(),
            "--",
            url,
        ],
    )?;

    let downloaded_bytes = fs::read(downloaded.path()).context("read downloaded repository key")?;
    if downloaded_bytes.is_empty() {
        bail!("repository key download produced empty output");
    }

    let binary_keyring = TempPath::new_with_suffix(host, "repository-key-binary", ".gpg")?;

    host.require(
        "repository key conversion",
        "gpg",
        [
            "--no-options",
            "--batch",
            "--yes",
            "--output",
            &binary_keyring.path().to_string_lossy(),
            "--dearmor",
            &downloaded.path().to_string_lossy(),
        ],
    )?;

    // Validate the keyring contains a public key
    let inspection = host.require(
        "repository key validation",
        "gpg",
        [
            "--no-options",
            "--batch",
            "--no-default-keyring",
            "--keyring",
            &binary_keyring.path().to_string_lossy(),
            "--with-colons",
            "--list-keys",
        ],
    )?;
    if !inspection
        .stdout
        .split(|byte| *byte == b'\n')
        .any(|line| line.strip_prefix(b"pub:").is_some_and(|fields| !fields.is_empty()))
    {
        bail!("repository key validation found no public key");
    }

    if preserve_armor {
        Ok(downloaded_bytes)
    } else {
        let bytes = fs::read(binary_keyring.path()).context("read dearmored repository key")?;
        if bytes.is_empty() {
            bail!("repository key conversion produced empty output");
        }
        Ok(bytes)
    }
}

pub(crate) mod managed_apt {

    use super::super::{Host, privileged_file};
    use crate::platform::{Architecture, ManagedAptSources};
    use anyhow::{Context, Result, bail};
    use sha2::{Digest, Sha256};
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::OsStr,
        path::{Path, PathBuf},
    };
    use url::Url;

    const APT_ROOT: &str = "/etc/apt";
    const OWNED_SOURCE: &str = "/etc/apt/sources.list.d/cozydot-base.sources";
    const BACKUP_ROOT: &str = "/var/lib/cozydot/apt-source-backups";

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

    pub(crate) fn execute(host: &Host, policy: &ManagedAptSources) -> Result<()> {
        preflight_keyring(host, policy)?;
        let files = inspect_sources(host)?;
        let changes = reconcile(policy, &files)?;

        for change in &changes {
            backup(host, change)?;
        }
        for change in changes.iter().filter(|change| change.path != Path::new(OWNED_SOURCE)) {
            require_unchanged(host, change)?;
            privileged_file::publish_bytes(host, &change.path, &change.replacement, "managed APT migration")?;
        }
        if let Some(change) = changes.iter().find(|change| change.path == Path::new(OWNED_SOURCE)) {
            require_unchanged(host, change)?;
            privileged_file::publish_bytes(host, &change.path, &change.replacement, "managed APT publication")?;
        } else {
            privileged_file::sync_parent(host, Path::new(OWNED_SOURCE), "managed APT publication")?;
        }

        let remaining = reconcile(policy, &inspect_sources(host)?)?;
        if !remaining.is_empty() {
            bail!("managed APT publication did not establish the exact source postcondition");
        }
        Ok(())
    }

    fn preflight_keyring(host: &Host, policy: &ManagedAptSources) -> Result<()> {
        let Some(keyring) = policy.stanzas.first().map(|stanza| stanza.signed_by.as_str()) else {
            bail!("managed APT policy has no source stanzas");
        };
        if policy.stanzas.iter().any(|stanza| stanza.signed_by != keyring) {
            bail!("managed APT policy has inconsistent keyrings");
        }
        let output = host.require(
            "managed APT keyring preflight",
            "sudo",
            ["stat", "--dereference", "--format=%f:%s", "--", keyring],
        )?;
        let state = std::str::from_utf8(&output.stdout)
            .context("managed APT keyring stat returned non-UTF-8 output")?
            .trim_end();
        let Some((mode, size)) = state.split_once(':') else {
            bail!("managed APT keyring stat returned malformed output");
        };
        let mode = u32::from_str_radix(mode, 16).context("managed APT keyring stat returned malformed mode")?;
        let size = size.parse::<u64>().context("managed APT keyring stat returned malformed size")?;
        if mode & 0o170000 != 0o100000 || size == 0 {
            bail!("managed APT keyring must be a nonempty regular file");
        }
        Ok(())
    }

    fn inspect_sources(host: &Host) -> Result<Vec<SourceFile>> {
        for directory in [APT_ROOT, "/etc/apt/sources.list.d"] {
            host.require("managed APT source directory symlink check", "sudo", ["test", "!", "-L", directory])?;
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
        let mut paths = BTreeSet::new();
        for raw in output.stdout.split(|byte| *byte == 0) {
            if raw.is_empty() {
                continue;
            }
            let path = std::str::from_utf8(raw).context("managed APT source discovery returned a non-UTF-8 path")?;
            let path = validate_source_path(path)?;
            if !paths.insert(path) {
                bail!("managed APT source discovery returned a duplicate path");
            }
        }

        let mut files = Vec::new();
        for path in paths {
            let state = host.require(
                "managed APT source inspection",
                "sudo",
                [OsStr::new("stat"), OsStr::new("--format=%f"), OsStr::new("--"), path.as_os_str()],
            )?;
            let mode = std::str::from_utf8(&state.stdout)
                .context("managed APT source stat returned non-UTF-8 output")?
                .trim_end();
            let mode = u32::from_str_radix(mode, 16).context("managed APT source stat returned malformed mode")?;
            if mode & 0o170000 != 0o100000 {
                bail!("managed APT source path is not a regular file: {}", path.display());
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
            || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
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
            let replacement = match file.path.extension().and_then(|extension| extension.to_str()) {
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
            let active = body.split_once('#').map_or(body, |(before, _)| before).trim();
            if active.split_ascii_whitespace().next() != Some("deb") {
                output.push_str(line);
                continue;
            }
            let entry = parse_list_entry(active).context("parse active one-line APT source")?;
            if official_uri(policy, &entry.uri) && entry.architecture_modifiers {
                bail!(
                    "managed APT cannot safely migrate an official one-line source with architecture add/remove modifiers"
                );
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
            self.architectures.as_ref().is_none_or(|values| values.iter().any(|value| value == architecture.debian()))
        }
    }

    fn parse_list_entry(line: &str) -> Result<ListEntry> {
        let mut rest = line.strip_prefix("deb").context("APT source does not start with deb")?.trim_start();
        let mut architectures = None;
        let mut architecture_modifiers = false;
        if rest.starts_with('[') {
            let end = rest.find(']').context("APT source has unterminated options")?;
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
        Ok(ListEntry { uri, suites: vec![fields[1].to_owned()], architectures, architecture_modifiers })
    }

    fn parse_architectures(value: &str) -> Result<Vec<String>> {
        let values = value.split(',').map(str::to_owned).collect::<Vec<_>>();
        if values.iter().any(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())) {
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
            let types =
                fields.get("types").map(|value| value.split_ascii_whitespace().collect::<Vec<_>>()).unwrap_or_default();
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
            let architectures = fields
                .get("architectures")
                .map(|value| value.split_ascii_whitespace().map(str::to_owned).collect::<Vec<_>>());
            let applies = architectures
                .as_ref()
                .is_none_or(|values| values.iter().any(|value| value == policy.architecture.debian()));
            let official = uris.iter().filter(|uri| official_uri(policy, uri)).count();
            if applies && official != 0 {
                if fields.contains_key("architectures-add") || fields.contains_key("architectures-remove") {
                    bail!(
                        "managed APT cannot safely migrate an official deb822 source with architecture add/remove fields"
                    );
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
                let key = current.as_ref().context("deb822 continuation has no field")?;
                let value = fields.get_mut(key).context("deb822 continuation field disappeared")?;
                value.push(' ');
                value.push_str(line.trim());
                continue;
            }
            let (name, value) = line.split_once(':').context("deb822 source has malformed field")?;
            if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') {
                bail!("deb822 source has malformed field name");
            }
            let key = name.to_ascii_lowercase();
            if fields.insert(key.clone(), value.trim().to_owned()).is_some() {
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
            if let Some((name, _)) = line.split_once(':')
                && name.eq_ignore_ascii_case("Types")
            {
                if found {
                    bail!("deb822 source has duplicate Types fields");
                }
                result.push(format!("{name}: {replacement}"));
                found = true;
                replacing = true;
                continue;
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
            _ => &[],
        };
        aliases.contains(&uri)
    }

    fn validate_official_suites(policy: &ManagedAptSources, suites: &[String]) -> Result<()> {
        let expected = policy.stanzas.iter().flat_map(|stanza| stanza.suites.iter().cloned()).collect::<BTreeSet<_>>();
        if suites.is_empty() || suites.iter().any(|suite| !expected.contains(suite)) {
            bail!("managed APT found an official base source for an unexpected release or pocket");
        }
        Ok(())
    }

    fn backup(host: &Host, change: &SourceChange) -> Result<()> {
        if !change.existed {
            return Ok(());
        }
        let digest_hex = format!("{:x}", Sha256::digest(&change.original));
        let relative = change.path.strip_prefix("/").context("managed APT source path is not absolute")?;
        let destination = Path::new(BACKUP_ROOT).join(digest_hex).join(relative);
        privileged_file::publish_bytes_with_mode(
            host,
            &destination,
            &change.original,
            "managed APT source backup",
            "0600",
        )
    }

    fn require_unchanged(host: &Host, change: &SourceChange) -> Result<()> {
        if !change.existed {
            host.require(
                "managed APT owned source absence check",
                "sudo",
                [OsStr::new("test"), OsStr::new("!"), OsStr::new("-e"), change.path.as_os_str()],
            )?;
            host.require(
                "managed APT owned source symlink check",
                "sudo",
                [OsStr::new("test"), OsStr::new("!"), OsStr::new("-L"), change.path.as_os_str()],
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
}
