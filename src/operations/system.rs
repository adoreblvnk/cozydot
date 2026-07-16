use anyhow::{bail, Context, Result};
use std::{collections::BTreeSet, path::Path};

use super::{apt, privileged_file::publish_bytes, Host};

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

pub(crate) fn unattended_upgrades(
    host: &Host<'_>,
    operation: &UnattendedUpgradesOperation,
) -> Result<()> {
    let contents = if operation.enabled {
        b"APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n"
            .as_slice()
    } else {
        b"APT::Periodic::Update-Package-Lists \"0\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n"
            .as_slice()
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
                [
                    "systemctl",
                    "enable",
                    "--now",
                    "unattended-upgrades.service",
                ],
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
                [
                    "systemctl",
                    "disable",
                    "--now",
                    "unattended-upgrades.service",
                ],
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
        host.require(
            "no-Snap APT pin removal",
            "sudo",
            ["rm", "-f", "--", NO_SNAP_PIN],
        )?;
        apt::packages(host, &["snapd".into()])?;
        if !systemd_state(host, "is-enabled", "snapd.socket")?
            || !systemd_state(host, "is-active", "snapd.socket")?
        {
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
    publish_bytes(
        host,
        Path::new(NO_SNAP_PIN),
        pin,
        "no-Snap APT pin publication",
    )?;
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
        let output = host.run(
            "sudo",
            [
                "test".as_ref(),
                "!".as_ref(),
                "-e".as_ref(),
                path.as_os_str(),
            ],
        )?;
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
    let output =
        std::str::from_utf8(&output.stdout).context("snap list returned non-UTF-8 state")?;
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
        host.require(
            "Snap package removal",
            "sudo",
            ["snap", "remove", "--purge", &name],
        )?;
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
        [
            "-W",
            "-f=${Package}\\t${db:Status-Abbrev}\\n",
            "--",
            package,
        ],
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
    if systemd_state(host, "is-enabled", unit)? != expected
        || systemd_state(host, "is-active", unit)? != expected
    {
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
    let output = host.require(
        "effective user query",
        "getent",
        ["passwd", &uid.to_string()],
    )?;
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
    Ok(Some(
        fields[2]
            .parse()
            .context("getent group returned malformed GID")?,
    ))
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
    let output = std::str::from_utf8(bytes)
        .with_context(|| format!("{command} returned non-UTF-8 output"))?;
    let record = output.strip_suffix('\n').unwrap_or(output);
    if record.is_empty() || record.contains(['\n', '\r']) {
        bail!("{command} returned malformed record output");
    }
    Ok(record)
}
