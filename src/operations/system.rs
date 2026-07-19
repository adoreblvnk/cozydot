use anyhow::{bail, Context, Result};
use std::{collections::BTreeSet, path::Path};

use super::{
    apt,
    privileged_file::{publish_bytes, sync_parent},
    Host, OperationOutcome, TempPath,
};
use serde_json::{Map, Value};

const AUTO_UPGRADES: &str = "/etc/apt/apt.conf.d/20auto-upgrades";
const NO_SNAP_PIN: &str = "/etc/apt/preferences.d/cozydot-no-snap.pref";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnsureAdminOperation;

impl EnsureAdminOperation {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn display_args(self) -> Vec<String> {
        vec!["ensure-admin".into()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnattendedUpgradesOperation {
    enabled: bool,
}

impl UnattendedUpgradesOperation {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub(crate) fn display_args(self) -> Vec<String> {
        vec!["unattended-upgrades".into(), self.enabled.to_string()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UbuntuSnapOperation {
    enabled: bool,
}

impl UbuntuSnapOperation {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub(crate) fn display_args(self) -> Vec<String> {
        vec!["ubuntu-snap".into(), self.enabled.to_string()]
    }
}

pub(crate) fn ensure_admin(host: &Host<'_>, _: &EnsureAdminOperation) -> Result<()> {
    let (username, _) = effective_user(host)?;
    let sudo_gid = group_gid(host, "sudo")?.context("administrative group sudo does not exist")?;
    if user_group_ids(host, &username)?.contains(&sudo_gid) {
        return Ok(());
    }
    host.require(
        "administrative group membership",
        "sudo",
        ["usermod", "-aG", "sudo", "--", &username],
    )?;
    if !user_group_ids(host, &username)?.contains(&sudo_gid) {
        bail!("administrative group mutation did not add {username} to sudo");
    }
    Ok(())
}

pub(crate) fn unattended_upgrades(host: &Host<'_>, operation: &UnattendedUpgradesOperation) -> Result<()> {
    let contents = if operation.enabled {
        b"APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n".as_slice()
    } else {
        b"APT::Periodic::Update-Package-Lists \"0\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n".as_slice()
    };
    if operation.enabled {
        apt::packages(host, &["unattended-upgrades".into()])?;
        publish_bytes(
            host,
            Path::new(AUTO_UPGRADES),
            contents,
            "unattended-upgrades periodic configuration",
        )?;
        if !systemd_state(host, "is-enabled", "unattended-upgrades.service")?
            || !systemd_state(host, "is-active", "unattended-upgrades.service")?
        {
            host.require(
                "unattended-upgrades service enablement",
                "sudo",
                ["systemctl", "enable", "--now", "unattended-upgrades.service"],
            )?;
        }
    } else {
        publish_bytes(
            host,
            Path::new(AUTO_UPGRADES),
            contents,
            "unattended-upgrades periodic configuration",
        )?;
        if systemd_state(host, "is-enabled", "unattended-upgrades.service")?
            || systemd_state(host, "is-active", "unattended-upgrades.service")?
        {
            host.require(
                "unattended-upgrades service disablement",
                "sudo",
                ["systemctl", "disable", "--now", "unattended-upgrades.service"],
            )?;
        }
        if package_installed(host, "unattended-upgrades")? {
            apt::purge(host, &["unattended-upgrades".into()])?;
        }
    }
    if package_installed(host, "unattended-upgrades")? != operation.enabled {
        bail!("unattended-upgrades package state did not converge");
    }
    require_root_file(host, AUTO_UPGRADES, contents, "unattended-upgrades")?;
    if operation.enabled {
        require_systemd_state(host, "unattended-upgrades.service", true)?;
    } else if systemd_state(host, "is-enabled", "unattended-upgrades.service")?
        || systemd_state(host, "is-active", "unattended-upgrades.service")?
    {
        bail!("unattended-upgrades service remains enabled or active");
    }
    Ok(())
}

pub(crate) fn ubuntu_snap(host: &Host<'_>, operation: &UbuntuSnapOperation) -> Result<()> {
    if operation.enabled {
        host.require("no-Snap APT pin removal", "sudo", ["rm", "-f", "--", NO_SNAP_PIN])?;
        apt::packages(host, &["snapd".into()])?;
        if !systemd_state(host, "is-enabled", "snapd.socket")? || !systemd_state(host, "is-active", "snapd.socket")? {
            host.require(
                "Snap service enablement",
                "sudo",
                ["systemctl", "enable", "--now", "snapd.socket"],
            )?;
        }
        if !package_installed(host, "snapd")? {
            bail!("Snap enablement did not install snapd");
        }
        require_systemd_state(host, "snapd.socket", true)?;
        return Ok(());
    }

    remove_snaps(host)?;
    for unit in ["snapd.socket", "snapd.service", "snapd.seeded.service"] {
        if systemd_state(host, "is-enabled", unit)? || systemd_state(host, "is-active", unit)? {
            host.require(
                "Snap service disablement",
                "sudo",
                ["systemctl", "disable", "--now", unit],
            )?;
        }
    }
    apt::purge(host, &["snapd".into()])?;
    let home_snap = host.home().join("snap");
    host.require(
        "Snap data removal",
        "sudo",
        [
            "rm".as_ref(),
            "-rf".as_ref(),
            "--".as_ref(),
            home_snap.as_os_str(),
            "/snap".as_ref(),
            "/var/snap".as_ref(),
            "/var/lib/snapd".as_ref(),
        ],
    )?;
    let pin = b"Package: snapd\nPin: release a=*\nPin-Priority: -10\n";
    publish_bytes(host, Path::new(NO_SNAP_PIN), pin, "no-Snap APT pin publication")?;
    if package_installed(host, "snapd")? {
        bail!("Snap disablement did not remove snapd");
    }
    require_root_file(host, NO_SNAP_PIN, pin, "no-Snap APT pin")?;
    for unit in ["snapd.socket", "snapd.service", "snapd.seeded.service"] {
        if systemd_state(host, "is-enabled", unit)? || systemd_state(host, "is-active", unit)? {
            bail!("Snap unit {unit} remains enabled or active");
        }
    }
    for path in [
        home_snap.as_path(),
        Path::new("/snap"),
        Path::new("/var/snap"),
        Path::new("/var/lib/snapd"),
    ] {
        let output = host.run("sudo", ["test".as_ref(), "!".as_ref(), "-e".as_ref(), path.as_os_str()])?;
        if !output.status.success() {
            bail!("Snap data path remains present: {}", path.display());
        }
    }
    Ok(())
}

fn remove_snaps(host: &Host<'_>) -> Result<()> {
    let output = host.run("snap", ["list"])?;
    if !output.status.success() {
        return Ok(());
    }
    let output = std::str::from_utf8(&output.stdout).context("snap list returned non-UTF-8 state")?;
    let mut names = Vec::new();
    for (index, line) in output.lines().enumerate() {
        if index == 0 {
            continue;
        }
        let name = line.split_ascii_whitespace().next().unwrap_or_default();
        if !valid_snap_name(name) {
            bail!("snap list returned malformed package state");
        }
        names.push(name.to_owned());
    }
    names.sort_by_key(|name| matches!(name.as_str(), "snapd" | "bare") || name.starts_with("core"));
    for name in names {
        host.require("Snap package removal", "sudo", ["snap", "remove", "--purge", &name])?;
    }
    Ok(())
}

fn valid_snap_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn package_installed(host: &Host<'_>, package: &str) -> Result<bool> {
    let output = host.run(
        "dpkg-query",
        ["-W", "-f=${Package}\\t${db:Status-Abbrev}\\n", "--", package],
    )?;
    if !output.status.success() && output.status.code() != Some(1) {
        bail!("dpkg-query failed while verifying {package}");
    }
    if output.stdout.is_empty() {
        return Ok(false);
    }
    let record = one_record(&output.stdout, "dpkg-query")?;
    let Some((returned, status)) = record.split_once('\t') else {
        bail!("dpkg-query returned malformed package state");
    };
    if returned != package || status.len() != 3 {
        bail!("dpkg-query returned mismatched package state");
    }
    Ok(status.as_bytes()[1] == b'i')
}

fn require_root_file(host: &Host<'_>, path: &str, expected: &[u8], operation: &str) -> Result<()> {
    let output = host.require(operation, "sudo", ["cat", "--", path])?;
    if output.stdout != expected {
        bail!("{operation} file content did not converge");
    }
    Ok(())
}

fn require_systemd_state(host: &Host<'_>, unit: &str, expected: bool) -> Result<()> {
    if systemd_state(host, "is-enabled", unit)? != expected || systemd_state(host, "is-active", unit)? != expected {
        bail!("systemd unit {unit} did not converge to enabled={expected}");
    }
    Ok(())
}

fn systemd_state(host: &Host<'_>, query: &str, unit: &str) -> Result<bool> {
    let output = host.run("systemctl", ["--quiet", query, unit])?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(_) => Ok(false),
        None => bail!("systemctl {query} {unit} terminated without an exit code"),
    }
}

fn effective_user(host: &Host<'_>) -> Result<(String, u32)> {
    let uid = rustix::process::geteuid().as_raw();
    let output = host.require("effective user query", "getent", ["passwd", &uid.to_string()])?;
    let record = one_record(&output.stdout, "getent passwd")?;
    let fields = record.split(':').collect::<Vec<_>>();
    if fields.len() != 7 || fields[0].is_empty() || fields[2].parse::<u32>().ok() != Some(uid) {
        bail!("getent passwd returned a malformed effective-user record");
    }
    Ok((fields[0].to_owned(), uid))
}

fn group_gid(host: &Host<'_>, group: &str) -> Result<Option<u32>> {
    let output = host.run("getent", ["group", group])?;
    if output.status.code() == Some(2) && output.stdout.is_empty() {
        return Ok(None);
    }
    if !output.status.success() {
        bail!("getent group failed");
    }
    let record = one_record(&output.stdout, "getent group")?;
    let fields = record.split(':').collect::<Vec<_>>();
    if fields.len() != 4 || fields[0] != group {
        bail!("getent group returned malformed state");
    }
    Ok(Some(fields[2].parse().context("getent group returned malformed GID")?))
}

fn user_group_ids(host: &Host<'_>, username: &str) -> Result<BTreeSet<u32>> {
    let output = host.require("user group query", "id", ["-G", "--", username])?;
    let record = one_record(&output.stdout, "id -G")?;
    record
        .split_ascii_whitespace()
        .map(|value| value.parse::<u32>().context("id -G returned malformed GID"))
        .collect()
}

fn one_record<'a>(bytes: &'a [u8], command: &str) -> Result<&'a str> {
    let output = std::str::from_utf8(bytes).with_context(|| format!("{command} returned non-UTF-8 output"))?;
    let record = output.strip_suffix('\n').unwrap_or(output);
    if record.is_empty() || record.contains(['\n', '\r']) {
        bail!("{command} returned malformed record output");
    }
    Ok(record)
}

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
    validate_max_size(operation.max_size.as_deref()).context("validate Docker logging operation")?;
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
    let mut bytes = serde_json::to_vec_pretty(&requested).context("serialize Docker daemon configuration")?;
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
pub(crate) fn vscode_extensions(host: &Host<'_>, operation: &VsCodeExtensionOperation) -> Result<()> {
    let extensions = canonical_extensions(&operation.extensions).context("validate VS Code extension operation")?;
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
    let version = std::str::from_utf8(&output.stdout)
        .with_context(|| format!("{} version probe returned non-UTF-8 output", product.label()))?;
    let valid = match product {
        Product::Docker => valid_docker_version(version),
        Product::VirtualBox => valid_virtualbox_version(version),
        Product::VsCode => valid_vscode_version(version),
    };
    if !valid {
        bail!("{} version probe returned malformed output", product.label());
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
            valid_version_token(version) && !revision.is_empty() && revision.bytes().all(|byte| byte.is_ascii_digit())
        })
}
fn valid_vscode_version(value: &str) -> bool {
    let mut lines = value.lines();
    let valid = lines.next().is_some_and(valid_version_token)
        && lines
            .next()
            .is_some_and(|commit| commit.len() >= 7 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()))
        && lines
            .next()
            .is_some_and(|arch| matches!(arch, "x64" | "arm64" | "armhf"));
    valid && lines.next().is_none()
}
fn valid_version_token(value: &str) -> bool {
    let suffix = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .map_or("", |index| &value[index..]);
    let core = &value[..value.len() - suffix.len()];
    let components = core.split('.').collect::<Vec<_>>();
    components.len() >= 2
        && components
            .iter()
            .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
        && (suffix.is_empty() || matches!(suffix.as_bytes()[0], b'-' | b'+' | b'_') && valid_token(&suffix[1..]))
}
fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().next().is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value.bytes().last().is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+' | b'_'))
}
fn ensure_product_group(host: &Host<'_>, product: Product) -> Result<()> {
    let (username, _) = effective_user(host)?;
    preflight(host, product)?;
    let group = product.group().context("group integration requires a system group")?;
    let gid = if let Some(gid) = group_gid(host, group)? {
        gid
    } else {
        host.require(
            &format!("{} group creation", product.label()),
            "sudo",
            ["groupadd", "--system", group],
        )?;
        group_gid(host, group)?
            .ok_or_else(|| anyhow::anyhow!("{} group creation did not create {group}", product.label()))?
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
fn one_utf8_record<'a>(bytes: &'a [u8], command: &str) -> Result<&'a str> {
    let output = std::str::from_utf8(bytes).with_context(|| format!("{command} returned non-UTF-8 output"))?;
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
    let text = std::str::from_utf8(&output.stdout).context("Docker daemon config is not valid UTF-8")?;
    let value: Value = serde_json::from_str(text).context("Docker daemon config is invalid JSON")?;
    if !value.is_object() {
        bail!("Docker daemon config must be a JSON object");
    }
    Ok(value)
}
fn inspect_extensions(host: &Host<'_>) -> Result<BTreeSet<String>> {
    let output = host.require("VS Code installed extension query", "code", ["--list-extensions"])?;
    let output = std::str::from_utf8(&output.stdout).context("code returned non-UTF-8 installed extension state")?;
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
    bytes.next().is_some_and(|byte| byte.is_ascii_alphanumeric())
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

const DASH_TO_DOCK_UUID: &str = "dash-to-dock@micxgx.gmail.com";
const ROUNDED_CORNERS_UUID: &str = "rounded-window-corners@fxgn";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Gnome,
    Cinnamon,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopTheme {
    Light,
    Dark,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopSetting {
    Theme(DesktopTheme),
    Terminal(String),
    IdleTimeoutSeconds(u32),
    IdleDim(bool),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopSettingOperation {
    target: DesktopEnvironment,
    setting: DesktopSetting,
}
impl DesktopSettingOperation {
    pub fn new(target: DesktopEnvironment, setting: DesktopSetting) -> Result<Self> {
        if let DesktopSetting::Terminal(executable) = &setting {
            validate_executable(executable)?;
        }
        Ok(Self { target, setting })
    }
    pub(crate) fn display_args(&self) -> Vec<String> {
        let target = match self.target {
            DesktopEnvironment::Gnome => "gnome",
            DesktopEnvironment::Cinnamon => "cinnamon",
        };
        let (name, value) = match &self.setting {
            DesktopSetting::Theme(DesktopTheme::Light) => ("theme", "light".into()),
            DesktopSetting::Theme(DesktopTheme::Dark) => ("theme", "dark".into()),
            DesktopSetting::Terminal(executable) => ("terminal", executable.clone()),
            DesktopSetting::IdleTimeoutSeconds(seconds) => ("idle-timeout-seconds", seconds.to_string()),
            DesktopSetting::IdleDim(enabled) => ("idle-dim", enabled.to_string()),
        };
        vec!["desktop-setting".into(), target.into(), name.into(), value]
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GnomeExtensionsOperation {
    extensions: Vec<String>,
}
impl GnomeExtensionsOperation {
    pub fn new(extensions: Vec<String>) -> Result<Self> {
        validate_extensions(&extensions)?;
        Ok(Self { extensions })
    }
    pub(crate) fn display_args(&self) -> Vec<String> {
        std::iter::once("gnome-extensions".into())
            .chain(self.extensions.iter().cloned())
            .collect()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GnomeDockOperation;
impl GnomeDockOperation {
    pub fn new() -> Self {
        Self
    }
    pub(crate) fn display_args(self) -> Vec<String> {
        vec!["gnome-dock".into()]
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GnomeRoundedCornersOperation;
impl GnomeRoundedCornersOperation {
    pub fn new() -> Self {
        Self
    }
    pub(crate) fn display_args(self) -> Vec<String> {
        vec!["gnome-rounded-corners".into()]
    }
}
pub(crate) fn desktop_setting(host: &Host<'_>, operation: &DesktopSettingOperation) -> Result<()> {
    let prefix = match operation.target {
        DesktopEnvironment::Gnome => "org.gnome",
        DesktopEnvironment::Cinnamon => "org.cinnamon",
    };
    match &operation.setting {
        DesktopSetting::Theme(theme) => ensure_gsetting(
            host,
            &format!("{prefix}.desktop.interface"),
            "color-scheme",
            match theme {
                DesktopTheme::Light => "'prefer-light'",
                DesktopTheme::Dark => "'prefer-dark'",
            },
        ),
        DesktopSetting::Terminal(executable) => {
            validate_executable(executable).context("validate desktop terminal operation")?;
            if !command_is_executable(host, executable) {
                bail!("desktop terminal executable {executable:?} is unavailable");
            }
            let schema = format!("{prefix}.desktop.default-applications.terminal");
            ensure_gsetting(host, &schema, "exec", &format!("'{executable}'"))?;
            ensure_gsetting(host, &schema, "exec-arg", "''")
        }
        DesktopSetting::IdleTimeoutSeconds(seconds) => ensure_gsetting(
            host,
            &format!("{prefix}.desktop.session"),
            "idle-delay",
            &format!("uint32 {seconds}"),
        ),
        DesktopSetting::IdleDim(enabled) => ensure_gsetting(
            host,
            &format!("{prefix}.settings-daemon.plugins.power"),
            "idle-dim",
            if *enabled { "true" } else { "false" },
        ),
    }
}
pub(crate) fn gnome_extensions(host: &Host<'_>, operation: &GnomeExtensionsOperation) -> Result<OperationOutcome> {
    validate_extensions(&operation.extensions).context("validate GNOME extensions operation")?;
    let mut outcome = OperationOutcome::Completed;
    for extension in &operation.extensions {
        if ensure_extension(host, extension)? == OperationOutcome::LoginRequired {
            outcome = OperationOutcome::LoginRequired;
        }
    }
    Ok(outcome)
}
pub(crate) fn gnome_dock(host: &Host<'_>, _: &GnomeDockOperation) -> Result<OperationOutcome> {
    let outcome = ensure_extension(host, DASH_TO_DOCK_UUID)?;
    if outcome == OperationOutcome::LoginRequired {
        return Ok(outcome);
    }
    let settings = [
        ("dock-position", "'BOTTOM'"),
        ("dash-max-icon-size", "32"),
        ("dock-fixed", "false"),
        ("autohide", "true"),
        ("require-pressure-to-show", "false"),
        ("intellihide", "true"),
        ("intellihide-mode", "'FOCUS_APPLICATION_WINDOWS'"),
        ("extend-height", "false"),
        ("click-action", "'minimize-or-previews'"),
    ];
    for (key, value) in settings {
        ensure_dconf(host, &format!("/org/gnome/shell/extensions/dash-to-dock/{key}"), value)?;
    }
    Ok(OperationOutcome::Completed)
}
pub(crate) fn gnome_rounded_corners(host: &Host<'_>, _: &GnomeRoundedCornersOperation) -> Result<OperationOutcome> {
    let outcome = ensure_extension(host, ROUNDED_CORNERS_UUID)?;
    if outcome == OperationOutcome::LoginRequired {
        return Ok(outcome);
    }
    let value = "{'padding': <{'left': uint32 1, 'right': 1, 'top': 1, 'bottom': 1}>, 'keepRoundedCorners': <{'maximized': false, 'fullscreen': false}>, 'borderRadius': <uint32 16>, 'smoothing': <0.5>, 'borderColor': <(0.5, 0.5, 0.5, 1.0)>, 'enabled': <true>}";
    ensure_dconf(
        host,
        "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings",
        value,
    )?;
    Ok(OperationOutcome::Completed)
}
fn ensure_extension(host: &Host<'_>, extension: &str) -> Result<OperationOutcome> {
    validate_extension(extension).context("validate fixed GNOME extension provider")?;
    let installed = extension_state(host, false)?;
    let newly_installed = !installed.contains(extension);
    if newly_installed {
        install_extension(host, extension)?;
        if !extension_state(host, false)?.contains(extension) {
            return Ok(OperationOutcome::LoginRequired);
        }
    }
    if !extension_state(host, true)?.contains(extension) {
        host.require("GNOME extension enable", "gnome-extensions", ["enable", extension])?;
    }
    if !extension_state(host, true)?.contains(extension) {
        return Ok(OperationOutcome::LoginRequired);
    }
    Ok(OperationOutcome::Completed)
}
fn ensure_gsetting(host: &Host<'_>, schema: &str, key: &str, expected: &str) -> Result<()> {
    let current = host.require("desktop setting query", "gsettings", ["get", schema, key])?;
    if state_line(&current.stdout, "gsettings")? != expected {
        host.require("desktop setting mutation", "gsettings", ["set", schema, key, expected])?;
    }
    let current = host.require("desktop setting postcondition", "gsettings", ["get", schema, key])?;
    if state_line(&current.stdout, "gsettings")? != expected {
        bail!("desktop setting {schema} {key} did not converge to {expected}");
    }
    Ok(())
}
fn ensure_dconf(host: &Host<'_>, key: &str, expected: &str) -> Result<()> {
    let current = host.require("GNOME dconf query", "dconf", ["read", key])?;
    if optional_state_line(&current.stdout, "dconf")? != Some(expected) {
        host.require("GNOME dconf mutation", "dconf", ["write", key, expected])?;
    }
    let current = host.require("GNOME dconf postcondition", "dconf", ["read", key])?;
    if optional_state_line(&current.stdout, "dconf")? != Some(expected) {
        bail!("GNOME dconf setting {key} did not converge to {expected}");
    }
    Ok(())
}
fn extension_state(host: &Host<'_>, enabled_only: bool) -> Result<BTreeSet<String>> {
    let mut command = vec!["list".to_owned()];
    if enabled_only {
        command.push("--enabled".into());
    }
    let output = host.require("GNOME extension state query", "gnome-extensions", command)?;
    let output = std::str::from_utf8(&output.stdout).context("gnome-extensions returned non-UTF-8 state")?;
    let mut extensions = BTreeSet::new();
    for extension in output.lines() {
        validate_extension(extension).map_err(|_| anyhow::anyhow!("gnome-extensions returned malformed UUID state"))?;
        if !extensions.insert(extension.to_owned()) {
            bail!("gnome-extensions returned duplicate UUID state");
        }
    }
    Ok(extensions)
}
fn install_extension(host: &Host<'_>, extension: &str) -> Result<()> {
    let endpoint = format!("https://extensions.gnome.org/extension-info/?uuid={extension}");
    let metadata = host.require("GNOME extension metadata", "curl", ["-fsSL", &endpoint])?;
    let shell = host.require("GNOME extension shell version", "gnome-shell", ["--version"])?;
    let shell_version =
        super::gnome_shell_version(std::str::from_utf8(&shell.stdout).context("GNOME Shell version is not UTF-8")?)?;
    let version = super::gnome_version(
        std::str::from_utf8(&metadata.stdout).context("GNOME extension metadata is not UTF-8")?,
        &shell_version,
    )?;
    let archive = TempPath::new_with_suffix(host, "gnome-extension", ".zip")?;
    let name = extension.replace('@', "");
    let url = format!("https://extensions.gnome.org/extension-data/{name}.v{version}.shell-extension.zip");
    host.require(
        "GNOME extension download",
        "curl",
        ["-fL", "-o", &archive.path().to_string_lossy(), &url],
    )?;
    host.require(
        "GNOME extension install",
        "gnome-extensions",
        ["install", "--force", &archive.path().to_string_lossy()],
    )?;
    Ok(())
}
fn command_is_executable(host: &Host<'_>, executable: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    host.value("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| {
            std::fs::metadata(directory.join(executable))
                .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        })
    })
}
fn validate_executable(value: &str) -> Result<()> {
    let mut bytes = value.bytes();
    if bytes.next().is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        bail!("invalid desktop terminal executable {value:?}");
    }
    Ok(())
}
fn validate_extensions(extensions: &[String]) -> Result<()> {
    if extensions.is_empty() {
        bail!("GNOME extension sequence must not be empty");
    }
    let mut seen = BTreeSet::new();
    for extension in extensions {
        validate_extension(extension)?;
        if !seen.insert(extension) {
            bail!("duplicate GNOME extension UUID {extension:?}");
        }
    }
    Ok(())
}
fn validate_extension(value: &str) -> Result<()> {
    let mut parts = value.split('@');
    if !valid_uuid_part(parts.next().unwrap_or_default())
        || !valid_uuid_part(parts.next().unwrap_or_default())
        || parts.next().is_some()
    {
        bail!("invalid GNOME extension UUID {value:?}");
    }
    Ok(())
}
fn valid_uuid_part(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}
fn state_line<'a>(output: &'a [u8], command: &str) -> Result<&'a str> {
    optional_state_line(output, command)?.context(format!("{command} returned empty state"))
}
fn optional_state_line<'a>(output: &'a [u8], command: &str) -> Result<Option<&'a str>> {
    let output = std::str::from_utf8(output).with_context(|| format!("{command} returned non-UTF-8 state"))?;
    let output = output.strip_suffix('\n').unwrap_or(output);
    if output.contains(['\n', '\r']) {
        bail!("{command} returned malformed multiline state");
    }
    Ok((!output.is_empty()).then_some(output))
}
