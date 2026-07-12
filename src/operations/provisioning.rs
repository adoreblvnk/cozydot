use super::{Host, TempPath};
use anyhow::{bail, Result};

pub fn apt_codecs(host: &Host<'_>, package: &str) -> Result<()> {
    host.require("codec installation", "sudo", ["apt-get", "update", "-qq"])?;
    host.require(
        "codec installation",
        "sudo",
        ["apt-get", "install", "-qq", package],
    )?;
    Ok(())
}

pub fn rustup(host: &Host<'_>) -> Result<()> {
    let installer = TempPath::new(host, "rustup")?;
    host.require(
        "rustup bootstrap",
        "curl",
        [
            "--proto",
            "=https",
            "--tlsv1.2",
            "-sSf",
            "-o",
            &installer.path().to_string_lossy(),
            "https://sh.rustup.rs",
        ],
    )?;
    host.require(
        "rustup bootstrap",
        "sh",
        [installer.path().as_os_str(), "-y".as_ref()],
    )?;
    Ok(())
}

pub fn repository_key(host: &Host<'_>, url: &str, destination: &str) -> Result<()> {
    let key = TempPath::new(host, "repository-key")?;
    host.require(
        "repository key download",
        "curl",
        ["-sSL", "-o", &key.path().to_string_lossy(), url],
    )?;
    host.require(
        "repository key conversion",
        "sudo",
        [
            "gpg",
            "--dearmor",
            "--yes",
            "--output",
            destination,
            &key.path().to_string_lossy(),
        ],
    )?;
    Ok(())
}

pub fn cargo_packages(host: &Host<'_>, packages: &[String]) -> Result<()> {
    let cargo_home = host.home().join(".cargo/bin");
    let cargo = if cargo_home.join("cargo").is_file() {
        cargo_home.join("cargo").to_string_lossy().into_owned()
    } else if host.command_exists("cargo") {
        "cargo".into()
    } else {
        return Ok(());
    };
    if !cargo_home.join("cargo-binstall").is_file() && !host.command_exists("cargo-binstall") {
        host.require(
            "cargo package installation",
            &cargo,
            ["install", "cargo-binstall", "--locked"],
        )?;
    }
    for package in packages {
        let mut args = vec!["binstall", "--no-confirm"];
        args.extend(package.split_whitespace());
        host.require("cargo package installation", &cargo, args)?;
    }
    Ok(())
}

pub fn gnome_dependencies(host: &Host<'_>) -> Result<()> {
    if host.command_exists("gnome-tweaks") && host.command_exists("gnome-extensions") {
        return Ok(());
    }
    host.require("GNOME dependencies", "sudo", ["apt-get", "update", "-qq"])?;
    host.require(
        "GNOME dependencies",
        "sudo",
        [
            "apt-get",
            "install",
            "-qq",
            "gnome-tweaks",
            "gnome-shell-extensions",
        ],
    )?;
    Ok(())
}

fn has_extension(host: &Host<'_>, needles: &[&str]) -> Result<bool> {
    let output = host.require("GNOME extension inspection", "gnome-extensions", ["list"])?;
    let list = String::from_utf8_lossy(&output.stdout);
    Ok(list
        .lines()
        .any(|line| needles.iter().any(|needle| line.contains(needle))))
}

pub fn gnome_dock_settings(host: &Host<'_>) -> Result<()> {
    if !has_extension(host, &["dash-to-dock", "ubuntu-dock"])? {
        return Ok(());
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
        host.require(
            "GNOME dock settings",
            "dconf",
            [
                "write",
                &format!("/org/gnome/shell/extensions/dash-to-dock/{key}"),
                value,
            ],
        )?;
    }
    Ok(())
}

pub fn gnome_rounded_settings(host: &Host<'_>) -> Result<()> {
    if !has_extension(host, &["rounded-window-corners"])? {
        return Ok(());
    }
    let value = "{'padding': <{'left': uint32 1, 'right': 1, 'top': 1, 'bottom': 1}>, 'keepRoundedCorners': <{'maximized': false, 'fullscreen': false}>, 'borderRadius': <uint32 16>, 'smoothing': <0.5>, 'borderColor': <(0.5, 0.5, 0.5, 1.0)>, 'enabled': <true>}";
    let output = host.require(
        "GNOME rounded corner settings",
        "dconf",
        [
            "write",
            "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings",
            value,
        ],
    )?;
    if !output.status.success() {
        bail!("GNOME rounded corner settings failed");
    }
    Ok(())
}
