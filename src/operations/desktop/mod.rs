use anyhow::{Result, bail, ensure};
use std::{
    fs::{self, File},
    io::Write,
};

use super::host::{self, stdout_line};
use crate::config::Theme;

pub(crate) mod fonts;
pub(crate) mod gnome;
pub(crate) mod macos;

const GNOME_MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const GNOME_TERMINAL_SHORTCUT: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/cozydot-terminal/";

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
    File::open(config_home)?.sync_all()?;

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

pub(crate) fn set_idle_delay(seconds: u32) -> Result<()> {
    gsettings_set("org.gnome.desktop.session", "idle-delay", &format!("uint32 {seconds}"))
}

pub(crate) fn set_idle_dim(enabled: bool) -> Result<()> {
    gsettings_set("org.gnome.settings-daemon.plugins.power", "idle-dim", if enabled { "true" } else { "false" })
}

fn gsettings_set(schema: &str, key: &str, value: &str) -> Result<()> {
    host::run("gsettings set", "gsettings", ["set", schema, key, value])?;
    Ok(())
}
