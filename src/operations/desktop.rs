use anyhow::{Result, bail};

use super::host::Host;
use crate::config::Theme;

#[derive(Clone, Copy, PartialEq)]
pub enum DesktopEnvironment {
    Gnome,
    Cinnamon,
}

fn prefix(environment: DesktopEnvironment) -> &'static str {
    match environment {
        DesktopEnvironment::Gnome => "org.gnome",
        DesktopEnvironment::Cinnamon => "org.cinnamon",
    }
}

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
    let schema = format!("{}.desktop.default-applications.terminal", prefix(environment));
    gsettings_set(host, &schema, "exec", &format!("'{executable}'"))?;
    gsettings_set(host, &schema, "exec-arg", "''")
}

pub(crate) fn set_idle_delay(host: &Host, environment: DesktopEnvironment, seconds: u32) -> Result<()> {
    gsettings_set(host, &format!("{}.desktop.session", prefix(environment)), "idle-delay", &format!("uint32 {seconds}"))
}

pub(crate) fn set_idle_dim(host: &Host, environment: DesktopEnvironment, enabled: bool) -> Result<()> {
    let prefix = prefix(environment);
    gsettings_set(
        host,
        &format!("{prefix}.settings-daemon.plugins.power"),
        "idle-dim",
        if enabled { "true" } else { "false" },
    )
}

fn gsettings_set(host: &Host, schema: &str, key: &str, value: &str) -> Result<()> {
    host.run("gsettings set", "gsettings", ["set", schema, key, value])?;
    Ok(())
}
