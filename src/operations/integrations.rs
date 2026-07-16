use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

use super::{
    privileged_file::{publish_bytes, sync_parent},
    Host,
};

const DOCKER_DAEMON_CONFIG: &str = "/etc/docker/daemon.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerLocalLogOperation {
    max_size: Option<String>,
}

impl DockerLocalLogOperation {
    pub fn new(max_size: Option<String>) -> Result<Self> {
        validate_max_size(max_size.as_deref())?;
        Ok(Self { max_size })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        std::iter::once("docker-local-log".into())
            .chain(self.max_size.iter().cloned())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VsCodeExtensionOperation {
    extensions: Vec<String>,
}

impl VsCodeExtensionOperation {
    pub fn new(extensions: Vec<String>) -> Result<Self> {
        let extensions = canonical_extensions(&extensions)?;
        Ok(Self { extensions })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        std::iter::once("vscode-extension-set".into())
            .chain(self.extensions.iter().cloned())
            .collect()
    }
}

pub(crate) fn docker_group(host: &Host<'_>) -> Result<()> {
    ensure_product_group(host, Product::Docker)
}

pub(crate) fn virtualbox_group(host: &Host<'_>) -> Result<()> {
    ensure_product_group(host, Product::VirtualBox)
}

pub(crate) fn docker_local_log(host: &Host<'_>, operation: &DockerLocalLogOperation) -> Result<()> {
    validate_max_size(operation.max_size.as_deref())
        .context("validate Docker logging operation")?;
    preflight(host, Product::Docker)?;
    let _lock = host.acquire_docker_lock()?;

    let current = read_daemon_config(host)?;
    let mut requested = current.clone();
    let object = requested
        .as_object_mut()
        .context("Docker daemon config must be a JSON object")?;
    object.insert("log-driver".into(), Value::String("local".into()));
    if let Some(max_size) = &operation.max_size {
        let log_options = object
            .entry("log-opts")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .context("Docker daemon config log-opts must be a JSON object")?;
        log_options.insert("max-size".into(), Value::String(max_size.clone()));
    }
    if requested == current {
        sync_parent(
            host,
            Path::new(DOCKER_DAEMON_CONFIG),
            "Docker daemon config publication",
        )?;
        return Ok(());
    }

    let mut bytes =
        serde_json::to_vec_pretty(&requested).context("serialize Docker daemon configuration")?;
    bytes.push(b'\n');
    publish_bytes(
        host,
        Path::new(DOCKER_DAEMON_CONFIG),
        &bytes,
        "Docker daemon config publication",
    )?;
    let published = read_daemon_config(host)?;
    if published != requested {
        bail!("Docker daemon config publication did not establish the requested state");
    }
    Ok(())
}

pub(crate) fn vscode_extensions(
    host: &Host<'_>,
    operation: &VsCodeExtensionOperation,
) -> Result<()> {
    let extensions = canonical_extensions(&operation.extensions)
        .context("validate VS Code extension operation")?;
    preflight(host, Product::VsCode)?;
    let installed = inspect_extensions(host)?;
    let missing = extensions
        .iter()
        .filter(|extension| !installed.contains(extension.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    for extension in missing {
        host.require(
            "VS Code extension installation",
            "code",
            ["--install-extension", extension.as_str()],
        )?;
    }
    let installed = inspect_extensions(host)?;
    let missing = extensions
        .iter()
        .filter(|extension| !installed.contains(extension.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "VS Code extension installation did not install configured extensions: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Product {
    Docker,
    VirtualBox,
    VsCode,
}

impl Product {
    fn label(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::VirtualBox => "VirtualBox",
            Self::VsCode => "VS Code",
        }
    }

    fn program(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::VirtualBox => "VBoxManage",
            Self::VsCode => "code",
        }
    }

    fn group(self) -> Option<&'static str> {
        match self {
            Self::Docker => Some("docker"),
            Self::VirtualBox => Some("vboxusers"),
            Self::VsCode => None,
        }
    }
}

fn preflight(host: &Host<'_>, product: Product) -> Result<()> {
    let output = host
        .require(
            &format!("{} existing-product preflight", product.label()),
            product.program(),
            ["--version"],
        )
        .with_context(|| {
            format!(
                "{} integration requires an existing usable {} CLI",
                product.label(),
                product.program()
            )
        })?;
    let version = std::str::from_utf8(&output.stdout).with_context(|| {
        format!(
            "{} version probe returned non-UTF-8 output",
            product.label()
        )
    })?;
    let valid = match product {
        Product::Docker => valid_docker_version(version),
        Product::VirtualBox => valid_virtualbox_version(version),
        Product::VsCode => valid_vscode_version(version),
    };
    if !valid {
        bail!(
            "{} version probe returned malformed output",
            product.label()
        );
    }
    Ok(())
}

fn valid_docker_version(value: &str) -> bool {
    let value = value.strip_suffix('\n').unwrap_or(value);
    let Some(value) = value.strip_prefix("Docker version ") else {
        return false;
    };
    let Some((version, build)) = value.split_once(", build ") else {
        return false;
    };
    valid_version_token(version) && valid_token(build)
}

fn valid_virtualbox_version(value: &str) -> bool {
    let value = value.strip_suffix('\n').unwrap_or(value);
    !value.contains(['\n', '\r'])
        && value.split_once('r').is_some_and(|(version, revision)| {
            valid_version_token(version)
                && !revision.is_empty()
                && revision.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn valid_vscode_version(value: &str) -> bool {
    let mut lines = value.lines();
    let valid = lines.next().is_some_and(valid_version_token)
        && lines.next().is_some_and(|commit| {
            commit.len() >= 7 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        && lines
            .next()
            .is_some_and(|arch| matches!(arch, "x64" | "arm64" | "armhf" | "ia32"));
    valid && lines.next().is_none()
}

fn valid_version_token(value: &str) -> bool {
    let suffix = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .map_or("", |index| &value[index..]);
    let core = &value[..value.len() - suffix.len()];
    let components = core.split('.').collect::<Vec<_>>();
    components.len() >= 2
        && components.iter().all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
        && (suffix.is_empty()
            || matches!(suffix.as_bytes()[0], b'-' | b'+' | b'_') && valid_token(&suffix[1..]))
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+' | b'_'))
}

fn ensure_product_group(host: &Host<'_>, product: Product) -> Result<()> {
    let (username, _) = effective_user(host)?;
    preflight(host, product)?;
    let group = product.group().expect("group product");
    let gid = if let Some(gid) = group_gid(host, group)? {
        gid
    } else {
        host.require(
            &format!("{} group creation", product.label()),
            "sudo",
            ["groupadd", "--system", group],
        )?;
        group_gid(host, group)?.ok_or_else(|| {
            anyhow::anyhow!("{} group creation did not create {group}", product.label())
        })?
    };
    if user_group_ids(host, &username)?.contains(&gid) {
        return Ok(());
    }
    host.require(
        &format!("{} group membership", product.label()),
        "sudo",
        ["usermod", "-aG", group, "--", username.as_str()],
    )?;
    if !user_group_ids(host, &username)?.contains(&gid) {
        bail!(
            "{} group membership mutation did not add {username} to {group}",
            product.label()
        );
    }
    Ok(())
}

fn effective_user(host: &Host<'_>) -> Result<(String, u32)> {
    let uid = rustix::process::geteuid().as_raw();
    let uid_arg = uid.to_string();
    let output = host.require(
        "effective system user query",
        "getent",
        ["passwd", &uid_arg],
    )?;
    let record = one_utf8_record(&output.stdout, "getent passwd")?;
    if record.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("getent passwd returned control characters");
    }
    let fields = record.split(':').collect::<Vec<_>>();
    if fields.len() != 7 || !valid_nss_login(fields[0]) || parse_decimal_id(fields[2]) != Some(uid)
    {
        bail!("getent passwd returned a malformed or mismatched effective-user record");
    }
    Ok((fields[0].to_owned(), uid))
}

fn valid_nss_login(value: &str) -> bool {
    !value.is_empty()
        && !value
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
}

fn parse_decimal_id(value: &str) -> Option<u32> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn group_gid(host: &Host<'_>, group: &str) -> Result<Option<u32>> {
    let output = host.run("getent", ["group", group])?;
    if !output.status.success() {
        if output.status.code() == Some(2) && output.stdout.is_empty() {
            return Ok(None);
        }
        bail!(
            "system group query failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let record = one_utf8_record(&output.stdout, "getent group")?;
    if record.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("getent group returned control characters");
    }
    let fields = record.split(':').collect::<Vec<_>>();
    if fields.len() != 4 || fields[0] != group || parse_decimal_id(fields[2]).is_none() {
        bail!("getent group returned a malformed or mismatched group record");
    }
    Ok(parse_decimal_id(fields[2]))
}

fn user_group_ids(host: &Host<'_>, username: &str) -> Result<BTreeSet<u32>> {
    let output = host.require("system user group query", "id", ["-G", "--", username])?;
    let record = one_utf8_record(&output.stdout, "id -G")?;
    let mut groups = BTreeSet::new();
    for group in record.split_ascii_whitespace() {
        let Some(gid) = parse_decimal_id(group) else {
            bail!("id -G returned malformed group IDs");
        };
        if !groups.insert(gid) {
            bail!("id -G returned duplicate group IDs");
        }
    }
    Ok(groups)
}

fn one_utf8_record<'a>(bytes: &'a [u8], command: &str) -> Result<&'a str> {
    let output = std::str::from_utf8(bytes)
        .with_context(|| format!("{command} returned non-UTF-8 output"))?;
    let record = output.strip_suffix('\n').unwrap_or(output);
    if record.is_empty() || record.contains(['\n', '\r']) {
        bail!("{command} returned malformed record output");
    }
    Ok(record)
}

fn read_daemon_config(host: &Host<'_>) -> Result<Value> {
    let kind = host.run("sudo", ["stat", "--format=%f", "--", DOCKER_DAEMON_CONFIG])?;
    if !kind.status.success() {
        host.require(
            "Docker daemon config absence check",
            "sudo",
            ["test", "!", "-e", DOCKER_DAEMON_CONFIG],
        )?;
        host.require(
            "Docker daemon config symlink absence check",
            "sudo",
            ["test", "!", "-L", DOCKER_DAEMON_CONFIG],
        )?;
        return Ok(Value::Object(Map::new()));
    }
    let mode = one_utf8_record(&kind.stdout, "sudo stat")?;
    let mode = u32::from_str_radix(mode, 16).context("sudo stat returned malformed mode output")?;
    if mode & 0o170000 != 0o100000 {
        bail!("Docker daemon config destination is not a regular file");
    }
    let output = host.require(
        "Docker daemon config inspection",
        "sudo",
        ["cat", "--", DOCKER_DAEMON_CONFIG],
    )?;
    let text =
        std::str::from_utf8(&output.stdout).context("Docker daemon config is not valid UTF-8")?;
    let value: Value =
        serde_json::from_str(text).context("Docker daemon config is invalid JSON")?;
    if !value.is_object() {
        bail!("Docker daemon config must be a JSON object");
    }
    Ok(value)
}

fn inspect_extensions(host: &Host<'_>) -> Result<BTreeSet<String>> {
    let output = host.require(
        "VS Code installed extension query",
        "code",
        ["--list-extensions"],
    )?;
    let output = std::str::from_utf8(&output.stdout)
        .context("code returned non-UTF-8 installed extension state")?;
    let mut installed = BTreeSet::new();
    for extension in output.lines() {
        let Some(extension) = canonical_extension(extension) else {
            bail!("code returned malformed extension identifier: {extension:?}");
        };
        if !installed.insert(extension) {
            bail!("code returned duplicate case-folded extension identifiers");
        }
    }
    Ok(installed)
}

fn canonical_extensions(extensions: &[String]) -> Result<Vec<String>> {
    if extensions.is_empty() {
        bail!("VS Code extension operation requires at least one extension");
    }
    let mut unique = BTreeSet::new();
    let mut canonical = Vec::with_capacity(extensions.len());
    for extension in extensions {
        let Some(normalized) = canonical_extension(extension) else {
            bail!("invalid VS Code publisher.extension identifier: {extension:?}");
        };
        if !unique.insert(normalized.clone()) {
            bail!("duplicate VS Code extension identifier: {extension:?}");
        }
        canonical.push(normalized);
    }
    Ok(canonical)
}

fn canonical_extension(value: &str) -> Option<String> {
    let mut parts = value.split('.');
    (valid_identifier(parts.next().unwrap_or_default())
        && valid_identifier(parts.next().unwrap_or_default())
        && parts.next().is_none())
    .then(|| value.to_ascii_lowercase())
}

fn valid_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn validate_max_size(value: Option<&str>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let number = value
        .strip_suffix('k')
        .or_else(|| value.strip_suffix('m'))
        .or_else(|| value.strip_suffix('g'));
    if number.is_none_or(|number| {
        number.is_empty()
            || !number.bytes().all(|byte| byte.is_ascii_digit())
            || number.bytes().all(|byte| byte == b'0')
    }) {
        bail!("Docker max size must be a positive integer followed by k, m, or g");
    }
    Ok(())
}
