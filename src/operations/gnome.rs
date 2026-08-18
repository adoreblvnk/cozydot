use anyhow::{Context, Result, bail};

use super::{Host, OperationOutcome, TempPath};

const DASH_TO_DOCK_UUID: &str = "dash-to-dock@micxgx.gmail.com";
const ROUNDED_CORNERS_UUID: &str = "rounded-window-corners@fxgn";
const ROUNDED_CORNERS_SETTINGS: &str =
    "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings";

pub(crate) fn apply_extensions(host: &Host, extensions: &[String]) -> Result<OperationOutcome> {
    let mut outcome = OperationOutcome::Completed;
    for extension in extensions {
        if install_or_enable_extension(host, extension)? == OperationOutcome::LoginRequired {
            outcome = OperationOutcome::LoginRequired;
        }
    }
    Ok(outcome)
}

pub(crate) fn install_dash_to_dock(host: &Host) -> Result<OperationOutcome> {
    if install_or_enable_extension(host, DASH_TO_DOCK_UUID)? == OperationOutcome::LoginRequired {
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
        let path = format!("/org/gnome/shell/extensions/dash-to-dock/{key}");
        host.run("dconf write", "dconf", ["write", &path, value])?;
    }
    Ok(OperationOutcome::Completed)
}

pub(crate) fn install_rounded_window_corners(host: &Host) -> Result<OperationOutcome> {
    if install_or_enable_extension(host, ROUNDED_CORNERS_UUID)? == OperationOutcome::LoginRequired {
        return Ok(OperationOutcome::LoginRequired);
    }
    let value = "{'padding': <{'left': uint32 1, 'right': 1, 'top': 1, 'bottom': 1}>, 'keepRoundedCorners': <{'maximized': false, 'fullscreen': false}>, 'borderRadius': <uint32 16>, 'smoothing': <0.5>, 'borderColor': <(0.5, 0.5, 0.5, 1.0)>, 'enabled': <true>}";
    host.run("dconf write", "dconf", ["write", ROUNDED_CORNERS_SETTINGS, value])?;
    Ok(OperationOutcome::Completed)
}

fn install_or_enable_extension(host: &Host, extension: &str) -> Result<OperationOutcome> {
    validate_extension(extension)?;
    if !host.output("gnome-extensions", ["info", extension])?.status.success() {
        install_extension(host, extension)?;
        // GNOME only finds newly installed extensions after next login
        return Ok(OperationOutcome::LoginRequired);
    }
    host.run("GNOME extension enable", "gnome-extensions", ["enable", extension])?;
    Ok(OperationOutcome::Completed)
}

fn install_extension(host: &Host, extension: &str) -> Result<()> {
    let endpoint = format!("https://extensions.gnome.org/extension-info/?uuid={extension}");
    let metadata = host.curl("GNOME extension metadata", &endpoint, std::iter::empty::<&str>())?;
    let shell = host.run("GNOME extension shell version", "gnome-shell", ["--version"])?;
    let shell_version =
        super::gnome_shell_version(std::str::from_utf8(&shell.stdout).context("GNOME Shell version is not UTF-8")?)?;
    let version = super::select_gnome_extension_version(
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
