use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashMap;

use crate::operations::host::{self, TempPath};

#[derive(PartialEq)]
pub(crate) enum Outcome {
    Completed,
    LoginRequired,
}

const DASH_TO_DOCK_UUID: &str = "dash-to-dock@micxgx.gmail.com";
const ROUNDED_CORNERS_UUID: &str = "rounded-window-corners@fxgn";
const ROUNDED_CORNERS_SETTINGS: &str =
    "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings";

pub(crate) fn apply_extensions(extensions: &[String]) -> Result<Outcome> {
    let mut outcome = Outcome::Completed;
    for uuid in extensions {
        if install_or_enable_extension(uuid)? == Outcome::LoginRequired {
            outcome = Outcome::LoginRequired;
        }
    }
    Ok(outcome)
}

pub(crate) fn apply_dash_to_dock() -> Result<Outcome> {
    if install_or_enable_extension(DASH_TO_DOCK_UUID)? == Outcome::LoginRequired {
        return Ok(Outcome::LoginRequired);
    }
    for (key, value) in [
        ("dock-position", "'BOTTOM'"),
        ("dash-max-icon-size", "32"),
        ("dock-fixed", "false"),
        ("autohide", "true"),
        ("require-pressure-to-show", "false"),
        ("intellihide", "true"),
        ("intellihide-mode", "'FOCUS_APPLICATION_WINDOWS'"),
        ("extend-height", "false"),
        ("click-action", "'minimize-or-previews'"),
    ] {
        let path = format!("/org/gnome/shell/extensions/dash-to-dock/{key}");
        host::run("dconf write", "dconf", ["write", &path, value])?;
    }
    Ok(Outcome::Completed)
}

pub(crate) fn apply_rounded_window_corners() -> Result<Outcome> {
    if install_or_enable_extension(ROUNDED_CORNERS_UUID)? == Outcome::LoginRequired {
        return Ok(Outcome::LoginRequired);
    }
    let value = "{'padding': <{'left': uint32 1, 'right': 1, 'top': 1, 'bottom': 1}>, 'keepRoundedCorners': <{'maximized': false, 'fullscreen': false}>, 'borderRadius': <uint32 16>, 'smoothing': <0.5>, 'borderColor': <(0.5, 0.5, 0.5, 1.0)>, 'enabled': <true>}";
    host::run("dconf write", "dconf", ["write", ROUNDED_CORNERS_SETTINGS, value])?;
    Ok(Outcome::Completed)
}

fn install_or_enable_extension(uuid: &str) -> Result<Outcome> {
    validate_uuid(uuid)?;
    if !host::output("gnome-extensions", ["info", uuid])?.status.success() {
        install_extension(uuid)?;
        // GNOME only finds newly installed extensions after next login
        return Ok(Outcome::LoginRequired);
    }
    host::run("GNOME extension enable", "gnome-extensions", ["enable", uuid])?;
    Ok(Outcome::Completed)
}

fn validate_uuid(value: &str) -> Result<()> {
    // UUIDs enter request URLs & archive names, so accept only GNOME's path-safe form
    let Some((left, right)) = value.split_once('@') else {
        bail!("invalid GNOME extension UUID {value:?}");
    };
    if !valid_uuid_part(left) || !valid_uuid_part(right) {
        bail!("invalid GNOME extension UUID {value:?}");
    }
    Ok(())
}

fn valid_uuid_part(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn install_extension(uuid: &str) -> Result<()> {
    // TODO: can we simplify this?
    let endpoint = format!("https://extensions.gnome.org/extension-info/?uuid={uuid}");
    let metadata = host::curl("GNOME extension metadata", &endpoint, std::iter::empty::<&str>())?;
    let shell = host::run("GNOME extension shell version", "gnome-shell", ["--version"])?;
    let shell_version = shell_version(std::str::from_utf8(&shell.stdout).context("GNOME Shell version is not UTF-8")?)?;
    let metadata = std::str::from_utf8(&metadata.stdout).context("GNOME extension metadata is not UTF-8")?;
    let version = select_extension_version(metadata, shell_version)?;
    let archive = TempPath::new_with_suffix("gnome-extension", ".zip")?;
    let name = uuid.replace('@', "");
    let url = format!("https://extensions.gnome.org/extension-data/{name}.v{version}.shell-extension.zip");
    host::curl("GNOME extension download", &url, ["--output", &archive.path().to_string_lossy()])?;
    host::run(
        "GNOME extension install",
        "gnome-extensions",
        ["install", "--force", &archive.path().to_string_lossy()],
    )?;
    Ok(())
}

fn shell_version(input: &str) -> Result<&str> {
    // TODO: can we simplify this?
    for token in input.split_whitespace() {
        let candidate = token.trim_matches(|character: char| !character.is_ascii_digit() && character != '.');
        let mut is_version = !candidate.is_empty();
        for component in candidate.split('.') {
            if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
                is_version = false;
                break;
            }
        }
        if is_version {
            return Ok(candidate);
        }
    }
    bail!("GNOME Shell version output has no numeric version")
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
