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

pub(crate) mod debian_components {
    use super::super::{Host, privileged_file};
    use anyhow::{Context, Result, bail};
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::OsStr,
        path::{Path, PathBuf},
    };
    use url::Url;

    const APT_ROOT: &str = "/etc/apt";
    const REQUIRED_COMPONENTS: [&str; 4] = ["main", "contrib", "non-free", "non-free-firmware"];

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SourceFile {
        path: PathBuf,
        bytes: Vec<u8>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SourceChange {
        path: PathBuf,
        original: Vec<u8>,
        replacement: Vec<u8>,
    }

    struct Reconciliation {
        changes: Vec<SourceChange>,
        matched_entries: usize,
    }

    pub(crate) fn execute(host: &Host, release: &str) -> Result<()> {
        validate_release(release)?;
        let reconciliation = reconcile(release, &inspect_sources(host)?)?;
        if reconciliation.matched_entries == 0 {
            return Ok(());
        }

        for change in &reconciliation.changes {
            require_unchanged(host, change)?;
            privileged_file::publish_bytes(
                host,
                &change.path,
                &change.replacement,
                "Debian APT component publication",
            )?;
        }

        let remaining = reconcile(release, &inspect_sources(host)?)?;
        if remaining.matched_entries == 0 || !remaining.changes.is_empty() {
            bail!("Debian APT component publication did not establish the required postcondition");
        }
        Ok(())
    }

    fn validate_release(release: &str) -> Result<()> {
        if !matches!(release, "bookworm" | "trixie") {
            bail!("unsupported Debian release {release:?}; supported releases are bookworm and trixie");
        }
        Ok(())
    }

    fn inspect_sources(host: &Host) -> Result<Vec<SourceFile>> {
        for directory in [APT_ROOT, "/etc/apt/sources.list.d"] {
            host.require("Debian APT source directory symlink check", "sudo", ["test", "!", "-L", directory])?;
        }
        let output = host.require(
            "Debian APT source discovery",
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
            let path = std::str::from_utf8(raw).context("Debian APT source discovery returned a non-UTF-8 path")?;
            let path = validate_source_path(path)?;
            if !paths.insert(path) {
                bail!("Debian APT source discovery returned a duplicate path");
            }
        }

        let mut files = Vec::new();
        for path in paths {
            let state = host.require(
                "Debian APT source inspection",
                "sudo",
                [OsStr::new("stat"), OsStr::new("--format=%f"), OsStr::new("--"), path.as_os_str()],
            )?;
            let mode = std::str::from_utf8(&state.stdout)
                .context("Debian APT source stat returned non-UTF-8 output")?
                .trim_end();
            let mode = u32::from_str_radix(mode, 16).context("Debian APT source stat returned malformed mode")?;
            if mode & 0o170000 != 0o100000 {
                bail!("Debian APT source path is not a regular file: {}", path.display());
            }
            let bytes = host
                .require(
                    "Debian APT source inspection",
                    "sudo",
                    [OsStr::new("cat"), OsStr::new("--"), path.as_os_str()],
                )?
                .stdout;
            std::str::from_utf8(&bytes)
                .with_context(|| format!("Debian APT source is not UTF-8: {}", path.display()))?;
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
            bail!("Debian APT discovery returned a path outside the source directories");
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            bail!("Debian APT discovery returned an invalid source filename");
        };
        if !name.ends_with(".list") && !name.ends_with(".sources")
            || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            bail!("Debian APT discovery returned an invalid source filename");
        }
        Ok(path.to_owned())
    }

    fn reconcile(release: &str, files: &[SourceFile]) -> Result<Reconciliation> {
        let mut changes = Vec::new();
        let mut matched_entries = 0;
        for file in files {
            let text = std::str::from_utf8(&file.bytes).context("Debian APT source is not UTF-8")?;
            let (replacement, matched) = match file.path.extension().and_then(|extension| extension.to_str()) {
                Some("list") | None => reconcile_list(release, text)?,
                Some("sources") => reconcile_deb822(release, text)?,
                _ => bail!("Debian APT source has an unsupported extension"),
            };
            matched_entries += matched;
            if replacement.as_bytes() != file.bytes {
                changes.push(SourceChange {
                    path: file.path.clone(),
                    original: file.bytes.clone(),
                    replacement: replacement.into_bytes(),
                });
            }
        }
        Ok(Reconciliation { changes, matched_entries })
    }

    fn reconcile_list(release: &str, text: &str) -> Result<(String, usize)> {
        let mut output = String::new();
        let mut matched = 0;
        for line in text.split_inclusive('\n') {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let comment_start = body.find('#').unwrap_or(body.len());
            let active = body[..comment_start].trim();
            if !matches!(active.split_ascii_whitespace().next(), Some("deb" | "deb-src")) {
                output.push_str(line);
                continue;
            }
            let entry = parse_list_entry(active).context("parse active one-line APT source")?;
            if !official_uri(&entry.uri) || !supported_suite(release, entry.suite) {
                output.push_str(line);
                continue;
            }
            matched += 1;
            let missing = missing_components(entry.components.iter().copied());
            if missing.is_empty() {
                output.push_str(line);
                continue;
            }
            let active_end = body[..comment_start].trim_end().len();
            output.push_str(&body[..active_end]);
            output.push(' ');
            output.push_str(&missing.join(" "));
            output.push_str(&body[active_end..]);
            if line.ends_with('\n') {
                output.push('\n');
            }
        }
        Ok((output, matched))
    }

    #[derive(Debug)]
    struct ListEntry<'a> {
        uri: String,
        suite: &'a str,
        components: Vec<&'a str>,
    }

    fn parse_list_entry(line: &str) -> Result<ListEntry<'_>> {
        let mut rest = line
            .strip_prefix("deb-src")
            .or_else(|| line.strip_prefix("deb"))
            .context("APT source does not start with deb or deb-src")?
            .trim_start();
        if rest.starts_with('[') {
            let end = rest.find(']').context("APT source has unterminated options")?;
            rest = rest[end + 1..].trim_start();
        }
        let fields = rest.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() < 3 {
            bail!("APT source is missing URI, suite, or components");
        }
        Ok(ListEntry { uri: normalized_uri(fields[0])?, suite: fields[1], components: fields[2..].to_vec() })
    }

    fn reconcile_deb822(release: &str, text: &str) -> Result<(String, usize)> {
        let trailing_newline = text.ends_with('\n');
        let mut output = Vec::new();
        let mut matched = 0;
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
            if !enabled || !types.iter().any(|value| matches!(*value, "deb" | "deb-src")) {
                output.push(paragraph.to_owned());
                continue;
            }
            let uris = fields
                .get("uris")
                .context("active deb822 APT source is missing URIs")?
                .split_ascii_whitespace()
                .map(normalized_uri)
                .collect::<Result<Vec<_>>>()?;
            let official = uris.iter().filter(|uri| official_uri(uri)).count();
            if official == 0 {
                output.push(paragraph.to_owned());
                continue;
            }
            if official != uris.len() {
                bail!("Debian APT cannot safely modify a deb822 stanza mixing official and unrelated URIs");
            }
            let suites = fields
                .get("suites")
                .context("active deb822 APT source is missing Suites")?
                .split_ascii_whitespace()
                .collect::<Vec<_>>();
            if !suites.iter().any(|suite| supported_suite(release, suite)) {
                output.push(paragraph.to_owned());
                continue;
            }
            if suites.iter().any(|suite| !supported_suite(release, suite)) {
                bail!("Debian APT found an official source mixing supported and unexpected suites");
            }
            matched += 1;
            let components = fields.get("components").context("active deb822 APT source is missing Components")?;
            let mut values = components.split_ascii_whitespace().collect::<Vec<_>>();
            let missing = missing_components(values.iter().copied());
            if missing.is_empty() {
                output.push(paragraph.to_owned());
                continue;
            }
            values.extend(missing);
            output.push(replace_deb822_field(paragraph, "Components", &values.join(" "))?);
        }
        let mut result = output.join("\n\n");
        if trailing_newline && !result.ends_with('\n') {
            result.push('\n');
        }
        Ok((result, matched))
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

    fn replace_deb822_field(paragraph: &str, field: &str, replacement: &str) -> Result<String> {
        let mut result = Vec::new();
        let mut replacing = false;
        let mut found = false;
        for line in paragraph.lines() {
            if line.starts_with([' ', '\t']) && replacing {
                continue;
            }
            replacing = false;
            if let Some((name, _)) = line.split_once(':')
                && name.eq_ignore_ascii_case(field)
            {
                if found {
                    bail!("deb822 source has duplicate {field} fields");
                }
                result.push(format!("{name}: {replacement}"));
                found = true;
                replacing = true;
                continue;
            }
            result.push(line.to_owned());
        }
        if !found {
            bail!("deb822 source is missing {field}");
        }
        Ok(result.join("\n"))
    }

    fn missing_components<'a>(existing: impl Iterator<Item = &'a str>) -> Vec<&'static str> {
        let existing = existing.collect::<BTreeSet<_>>();
        REQUIRED_COMPONENTS.into_iter().filter(|component| !existing.contains(component)).collect()
    }

    fn supported_suite(release: &str, suite: &str) -> bool {
        suite == release
            || suite == format!("{release}-updates")
            || suite == format!("{release}-backports")
            || suite == format!("{release}-security")
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

    fn official_uri(uri: &str) -> bool {
        [
            "http://deb.debian.org/debian",
            "https://deb.debian.org/debian",
            "http://deb.debian.org/debian-security",
            "https://deb.debian.org/debian-security",
            "http://security.debian.org/debian-security",
            "https://security.debian.org/debian-security",
        ]
        .contains(&uri)
    }

    fn require_unchanged(host: &Host, change: &SourceChange) -> Result<()> {
        let current = host.require(
            "Debian APT source prepublication check",
            "sudo",
            [OsStr::new("cat"), OsStr::new("--"), change.path.as_os_str()],
        )?;
        if current.stdout != change.original {
            bail!("Debian APT source changed concurrently before publication");
        }
        Ok(())
    }
}
