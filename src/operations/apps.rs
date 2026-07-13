use super::Host;
use anyhow::{bail, Result};

pub(super) fn docker(host: &Host<'_>, user: &str) -> Result<()> {
    if !host.command_exists("docker") {
        return Ok(());
    }
    ensure_group(host, "docker config", "docker", user)?;
    host.require("docker config", "sudo", ["mkdir", "-p", "/etc/docker"])?;
    host.require(
        "docker config",
        "sudo",
        ["touch", "/etc/docker/daemon.json"],
    )?;
    let current = host.run("sudo", ["cat", "/etc/docker/daemon.json"])?;
    let local = serde_json::from_slice::<serde_json::Value>(&current.stdout)
        .ok()
        .and_then(|value| value["log-driver"].as_str().map(str::to_owned))
        .as_deref()
        == Some("local");
    if !local {
        host.require_input(
            "docker config",
            "sudo",
            ["tee", "/etc/docker/daemon.json"],
            b"{\"log-driver\":\"local\",\"log-opts\":{\"max-size\":\"10m\"}}\n",
        )?;
    }
    Ok(())
}

pub(super) fn virtualbox(host: &Host<'_>, user: &str) -> Result<()> {
    if host.command_exists("virtualbox") {
        ensure_group(host, "virtualbox config", "vboxusers", user)?;
    }
    Ok(())
}

pub(super) fn vscode_extension(host: &Host<'_>, extension: &str) -> Result<()> {
    if !host.command_exists("code") {
        bail!("VS Code extension: code is unavailable after installation");
    }
    let installed = host.require("VS Code extension", "code", ["--list-extensions"])?;
    if !String::from_utf8_lossy(&installed.stdout)
        .lines()
        .any(|line| line == extension)
    {
        host.require(
            "VS Code extension",
            "code",
            ["--install-extension", extension],
        )?;
    }
    Ok(())
}

pub(super) fn gnome_terminal(host: &Host<'_>, terminal: &str) -> Result<()> {
    if !host.command_exists(terminal) {
        bail!("GNOME terminal: configured command {terminal} is unavailable");
    }
    if host
        .run(
            "gsettings",
            [
                "get",
                "org.gnome.settings-daemon.plugins.media-keys",
                "terminal",
            ],
        )?
        .status
        .success()
    {
        host.require(
            "GNOME terminal",
            "gsettings",
            [
                "set",
                "org.gnome.desktop.default-applications.terminal",
                "exec",
                terminal,
            ],
        )?;
        host.require(
            "GNOME terminal",
            "gsettings",
            [
                "set",
                "org.gnome.desktop.default-applications.terminal",
                "exec-arg",
                "",
            ],
        )?;
    } else {
        let writes = [
            (
                "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings",
                "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']",
            ),
            (
                "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/name",
                "'Terminal'",
            ),
            (
                "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/command",
                &format!("'{terminal}'"),
            ),
            (
                "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/binding",
                "'<Primary><Alt>T'",
            ),
        ];
        for (key, value) in writes {
            host.require("GNOME terminal", "dconf", ["write", key, value])?;
        }
    }
    Ok(())
}

fn ensure_group(host: &Host<'_>, operation: &str, group: &str, user: &str) -> Result<()> {
    let entry = host.run("getent", ["group", group])?;
    if !String::from_utf8_lossy(&entry.stdout).contains(user) {
        host.require(operation, "sudo", ["groupadd", "-f", group])?;
        host.require(operation, "sudo", ["usermod", "-aG", group, user])?;
        let _ = host.run("newgrp", [group])?;
    }
    Ok(())
}
