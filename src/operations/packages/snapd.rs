use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::operations::{
    host::{Host, privileged_file::write_atomic, systemd},
    packages::apt,
};

const NO_SNAP_PIN: &str = "/etc/apt/preferences.d/nosnap.pref";

pub(crate) fn set_enabled(host: &Host, enabled: bool) -> Result<()> {
    if enabled {
        host.run("no-snap APT pin removal", "sudo", ["rm", "-f", NO_SNAP_PIN])?;
        apt::install(host, &["snapd".into()])?;
        host.run("snapd service enablement", "sudo", ["systemctl", "enable", "--now", "snapd.socket"])?;
        return Ok(());
    }

    remove_snaps(host)?;
    for unit in ["snapd.socket", "snapd.service", "snapd.seeded.service"] {
        if systemd::enabled_or_active(host, unit)? {
            host.run("snapd service disablement", "sudo", ["systemctl", "disable", "--now", unit])?;
        }
    }
    apt::purge(host, &["snapd".into()])?;
    let home_snap = host.home().join("snap");
    host.run(
        "snap data removal",
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
    write_atomic(host, Path::new(NO_SNAP_PIN), pin, "no-snap APT pin write")?;
    Ok(())
}

fn remove_snaps(host: &Host) -> Result<()> {
    let output = host.output("snap", ["list"])?;
    if !output.status.success() {
        return Ok(());
    }
    let output = std::str::from_utf8(&output.stdout).context("snap list returned non-UTF-8 output")?;
    let mut names = Vec::new();
    for line in output.lines().skip(1) {
        let name = line.split_ascii_whitespace().next().unwrap_or_default();
        if !valid_snap_name(name) {
            bail!("snap list returned malformed package row");
        }
        names.push(name.to_owned());
    }
    // remove app snaps before base & runtime snaps
    names.sort_by_key(|name| matches!(name.as_str(), "snapd" | "bare") || name.starts_with("core"));
    for name in names {
        host.run("snap package removal", "sudo", ["snap", "remove", "--purge", &name])?;
    }
    Ok(())
}

fn valid_snap_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
