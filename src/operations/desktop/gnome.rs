use std::{collections::HashMap, fs, io::Write};

use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde::Deserialize;

use crate::{
    config::{Gnome, Theme},
    operations::host::{self, stdout_line},
};

#[derive(PartialEq)]
pub(crate) enum Outcome {
    Completed,
    LoginRequired,
}

const GNOME_MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const GNOME_TERMINAL_SHORTCUT: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/cozydot-terminal/";
const DASH_TO_DOCK_UUID: &str = "dash-to-dock@micxgx.gmail.com";
const ROUNDED_CORNERS_UUID: &str = "rounded-window-corners@fxgn";
const ROUNDED_CORNERS_SETTINGS: &str =
    "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings";

pub(crate) fn apply_settings(gnome: &Gnome) -> Result<()> {
    if let Some(button_layout) = &gnome.button_layout {
        gsettings_set("org.gnome.desktop.wm.preferences", "button-layout", &format!("'{button_layout}'"))?;
    }
    if let Some(files) = &gnome.files {
        if let Some(hidden) = files.show_hidden_files {
            let value = if hidden { "true" } else { "false" };
            gsettings_set("org.gtk.gtk4.Settings.FileChooser", "show-hidden", value)?;
        }
        if let Some(folders_first) = files.sort_folders_first {
            let value = if folders_first { "true" } else { "false" };
            gsettings_set("org.gtk.gtk4.Settings.FileChooser", "sort-directories-first", value)?;
        }
    }
    if let Some(Some(timeout)) = gnome.idle.as_ref().map(|i| i.timeout.as_deref()) {
        let seconds = humantime::parse_duration(timeout)?.as_secs();
        gsettings_set("org.gnome.desktop.session", "idle-delay", &format!("uint32 {seconds}"))?;
    }
    if let Some(keyboard) = &gnome.keyboard {
        let schema = "org.gnome.desktop.peripherals.keyboard";
        if let Some(delay) = keyboard.delay {
            gsettings_set(schema, "delay", &format!("uint32 {delay}"))?;
        }
        if let Some(interval) = keyboard.repeat_interval {
            gsettings_set(schema, "repeat-interval", &format!("uint32 {interval}"))?;
        }
    }
    Ok(())
}

pub(crate) fn set_color_scheme(color_scheme: Theme) -> Result<()> {
    gsettings_set(
        "org.gnome.desktop.interface",
        "color-scheme",
        match color_scheme {
            Theme::Light => "'prefer-light'",
            Theme::Dark => "'prefer-dark'",
        },
    )
}

pub(crate) fn set_terminal(executable: &str) -> Result<()> {
    ensure!(host::has_executable_on_path(executable), "desktop terminal executable {executable:?} is unavailable");
    set_xdg_terminal(executable)?;
    // Ubuntu provides this media key; upstream GNOME needs a custom binding
    if !host::output("gsettings", ["get", GNOME_MEDIA_KEYS, "terminal"]).is_ok_and(|output| output.status.success()) {
        ensure_gnome_terminal_shortcut("xdg-terminal-exec")?;
    }
    Ok(())
}

fn set_xdg_terminal(executable: &str) -> Result<()> {
    let config_home = crate::paths::config_home()?;
    fs::create_dir_all(&config_home)?;
    let entry = format!("{executable}.desktop");
    let mut temp = tempfile::NamedTempFile::with_prefix_in(".xdg-terminals.", &config_home)?;
    writeln!(temp, "{entry}")?;
    temp.as_file_mut().sync_all()?;
    temp.persist(config_home.join("xdg-terminals.list"))?;
    fs::File::open(config_home)?.sync_all()?;

    let output = host::run("xdg-terminal-exec selection", "xdg-terminal-exec", ["--print-id"])?;
    if stdout_line(&output.stdout, "xdg-terminal-exec --print-id")? != entry.as_str() {
        bail!("xdg-terminal-exec did not select {entry:?}");
    }
    Ok(())
}

