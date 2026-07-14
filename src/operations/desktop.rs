use super::{Host, TempPath};
use crate::json_helpers;
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Gnome,
    Cinnamon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopTheme {
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopSetting {
    Theme(DesktopTheme),
    Terminal(String),
    IdleTimeoutSeconds(u32),
    IdleDim(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopSettingOperation {
    target: DesktopEnvironment,
    setting: DesktopSetting,
}

impl DesktopSettingOperation {
    pub fn new(target: DesktopEnvironment, setting: DesktopSetting) -> Result<Self> {
        if let DesktopSetting::Terminal(executable) = &setting {
            validate_executable(executable)?;
        }
        Ok(Self { target, setting })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        let target = match self.target {
            DesktopEnvironment::Gnome => "gnome",
            DesktopEnvironment::Cinnamon => "cinnamon",
        };
        let (name, value) = match &self.setting {
            DesktopSetting::Theme(DesktopTheme::Light) => ("theme", "light".into()),
            DesktopSetting::Theme(DesktopTheme::Dark) => ("theme", "dark".into()),
            DesktopSetting::Terminal(executable) => ("terminal", executable.clone()),
            DesktopSetting::IdleTimeoutSeconds(seconds) => {
                ("idle-timeout-seconds", seconds.to_string())
            }
            DesktopSetting::IdleDim(enabled) => ("idle-dim", enabled.to_string()),
        };
        vec!["desktop-setting".into(), target.into(), name.into(), value]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GnomeExtensionsOperation {
    extensions: Vec<String>,
}

impl GnomeExtensionsOperation {
    pub fn new(extensions: Vec<String>) -> Result<Self> {
        validate_extensions(&extensions)?;
        Ok(Self { extensions })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        std::iter::once("gnome-extensions".into())
            .chain(self.extensions.iter().cloned())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GnomeDockOperation;

impl GnomeDockOperation {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn display_args(self) -> Vec<String> {
        vec!["gnome-dock".into()]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GnomeRoundedCornersOperation;

impl GnomeRoundedCornersOperation {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn display_args(self) -> Vec<String> {
        vec!["gnome-rounded-corners".into()]
    }
}

pub(crate) fn desktop_setting(host: &Host<'_>, operation: &DesktopSettingOperation) -> Result<()> {
    let prefix = match operation.target {
        DesktopEnvironment::Gnome => "org.gnome",
        DesktopEnvironment::Cinnamon => "org.cinnamon",
    };
    match &operation.setting {
        DesktopSetting::Theme(theme) => ensure_gsetting(
            host,
            &format!("{prefix}.desktop.interface"),
            "color-scheme",
            match theme {
                DesktopTheme::Light => "'prefer-light'",
                DesktopTheme::Dark => "'prefer-dark'",
            },
        ),
        DesktopSetting::Terminal(executable) => {
            validate_executable(executable).context("validate desktop terminal operation")?;
            if !command_is_executable(host, executable) {
                bail!("desktop terminal executable {executable:?} is unavailable");
            }
            let schema = format!("{prefix}.desktop.default-applications.terminal");
            ensure_gsetting(host, &schema, "exec", &format!("'{executable}'"))?;
            ensure_gsetting(host, &schema, "exec-arg", "''")
        }
        DesktopSetting::IdleTimeoutSeconds(seconds) => ensure_gsetting(
            host,
            &format!("{prefix}.desktop.session"),
            "idle-delay",
            &format!("uint32 {seconds}"),
        ),
        DesktopSetting::IdleDim(enabled) => ensure_gsetting(
            host,
            &format!("{prefix}.settings-daemon.plugins.power"),
            "idle-dim",
            if *enabled { "true" } else { "false" },
        ),
    }
}

pub(crate) fn gnome_extensions(
    host: &Host<'_>,
    operation: &GnomeExtensionsOperation,
) -> Result<()> {
    validate_extensions(&operation.extensions).context("validate GNOME extensions operation")?;
    let mut installed = extension_state(host, false)?;
    let mut needs_login_boundary = false;
    for extension in &operation.extensions {
        if !installed.contains(extension) {
            install_extension(host, extension)?;
            installed = extension_state(host, false)?;
            if !installed.contains(extension) {
                needs_login_boundary = true;
                continue;
            }
        }
        let enabled = extension_state(host, true)?;
        if !enabled.contains(extension) {
            host.require(
                "GNOME extension enable",
                "gnome-extensions",
                ["enable", extension],
            )?;
        }
        if !extension_state(host, true)?.contains(extension) {
            bail!("GNOME extension {extension:?} was not enabled");
        }
    }
    if needs_login_boundary {
        eprintln!(
            "cozydot: GNOME registered newly installed extensions after a login boundary; log out and back in once, then rerun `cozydot apply`"
        );
    }
    Ok(())
}

pub(crate) fn gnome_dock(host: &Host<'_>, _: &GnomeDockOperation) -> Result<()> {
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
        ensure_dconf(
            host,
            &format!("/org/gnome/shell/extensions/dash-to-dock/{key}"),
            value,
        )?;
    }
    Ok(())
}

pub(crate) fn gnome_rounded_corners(
    host: &Host<'_>,
    _: &GnomeRoundedCornersOperation,
) -> Result<()> {
    let value = "{'padding': <{'left': uint32 1, 'right': 1, 'top': 1, 'bottom': 1}>, 'keepRoundedCorners': <{'maximized': false, 'fullscreen': false}>, 'borderRadius': <uint32 16>, 'smoothing': <0.5>, 'borderColor': <(0.5, 0.5, 0.5, 1.0)>, 'enabled': <true>}";
    ensure_dconf(
        host,
        "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings",
        value,
    )
}

fn ensure_gsetting(host: &Host<'_>, schema: &str, key: &str, expected: &str) -> Result<()> {
    let current = host.require("desktop setting query", "gsettings", ["get", schema, key])?;
    if state_line(&current.stdout, "gsettings")? != expected {
        host.require(
            "desktop setting mutation",
            "gsettings",
            ["set", schema, key, expected],
        )?;
    }
    let current = host.require(
        "desktop setting postcondition",
        "gsettings",
        ["get", schema, key],
    )?;
    if state_line(&current.stdout, "gsettings")? != expected {
        bail!("desktop setting {schema} {key} did not converge to {expected}");
    }
    Ok(())
}

fn ensure_dconf(host: &Host<'_>, key: &str, expected: &str) -> Result<()> {
    let current = host.require("GNOME dconf query", "dconf", ["read", key])?;
    if optional_state_line(&current.stdout, "dconf")? != Some(expected) {
        host.require("GNOME dconf mutation", "dconf", ["write", key, expected])?;
    }
    let current = host.require("GNOME dconf postcondition", "dconf", ["read", key])?;
    if optional_state_line(&current.stdout, "dconf")? != Some(expected) {
        bail!("GNOME dconf setting {key} did not converge to {expected}");
    }
    Ok(())
}

fn extension_state(host: &Host<'_>, enabled_only: bool) -> Result<BTreeSet<String>> {
    let mut command = vec!["list".to_owned()];
    if enabled_only {
        command.push("--enabled".into());
    }
    let output = host.require("GNOME extension state query", "gnome-extensions", command)?;
    let output =
        std::str::from_utf8(&output.stdout).context("gnome-extensions returned non-UTF-8 state")?;
    let mut extensions = BTreeSet::new();
    for extension in output.lines() {
        validate_extension(extension)
            .map_err(|_| anyhow::anyhow!("gnome-extensions returned malformed UUID state"))?;
        if !extensions.insert(extension.to_owned()) {
            bail!("gnome-extensions returned duplicate UUID state");
        }
    }
    Ok(extensions)
}

fn install_extension(host: &Host<'_>, extension: &str) -> Result<()> {
    let endpoint = format!("https://extensions.gnome.org/extension-info/?uuid={extension}");
    let metadata = host.require("GNOME extension metadata", "curl", ["-fsSL", &endpoint])?;
    let shell = host.require(
        "GNOME extension shell version",
        "gnome-shell",
        ["--version"],
    )?;
    let shell_version = json_helpers::gnome_shell_version(
        std::str::from_utf8(&shell.stdout).context("GNOME Shell version is not UTF-8")?,
    )?;
    let version = json_helpers::gnome_version(
        std::str::from_utf8(&metadata.stdout).context("GNOME extension metadata is not UTF-8")?,
        &shell_version,
    )?;
    let archive = TempPath::new_with_suffix(host, "gnome-extension", ".zip")?;
    let name = extension.replace('@', "");
    let url = format!(
        "https://extensions.gnome.org/extension-data/{name}.v{version}.shell-extension.zip"
    );
    host.require(
        "GNOME extension download",
        "curl",
        ["-fL", "-o", &archive.path().to_string_lossy(), &url],
    )?;
    host.require(
        "GNOME extension install",
        "gnome-extensions",
        ["install", "--force", &archive.path().to_string_lossy()],
    )?;
    Ok(())
}

fn command_is_executable(host: &Host<'_>, executable: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    host.value("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| {
            std::fs::metadata(directory.join(executable)).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
    })
}

fn validate_executable(value: &str) -> Result<()> {
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        bail!("invalid desktop terminal executable {value:?}");
    }
    Ok(())
}

fn validate_extensions(extensions: &[String]) -> Result<()> {
    if extensions.is_empty() {
        bail!("GNOME extension sequence must not be empty");
    }
    let mut seen = BTreeSet::new();
    for extension in extensions {
        validate_extension(extension)?;
        if !seen.insert(extension) {
            bail!("duplicate GNOME extension UUID {extension:?}");
        }
    }
    Ok(())
}

fn validate_extension(value: &str) -> Result<()> {
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
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn state_line<'a>(output: &'a [u8], command: &str) -> Result<&'a str> {
    optional_state_line(output, command)?.context(format!("{command} returned empty state"))
}

fn optional_state_line<'a>(output: &'a [u8], command: &str) -> Result<Option<&'a str>> {
    let output = std::str::from_utf8(output)
        .with_context(|| format!("{command} returned non-UTF-8 state"))?;
    let output = output.strip_suffix('\n').unwrap_or(output);
    if output.contains(['\n', '\r']) {
        bail!("{command} returned malformed multiline state");
    }
    Ok((!output.is_empty()).then_some(output))
}

pub fn gnome_extension(host: &Host<'_>, extension: &str) -> Result<()> {
    if !host.command_exists("gnome-extensions") {
        bail!("gnome extension: gnome-extensions is unavailable after dependency installation");
    }
    let output = host.require("gnome extension", "gnome-extensions", ["list"])?;
    if String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|installed| installed == extension)
    {
        host.require("gnome extension", "gnome-extensions", ["enable", extension])?;
        return Ok(());
    }
    let endpoint = format!("https://extensions.gnome.org/extension-info/?uuid={extension}");
    let metadata = host.require("gnome extension", "curl", ["-fsSL", &endpoint])?;
    let shell = host.require("gnome extension", "gnome-shell", ["--version"])?;
    let shell_version = json_helpers::gnome_shell_version(&String::from_utf8(shell.stdout)?)?;
    let version =
        json_helpers::gnome_version(&String::from_utf8(metadata.stdout)?, &shell_version)?;
    let archive = TempPath::new(host, "gnome-extension.zip")?;
    let name = extension.replace('@', "");
    let url = format!(
        "https://extensions.gnome.org/extension-data/{name}.v{version}.shell-extension.zip"
    );
    host.require(
        "gnome extension",
        "curl",
        ["-fL", "-o", &archive.path().to_string_lossy(), &url],
    )?;
    host.require(
        "gnome extension",
        "gnome-extensions",
        ["install", "--force", &archive.path().to_string_lossy()],
    )?;
    let installed = host.require("gnome extension", "gnome-extensions", ["list"])?;
    if !String::from_utf8_lossy(&installed.stdout)
        .lines()
        .any(|installed| installed == extension)
    {
        eprintln!(
            "cozydot: installed GNOME extension {extension}; after this apply completes, log out and back in once, then rerun `cozydot apply` to enable all newly installed extensions"
        );
        return Ok(());
    }
    host.require("gnome extension", "gnome-extensions", ["enable", extension])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_and_extension_inputs_are_validated_by_operations() {
        assert!(DesktopSettingOperation::new(
            DesktopEnvironment::Gnome,
            DesktopSetting::Terminal("wezterm".into())
        )
        .is_ok());
        assert!(DesktopSettingOperation::new(
            DesktopEnvironment::Gnome,
            DesktopSetting::Terminal("wezterm;touch".into())
        )
        .is_err());
        assert!(GnomeExtensionsOperation::new(vec!["blur-my-shell@aunetx".into()]).is_ok());
        assert!(GnomeExtensionsOperation::new(vec!["not-an-extension".into()]).is_err());
    }
}
