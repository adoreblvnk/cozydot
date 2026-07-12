use super::Host;
use anyhow::Result;

pub(super) fn execute(host: &Host<'_>) -> Result<()> {
    if host.command_exists("snap") {
        let output = host.run("snap", ["list"])?;
        let mut core = None;
        if output.status.success() {
            for name in String::from_utf8_lossy(&output.stdout)
                .lines()
                .skip(1)
                .filter_map(|line| line.split_whitespace().next())
            {
                if name.starts_with("core")
                    && name.len() == 6
                    && name[4..].bytes().all(|byte| byte.is_ascii_digit())
                {
                    core.get_or_insert_with(|| name.to_owned());
                } else if name != "bare" && name != "snapd" {
                    host.require("snap cleanup", "snap", ["remove", "--purge", name])?;
                }
            }
        }
        if let Some(core) = core {
            let _ = host.run("sudo", ["snap", "remove", "--purge", &core])?;
        }
        let _ = host.run("sudo", ["snap", "remove", "--purge", "bare"])?;
        let _ = host.run("sudo", ["snap", "remove", "--purge", "snapd"])?;
    }
    if service_active(host, "snapd")? {
        host.require("snap cleanup", "sudo", ["systemctl", "stop", "snapd"])?;
        host.require("snap cleanup", "sudo", ["systemctl", "disable", "snapd"])?;
    }
    if host.command_exists("snap") {
        host.require("snap cleanup", "sudo", ["apt-get", "purge", "-qq", "snapd"])?;
    }
    if service_active(host, "snapd.mounts-pre.target")? {
        host.require(
            "snap cleanup",
            "sudo",
            ["systemctl", "stop", "snapd.mounts-pre.target"],
        )?;
    }
    let home_snap = host.home().join("snap");
    if home_snap.is_dir()
        || ["/snap", "/var/snap", "/var/lib/snapd"]
            .iter()
            .any(|path| std::path::Path::new(path).is_dir())
    {
        host.require(
            "snap cleanup",
            "sudo",
            [
                "rm",
                "-rf",
                &home_snap.to_string_lossy(),
                "/snap",
                "/var/snap",
                "/var/lib/snapd",
            ],
        )?;
    }
    Ok(())
}

fn service_active(host: &Host<'_>, service: &str) -> Result<bool> {
    Ok(host
        .run("systemctl", ["-q", "is-active", service])?
        .status
        .success())
}
