use anyhow::{Result, bail};
use std::{
    fs::{self, File},
    io::Write,
};

use super::host::{Host, stdout_line};
use crate::config::Theme;

pub(crate) mod fonts;
pub(crate) mod gnome;
pub(crate) mod macos;

#[derive(Clone, Copy, PartialEq)]
pub enum DesktopEnvironment {
    Gnome,
    Cinnamon,
}

const GNOME_MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const GNOME_TERMINAL_SHORTCUT: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/cozydot-terminal/";

pub(crate) fn set_color_scheme(host: &Host, environment: DesktopEnvironment, color_scheme: Theme) -> Result<()> {
    gsettings_set(
        host,
        &format!("{}.desktop.interface", prefix(environment)),
        "color-scheme",
        match color_scheme {
            Theme::Light => "'prefer-light'",
            Theme::Dark => "'prefer-dark'",
        },
    )
}

pub(crate) fn set_terminal(host: &Host, environment: DesktopEnvironment, executable: &str) -> Result<()> {
    if !host.executable_on_path(executable) {
        bail!("desktop terminal executable {executable:?} is unavailable");
    }
    if environment == DesktopEnvironment::Gnome {
        set_xdg_terminal(host, executable)?;
        // Ubuntu provides this media key; upstream GNOME needs a custom binding.
        if !host.output("gsettings", ["get", GNOME_MEDIA_KEYS, "terminal"]).is_ok_and(|output| output.status.success())
        {
            ensure_gnome_terminal_shortcut(host, "xdg-terminal-exec")?;
        }
        return Ok(());
    }

    let schema = format!("{}.desktop.default-applications.terminal", prefix(environment));
    gsettings_set(host, &schema, "exec", &format!("'{executable}'"))?;
    gsettings_set(host, &schema, "exec-arg", "''")
}

fn set_xdg_terminal(host: &Host, executable: &str) -> Result<()> {
    let config_home = crate::paths::config_home()?;
    fs::create_dir_all(&config_home)?;
    let destination = config_home.join("xdg-terminals.list");
    let entry = format!("{executable}.desktop");
    let mut temp = tempfile::NamedTempFile::with_prefix_in(".xdg-terminals.", &config_home)?;
    writeln!(temp, "{entry}")?;
    temp.as_file_mut().sync_all()?;
    temp.persist(destination).map_err(|error| error.error)?;
    File::open(config_home)?.sync_all()?;

    let output = host.run("xdg-terminal-exec selection", "xdg-terminal-exec", ["--print-id"])?;
    if stdout_line(&output.stdout, "xdg-terminal-exec --print-id")? != entry.as_str() {
        bail!("xdg-terminal-exec did not select {entry:?}");
    }
    Ok(())
}

fn ensure_gnome_terminal_shortcut(host: &Host, executable: &str) -> Result<()> {
    let output = host.run("gsettings get", "gsettings", ["get", GNOME_MEDIA_KEYS, "custom-keybindings"])?;
    let keybindings = stdout_line(&output.stdout, "gsettings get custom-keybindings")?;
    let quoted_path = format!("'{GNOME_TERMINAL_SHORTCUT}'");
    let updated = if keybindings.contains(&quoted_path) {
        None
    } else if matches!(keybindings, "[]" | "@as []") {
        Some(format!("[{quoted_path}]"))
    } else if keybindings.starts_with('[')
        && let Some(existing) = keybindings.strip_suffix(']')
    {
        Some(format!("{existing}, {quoted_path}]"))
    } else {
        bail!("gsettings get custom-keybindings returned malformed output");
    };

    let schema = format!("{GNOME_MEDIA_KEYS}.custom-keybinding:{GNOME_TERMINAL_SHORTCUT}");
    // Complete the binding before publishing its path to GNOME.
    gsettings_set(host, &schema, "name", "'Terminal'")?;
    gsettings_set(host, &schema, "command", &format!("'{executable}'"))?;
    gsettings_set(host, &schema, "binding", "'<Primary><Alt>T'")?;
    if let Some(updated) = updated {
        gsettings_set(host, GNOME_MEDIA_KEYS, "custom-keybindings", &updated)?;
    }
    Ok(())
}

pub(crate) fn set_idle_delay(host: &Host, environment: DesktopEnvironment, seconds: u32) -> Result<()> {
    gsettings_set(host, &format!("{}.desktop.session", prefix(environment)), "idle-delay", &format!("uint32 {seconds}"))
}

pub(crate) fn set_idle_dim(host: &Host, environment: DesktopEnvironment, enabled: bool) -> Result<()> {
    let prefix = prefix(environment);
    let value = if enabled { "true" } else { "false" };
    gsettings_set(host, &format!("{prefix}.settings-daemon.plugins.power"), "idle-dim", value)
}

fn prefix(environment: DesktopEnvironment) -> &'static str {
    match environment {
        DesktopEnvironment::Gnome => "org.gnome",
        DesktopEnvironment::Cinnamon => "org.cinnamon",
    }
}

fn gsettings_set(host: &Host, schema: &str, key: &str, value: &str) -> Result<()> {
    host.run("gsettings set", "gsettings", ["set", schema, key, value])?;
    Ok(())
}
