use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::path::Path;

use super::{Host, OperationOutcome, TempPath, apt, privileged_file::publish_bytes};

const AUTO_UPGRADES: &str = "/etc/apt/apt.conf.d/20auto-upgrades";
const NO_SNAP_PIN: &str = "/etc/apt/preferences.d/cozydot-no-snap.pref";

pub(crate) fn ensure_admin(host: &Host) -> Result<()> {
    let (username, _) = effective_user(host)?;
    host.require("administrative group membership", "sudo", ["usermod", "-aG", "sudo", "--", &username])?;
    Ok(())
}

pub(crate) fn unattended_upgrades(host: &Host, enabled: bool) -> Result<()> {
    let contents = if enabled {
        b"APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n".as_slice()
    } else {
        b"APT::Periodic::Update-Package-Lists \"0\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n".as_slice()
    };
    if enabled {
        apt::packages(host, &["unattended-upgrades".into()])?;
        publish_bytes(host, Path::new(AUTO_UPGRADES), contents, "unattended-upgrades periodic configuration")?;
        host.require(
            "unattended-upgrades service enablement",
            "sudo",
            ["systemctl", "enable", "--now", "unattended-upgrades.service"],
        )?;
    } else {
        publish_bytes(host, Path::new(AUTO_UPGRADES), contents, "unattended-upgrades periodic configuration")?;
        let is_enabled = systemd_state(host, "is-enabled", "unattended-upgrades.service")?;
        let is_active = systemd_state(host, "is-active", "unattended-upgrades.service")?;
        if is_enabled || is_active {
            host.require(
                "unattended-upgrades service disablement",
                "sudo",
                ["systemctl", "disable", "--now", "unattended-upgrades.service"],
            )?;
        }
        apt::purge(host, &["unattended-upgrades".into()])?;
    }
    Ok(())
}

fn systemd_state(host: &Host, query: &str, unit: &str) -> Result<bool> {
    Ok(host.run("systemctl", [query, unit])?.status.success())
}

