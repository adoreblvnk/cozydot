use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashMap;

use super::host::{Host, TempPath};

#[derive(PartialEq)]
pub(crate) enum Outcome {
    Completed,
    LoginRequired,
}

const DASH_TO_DOCK_UUID: &str = "dash-to-dock@micxgx.gmail.com";
const ROUNDED_CORNERS_UUID: &str = "rounded-window-corners@fxgn";
const ROUNDED_CORNERS_SETTINGS: &str =
    "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings";

pub(crate) fn apply_extensions(host: &Host, extensions: &[String]) -> Result<Outcome> {
    let mut outcome = Outcome::Completed;
    for extension in extensions {
        if install_or_enable_extension(host, extension)? == Outcome::LoginRequired {
            outcome = Outcome::LoginRequired;
        }
    }
    Ok(outcome)
}

pub(crate) fn install_dash_to_dock(host: &Host) -> Result<Outcome> {
    if install_or_enable_extension(host, DASH_TO_DOCK_UUID)? == Outcome::LoginRequired {
        return Ok(Outcome::LoginRequired);
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
        let path = format!("/org/gnome/shell/extensions/dash-to-dock/{key}");
        host.run("dconf write", "dconf", ["write", &path, value])?;
    }
    Ok(Outcome::Completed)
}

pub(crate) fn install_rounded_window_corners(host: &Host) -> Result<Outcome> {
    if install_or_enable_extension(host, ROUNDED_CORNERS_UUID)? == Outcome::LoginRequired {
        return Ok(Outcome::LoginRequired);
    }
    let value = "{'padding': <{'left': uint32 1, 'right': 1, 'top': 1, 'bottom': 1}>, 'keepRoundedCorners': <{'maximized': false, 'fullscreen': false}>, 'borderRadius': <uint32 16>, 'smoothing': <0.5>, 'borderColor': <(0.5, 0.5, 0.5, 1.0)>, 'enabled': <true>}";
    host.run("dconf write", "dconf", ["write", ROUNDED_CORNERS_SETTINGS, value])?;
    Ok(Outcome::Completed)
}

fn install_or_enable_extension(host: &Host, extension: &str) -> Result<Outcome> {
    validate_extension(extension)?;
    if !host.output("gnome-extensions", ["info", extension])?.status.success() {
        install_extension(host, extension)?;
        // GNOME only finds newly installed extensions after next login
        return Ok(Outcome::LoginRequired);
    }
    host.run("GNOME extension enable", "gnome-extensions", ["enable", extension])?;
    Ok(Outcome::Completed)
}

fn install_extension(host: &Host, extension: &str) -> Result<()> {
    let endpoint = format!("https://extensions.gnome.org/extension-info/?uuid={extension}");
    let metadata = host.curl("GNOME extension metadata", &endpoint, std::iter::empty::<&str>())?;
    let shell = host.run("GNOME extension shell version", "gnome-shell", ["--version"])?;
    let shell_version = shell_version(std::str::from_utf8(&shell.stdout).context("GNOME Shell version is not UTF-8")?)?;
    let version = select_extension_version(
        std::str::from_utf8(&metadata.stdout).context("GNOME extension metadata is not UTF-8")?,
        &shell_version,
    )?;
    let archive = TempPath::new_with_suffix("gnome-extension", ".zip")?;
    let name = extension.replace('@', "");
    let url = format!("https://extensions.gnome.org/extension-data/{name}.v{version}.shell-extension.zip");
    host.curl("GNOME extension download", &url, ["--output", &archive.path().to_string_lossy()])?;
    host.run("GNOME extension install", "gnome-extensions", ["install", "--force", &archive.path().to_string_lossy()])?;
    Ok(())
}

fn validate_extension(value: &str) -> Result<()> {
    // UUIDs enter request URLs & archive names, so accept only GNOME's path-safe form
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

fn select_extension_version(input: &str, shell_version: &str) -> Result<u64> {
    #[derive(Deserialize)]
    struct Response {
        shell_version_map: HashMap<String, ExtensionVersion>,
    }

    #[derive(Deserialize)]
    struct ExtensionVersion {
        version: u64,
    }

    let response: Response = serde_json::from_str(input).context("parse GNOME extension JSON")?;
    let mut candidate = shell_version;
    loop {
        if let Some(extension) = response.shell_version_map.get(candidate) {
            return Ok(extension.version);
        }
        let Some((parent, _)) = candidate.rsplit_once('.') else {
            bail!("GNOME response has no extension version for shell {shell_version}");
        };
        candidate = parent;
    }
}

fn shell_version(input: &str) -> Result<String> {
    input
        .split_whitespace()
        .map(|part| part.trim_matches(|character: char| !character.is_ascii_digit() && character != '.'))
        .find(|part| {
            !part.is_empty()
                && part
                    .split('.')
                    .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .map(str::to_owned)
        .context("GNOME Shell version output has no numeric version")
}
