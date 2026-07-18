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
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

const SOURCES_DIRECTORY: &str = "/etc/apt/sources.list.d";
const KEYRINGS_DIRECTORY: &str = "/etc/apt/keyrings";
const MANAGED_STATE_VERSION: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AptRepositoryToken(String);

impl AptRepositoryToken {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value != "*"
            && (value.is_empty()
                || !value.as_bytes()[0].is_ascii_lowercase() && !value.as_bytes()[0].is_ascii_digit()
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._+-".contains(&byte)))
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
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let valid = value == "./"
            || value.ends_with('/')
                && !value.starts_with('/')
                && !value.contains('\\')
                && !value.contains("//")
                && value[..value.len() - 1].split('/').all(valid_definition_name);
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
            bail!("APT repository filename stem {filename_stem:?} does not match name-derived stem {expected_stem:?}");
        }
        validate_canonical_url(&key_url, "key")?;
        validate_canonical_url(&source_url, "source")?;
        validate_layout(&layout)?;

        let keyring_path = PathBuf::from(format!("{KEYRINGS_DIRECTORY}/cozydot-{filename_stem}.gpg"));
        let source_list_path = PathBuf::from(format!("{SOURCES_DIRECTORY}/cozydot-{filename_stem}.list"));
        validate_destination(
            keyring_path.to_str().context("repository keyring path is not UTF-8")?,
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

    match record.as_ref().map(|record| (&record.status, &record.declaration)) {
        None => {
            require_absent(host, &operation.keyring_path, "repository keyring preflight")?;
            require_absent(host, &operation.source_list_path, "repository source preflight")?;
        }
        Some((ManagedStatus::PendingInitial | ManagedStatus::PendingUpdate, recorded)) if recorded != &declaration => {
            bail!("APT repository has a pending managed record for a different declaration")
        }
        Some((_, recorded)) => validate_record_destinations(recorded, &operation.filename_stem)?,
    }

    let completed_matching = record
        .as_ref()
        .is_some_and(|record| record.status == ManagedStatus::Completed && record.declaration == declaration);
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
        && inspect_owned_file(host, &operation.keyring_path, "APT repository key")?.as_deref() == Some(key.as_slice())
        && inspect_owned_file(host, &operation.source_list_path, "APT repository source")?.as_deref()
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
                bail!("APT repository suite 'system' must be resolved before operation construction");
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
                    bail!("duplicate APT repository component {:?}", component.as_str());
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
    value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
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
    let parsed = HttpsUrl::parse(url.as_str()).with_context(|| format!("APT repository {kind} URL is invalid"))?;
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
    if !inspection
        .stdout
        .split(|byte| *byte == b'\n')
        .any(|line| line.strip_prefix(b"pub:").is_some_and(|fields| !fields.is_empty()))
    {
        bail!("repository key validation found no public key");
    }
    Ok(bytes)
}

fn converge_owned_bytes(host: &Host<'_>, path: &Path, expected: &[u8], label: &str, no_replace: bool) -> Result<()> {
    let current = inspect_owned_file(host, path, label)?;
    if no_replace && current.as_deref().is_some_and(|bytes| bytes != expected) {
        bail!("{label} initial-publication destination collision");
    }
    if current.is_none() || !no_replace && current.as_deref() != Some(expected) {
        super::privileged_file::publish_bytes_with_policy(
            host,
            path,
            expected,
            &format!("{label} publication"),
            no_replace,
        )?;
    }
    let final_bytes =
        inspect_owned_file(host, path, label)?.with_context(|| format!("{label} postcondition is missing"))?;
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
            [OsStr::new("test"), OsStr::new("!"), OsStr::new("-e"), path_arg],
        )?
        .status
        .success();
    let not_symlink = host
        .run(
            "sudo",
            [OsStr::new("test"), OsStr::new("!"), OsStr::new("-L"), path_arg],
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
    let mode = fields.next().and_then(|value| u32::from_str_radix(value, 16).ok());
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
            [OsStr::new("test"), OsStr::new("!"), OsStr::new(kind), path_arg],
        )?;
        if !output.status.success() {
            bail!("{label}: unmanaged destination conflict at {}", path.display());
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
    SuiteComponents { suite: String, components: Vec<String> },
    ExactPath { path: String },
}

impl ManagedDeclaration {
    fn from_operation(operation: &AptRepositoryOperation) -> Self {
        let layout = match &operation.layout {
            AptRepositorySourceLayout::SuiteComponents { suite, components } => ManagedLayout::SuiteComponents {
                suite: suite.as_str().into(),
                components: components.iter().map(|component| component.as_str().into()).collect(),
            },
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
                let value: StrictJson =
                    serde_json::from_slice(&bytes).context("parse strict APT repository managed record")?;
                let record = parse_managed_record(value).context("validate APT repository managed record")?;
                validate_managed_declaration(&record.declaration)
                    .context("validate APT repository managed declaration")?;
                Ok(record)
            })
            .transpose()
    }

    fn publish_record(&self, record: &ManagedRecord) -> Result<()> {
        self.0
            .publish(&serde_json::to_vec(record).context("serialize APT repository managed record")?)
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
        ManagedLayout::SuiteComponents { suite, components } => AptRepositorySourceLayout::SuiteComponents {
            suite: AptRepositoryToken::parse(suite)?,
            components: components
                .iter()
                .map(AptRepositoryToken::parse)
                .collect::<Result<Vec<_>>>()?,
        },
        ManagedLayout::ExactPath { path } => AptRepositorySourceLayout::ExactPath(AptRepositoryPath::parse(path)?),
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
    if record.filename_stem != stem || record.keyring_path != key || record.source_list_path != source {
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
                return Err(serde::de::Error::custom(format!("duplicate JSON key {key:?}")));
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

pub(crate) mod managed_apt {

    use super::super::{privileged_file, Host};
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
        pub fn from_policy(policy: ManagedAptSources) -> Result<Self> {
            validate_policy(&policy)?;
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

        let remaining = reconcile(&operation.policy, &inspect_sources(host)?)?;
        if !remaining.is_empty() {
            bail!("managed APT publication did not establish the exact source postcondition");
        }
        Ok(())
    }

    fn validate_policy(policy: &ManagedAptSources) -> Result<()> {
        let upstream = if policy.distro == "ubuntu" { "ubuntu" } else { "debian" };
        let platform = Platform::from_parts(
            policy.distro.clone(),
            upstream.into(),
            policy.release.clone(),
            "none".into(),
            policy.architecture.canonical(),
        )?;
        let component_refs = policy.components.iter().map(String::as_str).collect::<Vec<_>>();
        if platform.managed_apt_sources(&component_refs)? != *policy {
            bail!("managed APT operation policy is not canonical");
        }
        Ok(())
    }

    fn preflight_keyring(host: &Host<'_>, policy: &ManagedAptSources) -> Result<()> {
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
            let path = std::str::from_utf8(raw).context("managed APT source discovery returned a non-UTF-8 path")?;
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
            || values
                .iter()
                .any(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
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
            let architectures = fields
                .get("architectures")
                .map(|value| value.split_ascii_whitespace().map(str::to_owned).collect::<Vec<_>>());
            let applies = architectures
                .as_ref()
                .is_none_or(|values| values.iter().any(|value| value == policy.architecture.debian()));
            let official = uris.iter().filter(|uri| official_uri(policy, uri)).count();
            if applies && official != 0 {
                if fields.contains_key("architectures-add") || fields.contains_key("architectures-remove") {
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
        privileged_file::publish_bytes_with_mode(
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
}