pub(crate) fn ubuntu_snap(host: &Host, enabled: bool) -> Result<()> {
    if enabled {
        host.require("no-Snap APT pin removal", "sudo", ["rm", "-f", "--", NO_SNAP_PIN])?;
        apt::packages(host, &["snapd".into()])?;
        host.require("Snap service enablement", "sudo", ["systemctl", "enable", "--now", "snapd.socket"])?;
        return Ok(());
    }

    remove_snaps(host)?;
    for unit in ["snapd.socket", "snapd.service", "snapd.seeded.service"] {
        let is_enabled = systemd_state(host, "is-enabled", unit)?;
        let is_active = systemd_state(host, "is-active", unit)?;
        if is_enabled || is_active {
            host.require("Snap service disablement", "sudo", ["systemctl", "disable", "--now", unit])?;
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
    Ok(())
}

fn remove_snaps(host: &Host) -> Result<()> {
    let output = host.run("snap", ["list"])?;
    if !output.status.success() {
        return Ok(());
    }
    let output = std::str::from_utf8(&output.stdout).context("snap list returned non-UTF-8 state")?;
    let mut names = Vec::new();
    for line in output.lines().skip(1) {
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
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn effective_user(host: &Host) -> Result<(String, u32)> {
    let uid = rustix::process::geteuid().as_raw();
    let output = host.require("effective user query", "getent", ["passwd", &uid.to_string()])?;
    let record = one_record(&output.stdout, "getent passwd")?;
    let fields = record.split(':').collect::<Vec<_>>();
    if fields.len() != 7 || fields[0].is_empty() || fields[2].parse::<u32>().ok() != Some(uid) {
        bail!("getent passwd returned a malformed effective-user record");
    }
    Ok((fields[0].to_owned(), uid))
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

pub(crate) fn docker_group(host: &Host) -> Result<()> {
    ensure_product_group(host, Product::Docker)
}

pub(crate) fn docker_local_log(host: &Host, max_size: Option<&str>) -> Result<()> {
    preflight(host, Product::Docker)?;
    let mut requested = read_daemon_config(host)?;
    let object = requested.as_object_mut().context("Docker daemon config must be a JSON object")?;
    object.insert("log-driver".into(), Value::String("local".into()));
    if let Some(max_size) = max_size {
        let log_options = object
            .entry("log-opts")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .context("Docker daemon config log-opts must be a JSON object")?;
        log_options.insert("max-size".into(), Value::String(max_size.to_owned()));
    }
    let mut bytes = serde_json::to_vec_pretty(&requested).context("serialize Docker daemon configuration")?;
    bytes.push(b'\n');
    publish_bytes(host, Path::new(DOCKER_DAEMON_CONFIG), &bytes, "Docker daemon config publication")?;
    Ok(())
}

pub(crate) fn virtualbox_group(host: &Host) -> Result<()> {
    ensure_product_group(host, Product::VirtualBox)
}

pub(crate) fn vscode_extensions(host: &Host, extensions: &[String]) -> Result<()> {
    let program = if cfg!(target_os = "macos") {
        [
            "code",
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
            "/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/bin/code",
        ]
        .into_iter()
        .find(|candidate| host.run(candidate, ["--version"]).is_ok_and(|output| output.status.success()))
        .context("VS Code integration requires the VS Code CLI; install the `code` shell command or the visual-studio-code cask")?
    } else {
        "code"
    };
    for extension in extensions {
        host.require("VS Code extension installation", program, ["--install-extension", extension.as_str()])?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Product {
    Docker,
    VirtualBox,
}

impl Product {
    fn label(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::VirtualBox => "VirtualBox",
        }
    }
    fn program(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::VirtualBox => "VBoxManage",
        }
    }
    fn group(self) -> Option<&'static str> {
        match self {
            Self::Docker => Some("docker"),
            Self::VirtualBox => Some("vboxusers"),
        }
    }
}

fn preflight(host: &Host, product: Product) -> Result<()> {
    host.require(&format!("{} existing-product preflight", product.label()), product.program(), ["--version"])
        .with_context(|| {
            format!("{} integration requires an existing usable {} CLI", product.label(), product.program())
        })?;
    Ok(())
}

fn ensure_product_group(host: &Host, product: Product) -> Result<()> {
    let (username, _) = effective_user(host)?;
    let group = product.group().context("group integration requires a system group")?;
    let groups =
        host.require(&format!("{} group membership query", product.label()), "id", ["-nG", "--", &username])?;
    if one_record(&groups.stdout, "id -nG")?.split_ascii_whitespace().any(|current| current == group) {
        return Ok(());
    }
    preflight(host, product)?;
    host.require(&format!("{} group creation", product.label()), "sudo", ["groupadd", "-f", group])?;
    host.require(
        &format!("{} group membership", product.label()),
        "sudo",
        ["usermod", "-aG", group, "--", username.as_str()],
    )?;
    Ok(())
}

fn read_daemon_config(host: &Host) -> Result<Value> {
    let kind = host.run("sudo", ["stat", "--format=%f", "--", DOCKER_DAEMON_CONFIG])?;
    if !kind.status.success() {
        host.require("Docker daemon config absence check", "sudo", ["test", "!", "-e", DOCKER_DAEMON_CONFIG])?;
        host.require("Docker daemon config symlink absence check", "sudo", ["test", "!", "-L", DOCKER_DAEMON_CONFIG])?;
        return Ok(Value::Object(Map::new()));
    }
    let mode = one_record(&kind.stdout, "sudo stat")?;
    let mode = u32::from_str_radix(mode, 16).context("sudo stat returned malformed mode output")?;
    if mode & 0o170000 != 0o100000 {
        bail!("Docker daemon config destination is not a regular file");
    }
    let output = host.require("Docker daemon config inspection", "sudo", ["cat", "--", DOCKER_DAEMON_CONFIG])?;
    let text = std::str::from_utf8(&output.stdout).context("Docker daemon config is not valid UTF-8")?;
    let value: Value = serde_json::from_str(text).context("Docker daemon config is invalid JSON")?;
    if !value.is_object() {
        bail!("Docker daemon config must be a JSON object");
    }
    Ok(value)
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

pub(crate) fn desktop_setting(host: &Host, target: DesktopEnvironment, setting: &DesktopSetting) -> Result<()> {
    let prefix = match target {
        DesktopEnvironment::Gnome => "org.gnome",
        DesktopEnvironment::Cinnamon => "org.cinnamon",
    };
    match setting {
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
            if !host.executable_on_path(executable) {
                bail!("desktop terminal executable {executable:?} is unavailable");
            }
            let schema = format!("{prefix}.desktop.default-applications.terminal");
            ensure_gsetting(host, &schema, "exec", &format!("'{executable}'"))?;
            ensure_gsetting(host, &schema, "exec-arg", "''")
        }
        DesktopSetting::IdleTimeoutSeconds(seconds) => {
            ensure_gsetting(host, &format!("{prefix}.desktop.session"), "idle-delay", &format!("uint32 {seconds}"))
        }
        DesktopSetting::IdleDim(enabled) => ensure_gsetting(
            host,
            &format!("{prefix}.settings-daemon.plugins.power"),
            "idle-dim",
            if *enabled { "true" } else { "false" },
        ),
    }
}

pub(crate) fn gnome_extensions(host: &Host, extensions: &[String]) -> Result<OperationOutcome> {
    let mut outcome = OperationOutcome::Completed;
    for extension in extensions {
        if ensure_extension(host, extension)? == OperationOutcome::LoginRequired {
            outcome = OperationOutcome::LoginRequired;
        }
    }
    Ok(outcome)
}

pub(crate) fn gnome_dock(host: &Host) -> Result<OperationOutcome> {
    if ensure_extension(host, DASH_TO_DOCK_UUID)? == OperationOutcome::LoginRequired {
        return Ok(OperationOutcome::LoginRequired);
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

pub(crate) fn gnome_rounded_corners(host: &Host) -> Result<OperationOutcome> {
    if ensure_extension(host, ROUNDED_CORNERS_UUID)? == OperationOutcome::LoginRequired {
        return Ok(OperationOutcome::LoginRequired);
    }
    let value = "{'padding': <{'left': uint32 1, 'right': 1, 'top': 1, 'bottom': 1}>, 'keepRoundedCorners': <{'maximized': false, 'fullscreen': false}>, 'borderRadius': <uint32 16>, 'smoothing': <0.5>, 'borderColor': <(0.5, 0.5, 0.5, 1.0)>, 'enabled': <true>}";
    ensure_dconf(
        host,
        "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings",
        value,
    )?;
    Ok(OperationOutcome::Completed)
}

fn ensure_extension(host: &Host, extension: &str) -> Result<OperationOutcome> {
    validate_extension(extension)?;
    if !host.run("gnome-extensions", ["info", extension])?.status.success() {
        install_extension(host, extension)?;
        return Ok(OperationOutcome::LoginRequired);
    }
    host.require("GNOME extension enable", "gnome-extensions", ["enable", extension])?;
    Ok(OperationOutcome::Completed)
}

fn ensure_gsetting(host: &Host, schema: &str, key: &str, expected: &str) -> Result<()> {
    host.require("desktop setting mutation", "gsettings", ["set", schema, key, expected])?;
    Ok(())
}

fn ensure_dconf(host: &Host, key: &str, expected: &str) -> Result<()> {
    host.require("GNOME dconf mutation", "dconf", ["write", key, expected])?;
    Ok(())
}

fn install_extension(host: &Host, extension: &str) -> Result<()> {
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
    host.require("GNOME extension download", "curl", ["-fL", "-o", &archive.path().to_string_lossy(), &url])?;
    host.require(
        "GNOME extension install",
        "gnome-extensions",
        ["install", "--force", &archive.path().to_string_lossy()],
    )?;
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
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}
