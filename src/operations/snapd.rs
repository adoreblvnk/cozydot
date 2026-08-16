use anyhow::{Context, Result, bail};
use std::path::Path;

use super::{Host, apt, privileged_file::write_atomic};

const NO_SNAP_PIN: &str = "/etc/apt/preferences.d/cozydot-no-snap.pref";

pub(crate) fn set_snapd_enabled(host: &Host, enabled: bool) -> Result<()> {
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
    write_atomic(host, Path::new(NO_SNAP_PIN), pin, "no-Snap APT pin write")?;
    Ok(())
}

fn systemd_state(host: &Host, query: &str, unit: &str) -> Result<bool> {
    Ok(host.run("systemctl", [query, unit])?.status.success())
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
    // remove app snaps before base & runtime snaps
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