fn ensure_gnome_terminal_shortcut(executable: &str) -> Result<()> {
    let output = host::run("gsettings get", "gsettings", ["get", GNOME_MEDIA_KEYS, "custom-keybindings"])?;
    let keybindings = stdout_line(&output.stdout, "gsettings get custom-keybindings")?;
    let quoted_path = format!("'{GNOME_TERMINAL_SHORTCUT}'");
    // gsettings renders an empty string array as [] or @as []
    let updated = if keybindings.contains(&quoted_path) {
        None
    } else if matches!(keybindings, "[]" | "@as []") {
        Some(format!("[{quoted_path}]"))
    } else if let Some(existing) = keybindings.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
        Some(format!("[{existing}, {quoted_path}]"))
    } else {
        bail!("gsettings get custom-keybindings returned malformed output");
    };

    let schema = format!("{GNOME_MEDIA_KEYS}.custom-keybinding:{GNOME_TERMINAL_SHORTCUT}");
    // complete the binding before publishing its path to GNOME
    gsettings_set(&schema, "name", "'Terminal'")?;
    gsettings_set(&schema, "command", &format!("'{executable}'"))?;
    gsettings_set(&schema, "binding", "'<Primary><Alt>T'")?;
    if let Some(updated) = updated {
        gsettings_set(GNOME_MEDIA_KEYS, "custom-keybindings", &updated)?;
    }
    Ok(())
}

pub(crate) fn gsettings_set(schema: &str, key: &str, value: &str) -> Result<()> {
    host::run("gsettings set", "gsettings", ["set", schema, key, value])?;
    Ok(())
}

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
    // UUIDs enter request URLs & archive names, so accept only GNOME's path-safe form
    let valid = Regex::new(r"^[A-Za-z0-9._-]+@[A-Za-z0-9._-]+$")?.is_match(uuid);
    ensure!(valid, "invalid GNOME extension UUID {uuid:?}");
    if !host::output("gnome-extensions", ["info", uuid])?.status.success() {
        install_extension(uuid)?;
        // GNOME only finds newly installed extensions after next login
        return Ok(Outcome::LoginRequired);
    }
    host::run("GNOME extension enable", "gnome-extensions", ["enable", uuid])?;
    Ok(Outcome::Completed)
}

fn install_extension(uuid: &str) -> Result<()> {
    let endpoint = format!("https://extensions.gnome.org/extension-info/?uuid={uuid}");
    let metadata = host::curl("GNOME extension metadata", &endpoint, std::iter::empty::<&str>())?;
    let shell = host::run("GNOME extension shell version", "gnome-shell", ["--version"])?;
    let shell_version = shell_version(host::stdout_line(&shell.stdout, "gnome-shell --version")?)?;
    let metadata = std::str::from_utf8(&metadata.stdout).context("GNOME extension metadata is not UTF-8")?;
    let version = select_extension_version(metadata, shell_version)?;
    let archive = host::temp_path("gnome-extension", ".zip")?;
    // extension archive names omit @ although metadata UUIDs retain it
    let name = uuid.replace('@', "");
    let url = format!("https://extensions.gnome.org/extension-data/{name}.v{version}.shell-extension.zip");
    host::curl("GNOME extension download", &url, ["--output", &archive.to_string_lossy()])?;
    host::run("GNOME extension install", "gnome-extensions", ["install", "--force", &archive.to_string_lossy()])?;
    Ok(())
}

fn shell_version(input: &str) -> Result<&str> {
    let pattern = Regex::new(r"^[0-9]+(\.[0-9]+)*$")?;
    for token in input.split_whitespace() {
        let candidate = token.trim_matches(|character: char| !character.is_ascii_digit() && character != '.');
        if pattern.is_match(candidate) {
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
    // metadata may key compatibility by the full shell version or a shorter parent
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
