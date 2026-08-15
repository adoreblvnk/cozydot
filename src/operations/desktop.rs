use anyhow::{Result, bail};

use super::Host;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Gnome,
    Cinnamon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopSetting {
    ColorScheme(ColorScheme),
    Terminal(String),
    IdleDelaySeconds(u32),
    IdleDim(bool),
}

pub(crate) fn desktop_setting(host: &Host, target: DesktopEnvironment, setting: &DesktopSetting) -> Result<()> {
    let prefix = match target {
        DesktopEnvironment::Gnome => "org.gnome",
        DesktopEnvironment::Cinnamon => "org.cinnamon",
    };
    match setting {
        DesktopSetting::ColorScheme(color_scheme) => gsettings_set(
            host,
            &format!("{prefix}.desktop.interface"),
            "color-scheme",
            match color_scheme {
                ColorScheme::Light => "'prefer-light'",
                ColorScheme::Dark => "'prefer-dark'",
            },
        ),
        DesktopSetting::Terminal(executable) => {
            if !host.executable_on_path(executable) {
                bail!("desktop terminal executable {executable:?} is unavailable");
            }
            let schema = format!("{prefix}.desktop.default-applications.terminal");
            gsettings_set(host, &schema, "exec", &format!("'{executable}'"))?;
            gsettings_set(host, &schema, "exec-arg", "''")
        }
        DesktopSetting::IdleDelaySeconds(seconds) => {
            gsettings_set(host, &format!("{prefix}.desktop.session"), "idle-delay", &format!("uint32 {seconds}"))
        }
        DesktopSetting::IdleDim(enabled) => gsettings_set(
            host,
            &format!("{prefix}.settings-daemon.plugins.power"),
            "idle-dim",
            if *enabled { "true" } else { "false" },
        ),
    }
}

fn gsettings_set(host: &Host, schema: &str, key: &str, value: &str) -> Result<()> {
    host.require("gsettings set", "gsettings", ["set", schema, key, value])?;
    Ok(())
}
