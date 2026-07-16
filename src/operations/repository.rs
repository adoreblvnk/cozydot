use super::{Host, TempPath};
use crate::{domain::HttpsUrl, platform::Architecture};
use anyhow::{bail, Context, Result};
use serde::{
    de::Visitor,
    de::{MapAccess, SeqAccess},
    Deserialize, Deserializer, Serialize,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fmt, fs,
    fs::File,
    io::Write,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

const SOURCES_DIRECTORY: &str = "/etc/apt/sources.list.d";
const KEYRINGS_DIRECTORY: &str = "/etc/apt/keyrings";
const MANAGED_STATE_VERSION: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AptRepositoryToken(String);

impl AptRepositoryToken {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        Self::parse(value)
    }

    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value != "*"
            && (value.is_empty()
                || !value.as_bytes()[0].is_ascii_lowercase()
                    && !value.as_bytes()[0].is_ascii_digit()
                || !value.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._+-".contains(&byte)
                }))
        {
            bail!("invalid APT repository token {value:?}");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AptRepositoryPath(String);

impl AptRepositoryPath {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        Self::parse(value)
    }

    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let valid = value == "./"
            || value.ends_with('/')
                && !value.starts_with('/')
                && !value.contains('\\')
                && !value.contains("//")
                && value[..value.len() - 1]
                    .split('/')
                    .all(valid_definition_name);
        if !valid {
            bail!("invalid exact APT repository path {value:?}");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AptRepositorySourceLayout {
    SuiteComponents {
        suite: AptRepositoryToken,
        components: Vec<AptRepositoryToken>,
    },
    ExactPath(AptRepositoryPath),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AptRepositoryOperation {
    name: String,
    filename_stem: String,
    key_url: HttpsUrl,
    source_url: HttpsUrl,
    architecture: Architecture,
    layout: AptRepositorySourceLayout,
    keyring_path: PathBuf,
    source_list_path: PathBuf,
}

impl AptRepositoryOperation {
    pub fn new(
        name: impl Into<String>,
        filename_stem: impl Into<String>,
        key_url: HttpsUrl,
        source_url: HttpsUrl,
        architecture: Architecture,
        layout: AptRepositorySourceLayout,
    ) -> Result<Self> {
        let name = name.into();
        let filename_stem = filename_stem.into();
        if !valid_definition_name(&name) {
            bail!("invalid APT repository name {name:?}");
        }
        if !valid_filename_stem(&filename_stem) {
            bail!("invalid APT repository filename stem {filename_stem:?}");
        }
        let expected_stem = repository_stem(&name);
        if filename_stem != expected_stem {
            bail!(
                "APT repository filename stem {filename_stem:?} does not match name-derived stem {expected_stem:?}"
            );
        }
        validate_canonical_url(&key_url, "key")?;
        validate_canonical_url(&source_url, "source")?;
        validate_layout(&layout)?;

        let keyring_path =
            PathBuf::from(format!("{KEYRINGS_DIRECTORY}/cozydot-{filename_stem}.gpg"));
        let source_list_path =
            PathBuf::from(format!("{SOURCES_DIRECTORY}/cozydot-{filename_stem}.list"));
        validate_destination(
            keyring_path
                .to_str()
                .context("repository keyring path is not UTF-8")?,
            KEYRINGS_DIRECTORY,
            ".gpg",
        )?;
        validate_destination(
            source_list_path
                .to_str()
                .context("repository source-list path is not UTF-8")?,
            SOURCES_DIRECTORY,
            ".list",
        )?;

        Ok(Self {
            name,
            filename_stem,
            key_url,
            source_url,
            architecture,
            layout,
            keyring_path,
            source_list_path,
        })
    }

    pub fn keyring_path(&self) -> &Path {
        &self.keyring_path
    }

    pub fn source_list_path(&self) -> &Path {
        &self.source_list_path
    }

    pub fn render_source(&self) -> String {
        let prefix = format!(
            "deb [arch={} signed-by={}] {} ",
            self.architecture.debian(),
            self.keyring_path.display(),
            self.source_url.as_str()
        );
        match &self.layout {
            AptRepositorySourceLayout::SuiteComponents { suite, components } => format!(
                "{prefix}{} {}\n",
                suite.as_str(),
                components
                    .iter()
                    .map(AptRepositoryToken::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            AptRepositorySourceLayout::ExactPath(path) => {
                format!("{prefix}{}\n", path.as_str())
            }
        }
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        vec![
            "apt-repository".into(),
            self.name.clone(),
            self.keyring_path.display().to_string(),
            self.source_list_path.display().to_string(),
        ]
    }
}

pub(crate) fn execute(host: &Host<'_>, operation: &AptRepositoryOperation) -> Result<()> {
    validate_operation(operation)?;
    let declaration = ManagedDeclaration::from_operation(operation);
    let state = ManagedState::open(host, &operation.filename_stem)?;
    let lock = state.acquire_lock()?;
    let record = state.read_record()?;
    state.validate_lock_entry(&lock)?;

    match record
        .as_ref()
        .map(|record| (&record.status, &record.declaration))
    {
        None => {
            require_absent(
                host,
                &operation.keyring_path,
                "repository keyring preflight",
            )?;
            require_absent(
                host,
                &operation.source_list_path,
                "repository source preflight",
            )?;
        }
        Some((ManagedStatus::PendingInitial | ManagedStatus::PendingUpdate, recorded))
            if recorded != &declaration =>
        {
            bail!("APT repository has a pending managed record for a different declaration")
        }
        Some((_, recorded)) => validate_record_destinations(recorded, &operation.filename_stem)?,
    }

    let completed_matching = record.as_ref().is_some_and(|record| {
        record.status == ManagedStatus::Completed && record.declaration == declaration
    });
    let initial_publication = match record.as_ref().map(|record| &record.status) {
        None => {
            state.publish_record(&ManagedRecord {
                version: MANAGED_STATE_VERSION,
                status: ManagedStatus::PendingInitial,
                declaration: declaration.clone(),
            })?;
            true
        }
        Some(ManagedStatus::PendingInitial) => true,
        Some(ManagedStatus::PendingUpdate | ManagedStatus::Completed) => false,
    };

    let key = normalized_key(host, operation.key_url.as_str())?;
    let source = operation.render_source().into_bytes();
    if completed_matching
        && inspect_owned_file(host, &operation.keyring_path, "APT repository key")?.as_deref()
            == Some(key.as_slice())
        && inspect_owned_file(host, &operation.source_list_path, "APT repository source")?
            .as_deref()
            == Some(source.as_slice())
    {
        return Ok(());
    }
    if record
        .as_ref()
        .is_some_and(|record| record.status == ManagedStatus::Completed)
    {
        state.publish_record(&ManagedRecord {
            version: MANAGED_STATE_VERSION,
            status: ManagedStatus::PendingUpdate,
            declaration: declaration.clone(),
        })?;
    }
    converge_owned_bytes(
        host,
        &operation.keyring_path,
        &key,
        "APT repository key",
        initial_publication,
    )?;
    converge_owned_bytes(
        host,
        &operation.source_list_path,
        &source,
        "APT repository source",
        initial_publication,
    )?;

    state.validate_lock_entry(&lock)?;
    state.publish_record(&ManagedRecord {
        version: MANAGED_STATE_VERSION,
        status: ManagedStatus::Completed,
        declaration,
    })
}

fn validate_operation(operation: &AptRepositoryOperation) -> Result<()> {
    let rebuilt = AptRepositoryOperation::new(
        operation.name.clone(),
        operation.filename_stem.clone(),
        operation.key_url.clone(),
        operation.source_url.clone(),
        operation.architecture,
        operation.layout.clone(),
    )?;
    if rebuilt != *operation {
        bail!("APT repository operation is not canonical");
    }
    Ok(())
}

fn validate_layout(layout: &AptRepositorySourceLayout) -> Result<()> {
    match layout {
        AptRepositorySourceLayout::SuiteComponents { suite, components } => {
            AptRepositoryToken::parse(suite.as_str())?;
            if suite.as_str() == "system" {
                bail!(
                    "APT repository suite 'system' must be resolved before operation construction"
                );
            }
            if components.is_empty() {
                bail!("APT repository components must be nonempty");
            }
            let mut seen = BTreeSet::new();
            for component in components {
                AptRepositoryToken::parse(component.as_str())?;
                if component.as_str() == "system" {
                    bail!("APT repository component 'system' is reserved for suite resolution");
                }
                if !seen.insert(component.as_str()) {
                    bail!(
                        "duplicate APT repository component {:?}",
                        component.as_str()
                    );
                }
            }
        }
        AptRepositorySourceLayout::ExactPath(path) => {
            AptRepositoryPath::parse(path.as_str())?;
        }
    }
    Ok(())
}

fn valid_definition_name(value: &str) -> bool {
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
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn repository_stem(name: &str) -> String {
    let mut stem = String::new();
    let mut separator = false;
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() {
            if separator && !stem.is_empty() {
                stem.push('-');
            }
            stem.push((byte as char).to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    stem
}

fn valid_filename_stem(value: &str) -> bool {
    !value.is_empty()
        && value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value
            .as_bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_canonical_url(url: &HttpsUrl, kind: &str) -> Result<()> {
    let parsed = HttpsUrl::parse(url.as_str())
        .with_context(|| format!("APT repository {kind} URL is invalid"))?;
    if parsed != *url {
        bail!("APT repository {kind} URL is not canonical");
    }
    Ok(())
}

fn normalized_key(host: &Host<'_>, url: &str) -> Result<Vec<u8>> {
    let downloaded = TempPath::new(host, "repository-key-download")?;
    let normalized = TempPath::new(host, "repository-key-normalized")?;
    host.require(
        "repository key download",
        "curl",
        [
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--retry",
            "3",
            "--retry-all-errors",
            "--output",
            &downloaded.path().to_string_lossy(),
            url,
        ],
    )?;
    host.require(
        "repository key conversion",
        "gpg",
        [
            "--no-options",
            "--batch",
            "--yes",
            "--dearmor",
            "--output",
            &normalized.path().to_string_lossy(),
            &downloaded.path().to_string_lossy(),
        ],
    )?;
    let bytes = fs::read(normalized.path()).context("read normalized repository key")?;
    if bytes.is_empty() {
        bail!("repository key conversion produced empty output");
    }
    let inspection = host.require(
        "repository key validation",
        "gpg",
        [
            "--no-options",
            "--batch",
            "--no-default-keyring",
            "--keyring",
            &normalized.path().to_string_lossy(),
            "--list-keys",
            "--with-colons",
        ],
    )?;
    if !inspection.stdout.split(|byte| *byte == b'\n').any(|line| {
        line.strip_prefix(b"pub:")
            .is_some_and(|fields| !fields.is_empty())
    }) {
        bail!("repository key validation found no public key");
    }
    Ok(bytes)
}

fn converge_owned_bytes(
    host: &Host<'_>,
    path: &Path,
    expected: &[u8],
    label: &str,
    no_replace: bool,
) -> Result<()> {
    let current = inspect_owned_file(host, path, label)?;
    if no_replace && current.as_deref().is_some_and(|bytes| bytes != expected) {
        bail!("{label} initial-publication destination collision");
    }
    if current.is_none() || !no_replace && current.as_deref() != Some(expected) {
        publish_bytes_with_policy(
            host,
            path,
            expected,
            &format!("{label} publication"),
            no_replace,
        )?;
    }
    let final_bytes = inspect_owned_file(host, path, label)?
        .with_context(|| format!("{label} postcondition is missing"))?;
    if final_bytes != expected {
        bail!("{label} publication did not establish exact bytes");
    }
    Ok(())
}

fn inspect_owned_file(host: &Host<'_>, path: &Path, label: &str) -> Result<Option<Vec<u8>>> {
    let path_arg = path.as_os_str();
    let absent = host
        .run(
            "sudo",
            [
                OsStr::new("test"),
                OsStr::new("!"),
                OsStr::new("-e"),
                path_arg,
            ],
        )?
        .status
        .success();
    let not_symlink = host
        .run(
            "sudo",
            [
                OsStr::new("test"),
                OsStr::new("!"),
                OsStr::new("-L"),
                path_arg,
            ],
        )?
        .status
        .success();
    if absent && not_symlink {
        return Ok(None);
    }
    if !not_symlink {
        bail!("{label} destination is a symlink");
    }
    let state = host.require(
        &format!("{label} inspection"),
        "sudo",
        [
            OsStr::new("stat"),
            OsStr::new("--format=%f:%u:%g"),
            OsStr::new("--"),
            path_arg,
        ],
    )?;
    let state = std::str::from_utf8(&state.stdout)
        .with_context(|| format!("{label} stat returned non-UTF-8 output"))?
        .trim_end();
    let mut fields = state.split(':');
    let mode = fields
        .next()
        .and_then(|value| u32::from_str_radix(value, 16).ok());
    let uid = fields.next().and_then(|value| value.parse::<u32>().ok());
    let gid = fields.next().and_then(|value| value.parse::<u32>().ok());
    if fields.next().is_some()
        || mode.is_none_or(|mode| mode & 0o170000 != 0o100000 || mode & 0o7777 != 0o0644)
        || uid != Some(0)
        || gid != Some(0)
    {
        bail!("{label} has mismatched type, ownership, or permissions");
    }
    Ok(Some(
        host.require(
            &format!("{label} inspection"),
            "sudo",
            [OsStr::new("cat"), OsStr::new("--"), path_arg],
        )?
        .stdout,
    ))
}

fn require_absent(host: &Host<'_>, path: &Path, label: &str) -> Result<()> {
    let path_arg = path.as_os_str();
    for kind in ["-e", "-L"] {
        let output = host.run(
            "sudo",
            [
                OsStr::new("test"),
                OsStr::new("!"),
                OsStr::new(kind),
                path_arg,
            ],
        )?;
        if !output.status.success() {
            bail!(
                "{label}: unmanaged destination conflict at {}",
                path.display()
            );
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct ManagedRecord {
    version: u64,
    status: ManagedStatus,
    declaration: ManagedDeclaration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ManagedStatus {
    PendingInitial,
    PendingUpdate,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct ManagedDeclaration {
    name: String,
    filename_stem: String,
    key_url: String,
    source_url: String,
    architecture: String,
    layout: ManagedLayout,
    keyring_path: String,
    source_list_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ManagedLayout {
    SuiteComponents {
        suite: String,
        components: Vec<String>,
    },
    ExactPath {
        path: String,
    },
}

impl ManagedDeclaration {
    fn from_operation(operation: &AptRepositoryOperation) -> Self {
        let layout = match &operation.layout {
            AptRepositorySourceLayout::SuiteComponents { suite, components } => {
                ManagedLayout::SuiteComponents {
                    suite: suite.as_str().into(),
                    components: components
                        .iter()
                        .map(|component| component.as_str().into())
                        .collect(),
                }
            }
            AptRepositorySourceLayout::ExactPath(path) => ManagedLayout::ExactPath {
                path: path.as_str().into(),
            },
        };
        Self {
            name: operation.name.clone(),
            filename_stem: operation.filename_stem.clone(),
            key_url: operation.key_url.as_str().into(),
            source_url: operation.source_url.as_str().into(),
            architecture: operation.architecture.canonical().into(),
            layout,
            keyring_path: operation.keyring_path.display().to_string(),
            source_list_path: operation.source_list_path.display().to_string(),
        }
    }
}

struct ManagedState(super::managed_state::ManagedState);

impl ManagedState {
    fn open(host: &Host<'_>, stem: &str) -> Result<Self> {
        Ok(Self(super::managed_state::ManagedState::open(
            host,
            "apt-repositories",
            stem,
            "APT repository",
        )?))
    }

    fn acquire_lock(&self) -> Result<File> {
        self.0.acquire_lock()
    }

    fn validate_lock_entry(&self, lock: &File) -> Result<()> {
        self.0.validate_lock_entry(lock)
    }

    fn read_record(&self) -> Result<Option<ManagedRecord>> {
        self.0
            .read()?
            .map(|bytes| {
                let value: StrictJson = serde_json::from_slice(&bytes)
                    .context("parse strict APT repository managed record")?;
                let record = parse_managed_record(value)
                    .context("validate APT repository managed record")?;
                validate_managed_declaration(&record.declaration)
                    .context("validate APT repository managed declaration")?;
                Ok(record)
            })
            .transpose()
    }

    fn publish_record(&self, record: &ManagedRecord) -> Result<()> {
        self.0.publish(
            &serde_json::to_vec(record).context("serialize APT repository managed record")?,
        )
    }
}

fn validate_managed_declaration(declaration: &ManagedDeclaration) -> Result<()> {
    let key_url = HttpsUrl::parse(&declaration.key_url)?;
    let source_url = HttpsUrl::parse(&declaration.source_url)?;
    if key_url.as_str() != declaration.key_url || source_url.as_str() != declaration.source_url {
        bail!("managed declaration URLs are not canonical");
    }
    let architecture = Architecture::normalize(&declaration.architecture)?;
    if architecture.canonical() != declaration.architecture {
        bail!("managed declaration architecture is not canonical");
    }
    let layout = match &declaration.layout {
        ManagedLayout::SuiteComponents { suite, components } => {
            AptRepositorySourceLayout::SuiteComponents {
                suite: AptRepositoryToken::parse(suite)?,
                components: components
                    .iter()
                    .map(AptRepositoryToken::parse)
                    .collect::<Result<Vec<_>>>()?,
            }
        }
        ManagedLayout::ExactPath { path } => {
            AptRepositorySourceLayout::ExactPath(AptRepositoryPath::parse(path)?)
        }
    };
    let operation = AptRepositoryOperation::new(
        declaration.name.clone(),
        declaration.filename_stem.clone(),
        key_url,
        source_url,
        architecture,
        layout,
    )?;
    if ManagedDeclaration::from_operation(&operation) != *declaration {
        bail!("managed declaration does not match its canonical operation identity");
    }
    Ok(())
}

fn validate_record_destinations(record: &ManagedDeclaration, stem: &str) -> Result<()> {
    let key = format!("{KEYRINGS_DIRECTORY}/cozydot-{stem}.gpg");
    let source = format!("{SOURCES_DIRECTORY}/cozydot-{stem}.list");
    if record.filename_stem != stem
        || record.keyring_path != key
        || record.source_list_path != source
    {
        bail!("APT repository managed record has mismatched deterministic destinations");
    }
    Ok(())
}

#[derive(Debug)]
enum StrictJson {
    Null,
    Bool,
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonVisitor)
    }
}

struct StrictJsonVisitor;

impl<'de> Visitor<'de> for StrictJsonVisitor {
    type Value = StrictJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("strict JSON")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        let _ = value;
        Ok(StrictJson::Bool)
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(StrictJson::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(StrictJson::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(StrictJson::Number)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(StrictJson::String(value.into()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(StrictJson::String(value))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(StrictJson::Null)
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(StrictJson::Null)
    }

    fn visit_seq<A>(self, mut values: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut result = Vec::new();
        while let Some(value) = values.next_element()? {
            result.push(value);
        }
        Ok(StrictJson::Array(result))
    }

    fn visit_map<A>(self, mut values: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut result = BTreeMap::new();
        while let Some((key, value)) = values.next_entry::<String, StrictJson>()? {
            if result.insert(key.clone(), value).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON key {key:?}"
                )));
            }
        }
        Ok(StrictJson::Object(result))
    }
}

fn parse_managed_record(value: StrictJson) -> Result<ManagedRecord> {
    let mut object = object(value, "record")?;
    let version = number(take(&mut object, "version")?, "version")?;
    if version != MANAGED_STATE_VERSION {
        bail!("unsupported managed record version {version}");
    }
    let status = match string(take(&mut object, "status")?, "status")?.as_str() {
        "pending_initial" => ManagedStatus::PendingInitial,
        "pending_update" => ManagedStatus::PendingUpdate,
        "completed" => ManagedStatus::Completed,
        other => bail!("invalid managed record status {other:?}"),
    };
    let declaration = parse_declaration(take(&mut object, "declaration")?)?;
    reject_extra(&object, "record")?;
    Ok(ManagedRecord {
        version,
        status,
        declaration,
    })
}

fn parse_declaration(value: StrictJson) -> Result<ManagedDeclaration> {
    let mut object = object(value, "declaration")?;
    let name = string(take(&mut object, "name")?, "name")?;
    let filename_stem = string(take(&mut object, "filename_stem")?, "filename_stem")?;
    let key_url = string(take(&mut object, "key_url")?, "key_url")?;
    let source_url = string(take(&mut object, "source_url")?, "source_url")?;
    let architecture = string(take(&mut object, "architecture")?, "architecture")?;
    let layout = parse_layout(take(&mut object, "layout")?)?;
    let keyring_path = string(take(&mut object, "keyring_path")?, "keyring_path")?;
    let source_list_path = string(take(&mut object, "source_list_path")?, "source_list_path")?;
    reject_extra(&object, "declaration")?;
    Ok(ManagedDeclaration {
        name,
        filename_stem,
        key_url,
        source_url,
        architecture,
        layout,
        keyring_path,
        source_list_path,
    })
}

fn parse_layout(value: StrictJson) -> Result<ManagedLayout> {
    let mut object = object(value, "layout")?;
    let kind = string(take(&mut object, "type")?, "layout.type")?;
    let layout = match kind.as_str() {
        "suite_components" => {
            let suite = string(take(&mut object, "suite")?, "layout.suite")?;
            let components = match take(&mut object, "components")? {
                StrictJson::Array(values) => values
                    .into_iter()
                    .map(|value| string(value, "layout.components"))
                    .collect::<Result<Vec<_>>>()?,
                _ => bail!("layout.components must be an array"),
            };
            ManagedLayout::SuiteComponents { suite, components }
        }
        "exact_path" => ManagedLayout::ExactPath {
            path: string(take(&mut object, "path")?, "layout.path")?,
        },
        other => bail!("invalid managed layout type {other:?}"),
    };
    reject_extra(&object, "layout")?;
    Ok(layout)
}

fn object(value: StrictJson, field: &str) -> Result<BTreeMap<String, StrictJson>> {
    match value {
        StrictJson::Object(value) => Ok(value),
        _ => bail!("{field} must be an object"),
    }
}
fn take(object: &mut BTreeMap<String, StrictJson>, field: &str) -> Result<StrictJson> {
    object
        .remove(field)
        .with_context(|| format!("managed record is missing {field:?}"))
}
fn string(value: StrictJson, field: &str) -> Result<String> {
    match value {
        StrictJson::String(value) => Ok(value),
        _ => bail!("{field} must be a string"),
    }
}
fn number(value: StrictJson, field: &str) -> Result<u64> {
    match value {
        StrictJson::Number(value) => value
            .as_u64()
            .with_context(|| format!("{field} must be an unsigned integer")),
        _ => bail!("{field} must be a number"),
    }
}
fn reject_extra(object: &BTreeMap<String, StrictJson>, field: &str) -> Result<()> {
    if let Some(key) = object.keys().next() {
        bail!("unknown {field} key {key:?}");
    }
    Ok(())
}

pub fn source(host: &Host<'_>, destination: &str, contents: &str) -> Result<()> {
    let destination = validate_destination(destination, SOURCES_DIRECTORY, ".list")?;
    validate_source_contents(contents)?;
    publish_bytes(
        host,
        &destination,
        contents.as_bytes(),
        "APT source publication",
    )
}

pub fn key(host: &Host<'_>, url: &str, destination: &str) -> Result<()> {
    validate_https_url(url)?;
    let destination = validate_destination(destination, KEYRINGS_DIRECTORY, ".gpg")?;
    let bytes = normalized_key(host, url)?;
    publish_bytes(host, &destination, &bytes, "repository key publication")
}

pub(crate) fn publish_bytes(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
) -> Result<()> {
    publish_bytes_with_mode(host, destination, contents, operation, "0644")
}

pub(crate) fn publish_bytes_with_mode(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
    mode: &str,
) -> Result<()> {
    publish_bytes_with_mode_and_policy(host, destination, contents, operation, mode, false)
}

fn publish_bytes_with_policy(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
    no_replace: bool,
) -> Result<()> {
    publish_bytes_with_mode_and_policy(host, destination, contents, operation, "0644", no_replace)
}

fn publish_bytes_with_mode_and_policy(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
    mode: &str,
    no_replace: bool,
) -> Result<()> {
    if !matches!(mode, "0600" | "0644") {
        bail!("unsupported privileged publication mode");
    }
    let local = TempPath::new(host, "privileged-publication")?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(local.path())
        .context("open local publication staging file")?;
    file.write_all(contents)
        .context("write local publication staging file")?;
    file.sync_all()
        .context("sync local publication staging file")?;
    drop(file);
    let parent = destination
        .parent()
        .context("publication destination has no parent")?;
    let file_name = destination
        .file_name()
        .context("publication destination has no filename")?
        .to_string_lossy();
    let nonce = local
        .path()
        .file_name()
        .context("publication staging file has no filename")?
        .to_string_lossy();
    let staged = parent.join(format!(".{file_name}.{nonce}.tmp"));
    let parent_arg = parent.as_os_str();
    let local_arg = local.path().as_os_str();
    let staged_arg = staged.as_os_str();
    let destination_arg = destination.as_os_str();
    host.require(
        operation,
        "sudo",
        [
            OsStr::new("install"),
            OsStr::new("-d"),
            OsStr::new("-o"),
            OsStr::new("root"),
            OsStr::new("-g"),
            OsStr::new("root"),
            OsStr::new("-m"),
            OsStr::new("0755"),
            OsStr::new("--"),
            parent_arg,
        ],
    )?;
    let result = (|| {
        host.require(
            operation,
            "sudo",
            [
                OsStr::new("install"),
                OsStr::new("-o"),
                OsStr::new("root"),
                OsStr::new("-g"),
                OsStr::new("root"),
                OsStr::new("-m"),
                OsStr::new(mode),
                OsStr::new("--"),
                local_arg,
                staged_arg,
            ],
        )?;
        host.require(
            operation,
            "sudo",
            [OsStr::new("sync"), OsStr::new("--"), staged_arg],
        )?;
        if no_replace {
            // `link(2)` is an atomic no-replace publication here: both names are in the
            // destination directory, and an existing destination makes `ln` fail rather
            // than report a skipped move as success. The staging name is removed only
            // after the destination link exists.
            host.require(
                operation,
                "sudo",
                [
                    OsStr::new("ln"),
                    OsStr::new("--"),
                    staged_arg,
                    destination_arg,
                ],
            )?;
            host.require(
                operation,
                "sudo",
                [
                    OsStr::new("rm"),
                    OsStr::new("-f"),
                    OsStr::new("--"),
                    staged_arg,
                ],
            )?;
        } else {
            host.require(
                operation,
                "sudo",
                [
                    OsStr::new("test"),
                    OsStr::new("!"),
                    OsStr::new("-d"),
                    destination_arg,
                ],
            )?;
            host.require(
                operation,
                "sudo",
                [
                    OsStr::new("mv"),
                    OsStr::new("-fT"),
                    OsStr::new("--"),
                    staged_arg,
                    destination_arg,
                ],
            )?;
        }
        sync_parent(host, destination, operation)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = host.run(
            "sudo",
            [
                OsStr::new("rm"),
                OsStr::new("-f"),
                OsStr::new("--"),
                staged_arg,
            ],
        );
    }
    result
}

pub(crate) fn sync_parent(host: &Host<'_>, destination: &Path, operation: &str) -> Result<()> {
    let parent = destination
        .parent()
        .context("publication destination has no parent")?;
    host.require(
        operation,
        "sudo",
        [OsStr::new("sync"), OsStr::new("--"), parent.as_os_str()],
    )?;
    Ok(())
}

fn validate_destination(destination: &str, directory: &str, suffix: &str) -> Result<PathBuf> {
    let path = Path::new(destination);
    if destination.as_bytes().contains(&0)
        || !path.is_absolute()
        || path.parent() != Some(Path::new(directory))
        || path.file_name().is_none_or(|name| {
            name.as_bytes().contains(&0)
                || !name.as_bytes().ends_with(suffix.as_bytes())
                || name.as_bytes().len() == suffix.len()
        })
    {
        bail!("destination must be a direct {suffix} file under {directory}");
    }
    Ok(path.to_owned())
}

fn validate_source_contents(contents: &str) -> Result<()> {
    if contents.as_bytes().contains(&0)
        || !contents.ends_with('\n')
        || contents.lines().count() != 1
        || contents
            .lines()
            .next()
            .is_none_or(|line| line.trim().is_empty())
    {
        bail!("APT source contents must be exactly one non-empty generated line");
    }
    Ok(())
}

fn validate_https_url(value: &str) -> Result<()> {
    let validated = HttpsUrl::parse(value).context("repository key URL is invalid")?;
    if validated.as_str() != value {
        bail!("repository key URL must be canonical HTTPS");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> HttpsUrl {
        HttpsUrl::parse(value).unwrap()
    }
    fn operation(
        architecture: Architecture,
        layout: AptRepositorySourceLayout,
    ) -> AptRepositoryOperation {
        AptRepositoryOperation::new(
            "Vendor_Name",
            "vendor-name",
            url("https://example.test/key"),
            url("https://example.test/apt/"),
            architecture,
            layout,
        )
        .unwrap()
    }
    fn token(value: &str) -> AptRepositoryToken {
        AptRepositoryToken::parse(value).unwrap()
    }

    #[test]
    fn renders_all_architectures_and_both_layouts_exactly() {
        for (architecture, debian) in [
            (Architecture::Amd64, "amd64"),
            (Architecture::Arm64, "arm64"),
            (Architecture::Arm32, "armhf"),
            (Architecture::Riscv64, "riscv64"),
        ] {
            let operation = operation(
                architecture,
                AptRepositorySourceLayout::SuiteComponents {
                    suite: token("stable"),
                    components: vec![token("main"), token("contrib")],
                },
            );
            assert_eq!(operation.render_source(), format!("deb [arch={debian} signed-by=/etc/apt/keyrings/cozydot-vendor-name.gpg] https://example.test/apt/ stable main contrib\n"));
        }
        let stars = operation(
            Architecture::Amd64,
            AptRepositorySourceLayout::SuiteComponents {
                suite: token("*"),
                components: vec![token("*")],
            },
        );
        assert!(stars.render_source().ends_with(" * *\n"));
        let exact = operation(
            Architecture::Amd64,
            AptRepositorySourceLayout::ExactPath(AptRepositoryPath::parse("pool/vendor/").unwrap()),
        );
        assert_eq!(exact.render_source(), "deb [arch=amd64 signed-by=/etc/apt/keyrings/cozydot-vendor-name.gpg] https://example.test/apt/ pool/vendor/\n");
    }

    #[test]
    fn rejects_partial_wildcards_unsafe_paths_and_invalid_layouts() {
        for value in ["sta*ble", "*main", "Main", "", "two words"] {
            assert!(AptRepositoryToken::parse(value).is_err(), "{value}");
        }
        for value in [
            "/absolute/",
            "../escape/",
            "a//b/",
            "a\\b/",
            "a",
            ".",
            "a/./",
        ] {
            assert!(AptRepositoryPath::parse(value).is_err(), "{value}");
        }
        assert!(AptRepositoryOperation::new(
            "../bad",
            "bad",
            url("https://example.test/key"),
            url("https://example.test/apt"),
            Architecture::Amd64,
            AptRepositorySourceLayout::ExactPath(AptRepositoryPath::parse("./").unwrap())
        )
        .is_err());
        assert!(AptRepositoryOperation::new(
            "Good_Name",
            "wrong",
            url("https://example.test/key"),
            url("https://example.test/apt"),
            Architecture::Amd64,
            AptRepositorySourceLayout::SuiteComponents {
                suite: token("stable"),
                components: vec![]
            }
        )
        .is_err());
        assert!(AptRepositoryOperation::new(
            "Good_Name",
            "good-name",
            url("https://example.test/key"),
            url("https://example.test/apt"),
            Architecture::Amd64,
            AptRepositorySourceLayout::SuiteComponents {
                suite: token("system"),
                components: vec![token("main")]
            }
        )
        .is_err());
    }

    #[test]
    fn strict_json_rejects_duplicate_keys_at_every_level() {
        for value in [
            r#"{"version":1,"version":1,"status":"pending_initial","declaration":{}}"#,
            r#"{"version":1,"status":"pending_initial","declaration":{"name":"a","name":"b"}}"#,
            r#"{"layout":{"type":"suite_components","suite":"stable","suite":"testing"}}"#,
            r#"{"layout":{"components":[{"name":"a","name":"b"}]}}"#,
        ] {
            assert!(serde_json::from_str::<StrictJson>(value).is_err());
        }
    }
}
