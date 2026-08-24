//! Define & validate Cozydot config.

use crate::platform::{Architecture, DesktopKind, Distro, Family, Platform, PlatformIdentity};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, de};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ConfigVersion {
    #[serde(rename = "1.0.0")]
    V1_0_0,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: ConfigVersion,
    pub system: System,
    pub packages: Packages,
    pub tools: Tools,
    pub fonts: Fonts,
    pub dotfiles: Dotfiles,
    pub integrations: Integrations,
    pub desktop: Option<Desktop>,
    pub updates: Updates,
}

impl Config {
    /// Load & validate config at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let config: Self = yaml_serde::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        config.validate().with_context(|| format!("validate {}", path.display()))?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.tools.cargo.as_ref().is_some_and(|values| !values.is_empty()) && self.tools.rust.is_none() {
            bail!("tools.cargo: requires tools.rust");
        }
        if self.tools.npm.as_ref().is_some_and(|values| !values.is_empty()) && self.tools.node.is_none() {
            bail!("tools.npm: requires tools.node");
        }
        if let Some(go) = self.tools.go.as_deref() {
            let exact = go.split('.').count() == 3
                && go.split('.').all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
            if go != "latest" && !exact {
                bail!("tools.go: expected `latest` or an exact version such as `1.24.6`");
            }
        }
        self.fonts.validate()?;
        self.dotfiles.validate()?;
        self.packages.linux.validate()?;
        if let Some(linux) = self.desktop.as_ref().and_then(|desktop| desktop.linux.as_ref()) {
            linux.validate()?;
        }
        Ok(())
    }

    /// Validate config intent that depends on the detected `platform`.
    pub fn validate_for_platform(&self, platform: &Platform) -> Result<()> {
        let PlatformIdentity::Linux { .. } = platform.identity else { return Ok(()) };

        let theme = self.desktop.as_ref().and_then(|desktop| desktop.theme);
        let linux_desktop = self.desktop.as_ref().and_then(|desktop| desktop.linux.as_ref());
        if platform.desktop != DesktopKind::Gnome
            && (theme.is_some() || linux_desktop.is_some_and(LinuxDesktop::has_intent))
        {
            bail!(
                "desktop.theme and desktop.linux.gnome settings require GNOME; detected {:?}",
                platform.desktop.as_str()
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub debian: Option<DebianSystem>,
    pub ubuntu: Option<UbuntuSystem>,
    pub macos: MacSystem,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    pub linux: LinuxPackages,
    pub macos: MacPackages,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    pub rust: Option<String>,
    pub node: Option<String>,
    pub python: Option<String>,
    pub go: Option<String>,
    pub cargo: Option<Vec<String>>,
    pub npm: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    pub nerd: Option<Vec<String>>,
}

impl Fonts {
    fn validate(&self) -> Result<()> {
        let Some(families) = self.nerd.as_deref() else { return Ok(()) };
        validate_definition_names(families, "fonts.nerd")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dotfiles {
    pub replace: Option<bool>,
    pub packages: DotfilePackages,
}

impl Dotfiles {
    fn validate(&self) -> Result<()> {
        validate_definition_names(&self.packages.all, "dotfiles.packages.all")?;
        validate_definition_names(&self.packages.linux, "dotfiles.packages.linux")?;
        validate_definition_names(&self.packages.macos, "dotfiles.packages.macos")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DotfilePackages {
    pub all: Vec<String>,
    pub linux: Vec<String>,
    pub macos: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Integrations {
    pub vscode: VsCodeIntegration,
    pub linux: LinuxIntegrations,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VsCodeIntegration {
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Desktop {
    pub theme: Option<Theme>,
    pub linux: Option<LinuxDesktop>,
    pub macos: Option<MacDesktop>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Updates {
    pub packages: PackageUpdates,
    pub tools: ToolUpdates,
    pub fonts: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUpdates {
    pub rust: Option<bool>,
    pub node: Option<bool>,
    pub python: Option<bool>,
    pub go: Option<bool>,
    pub cargo: Option<bool>,
    pub npm: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdates {
    pub linux: LinuxUpdates,
    pub macos: MacUpdates,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebianSystem {
    pub sudo_group: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Enablement {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UbuntuSystem {
    pub unattended_upgrades: Option<Enablement>,
    pub snapd: Option<Enablement>,
    #[serde(default)]
    pub restricted_extras: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxPackages {
    pub apt: Option<AptPackages>,
    pub flatpak: Option<Vec<String>>,
    pub binaries: Option<Vec<BinaryPackage>>,
}

impl LinuxPackages {
    pub fn validate(&self) -> Result<()> {
        if let Some(apt) = &self.apt {
            apt.validate()?;
        }
        if let Some(binaries) = &self.binaries {
            let mut names = HashSet::new();
            for (index, binary) in binaries.iter().enumerate() {
                binary.validate(index)?;
                if !names.insert(binary.name.as_str()) {
                    bail!("packages.linux.binaries[{index}].name: duplicate binary name {:?}", binary.name);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AptPackages {
    pub install: Option<Vec<String>>,
    pub repos: Option<Vec<AptRepoConfig>>,
}

impl AptPackages {
    fn validate(&self) -> Result<()> {
        if let Some(repos) = &self.repos {
            let mut names = HashSet::new();
            let mut key_paths = HashSet::new();
            for (index, repo) in repos.iter().enumerate() {
                repo.validate(index)?;
                if !names.insert(repo.name.as_str()) {
                    bail!("packages.linux.apt.repos[{index}].name: duplicate repo name {:?}", repo.name);
                }
                if !key_paths.insert(repo.key_path.as_str()) {
                    bail!(
                        "packages.linux.apt.repos[{index}].key_path: destination {:?} collides with an earlier repo",
                        repo.key_path
                    );
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistroMapKey {
    Default,
    Ubuntu,
    LinuxMint,
    Pop,
    Debian,
}

impl DistroMapKey {
    fn from_distro(distro: Distro) -> Self {
        match distro {
            Distro::Ubuntu => Self::Ubuntu,
            Distro::LinuxMint => Self::LinuxMint,
            Distro::Pop => Self::Pop,
            Distro::Debian => Self::Debian,
        }
    }

    fn from_family(family: Family) -> Self {
        match family {
            Family::Ubuntu => Self::Ubuntu,
            Family::Debian => Self::Debian,
        }
    }
}

pub fn select_distro_entry<T>(
    map: &BTreeMap<DistroMapKey, T>,
    identity: PlatformIdentity,
) -> Option<(DistroMapKey, &T)> {
    let PlatformIdentity::Linux { distro, family } = identity else { return None };
    let exact_key = DistroMapKey::from_distro(distro);
    let family_key = DistroMapKey::from_family(family);
    [exact_key, family_key, DistroMapKey::Default].into_iter().find_map(|key| map.get(&key).map(|value| (key, value)))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AptRepoConfig {
    pub name: String,
    pub key_url: String,
    pub key_path: String,
    pub uris: BTreeMap<DistroMapKey, String>,
    pub suite: String,
    pub components: Vec<String>,
    pub arch: Option<Vec<AptArchitecture>>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub packages: Vec<String>,
}

impl AptRepoConfig {
    fn validate(&self, index: usize) -> Result<()> {
        let path = format!("packages.linux.apt.repos[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        if self.uris.is_empty() {
            bail!("{path}.uris: must be a non-empty mapping");
        }
        if self.key_url.chars().any(char::is_control) {
            bail!("{path}.key_url: must contain no control characters");
        }
        validate_repo_key_path(Path::new(&self.key_path))?;
        if self.suite.is_empty() {
            bail!("{path}.suite: must not be empty");
        }
        if self.components.is_empty() || self.components.iter().any(String::is_empty) {
            bail!("{path}.components: must contain only non-empty values");
        }
        if self.arch.as_ref().is_some_and(Vec::is_empty) {
            bail!("{path}.arch: must not be empty when present");
        }
        let has_control = |value: &str| value.chars().any(char::is_control);
        if self.uris.values().any(|value| has_control(value))
            || has_control(&self.suite)
            || self.components.iter().any(|value| has_control(value))
        {
            bail!("{path}: source values must fit on one line and contain no control characters");
        }
        Ok(())
    }
}

pub(crate) fn validate_repo_key_path(path: &Path) -> Result<()> {
    let parent = path.parent().context("APT repo key path has no parent")?;
    if parent != Path::new("/etc/apt/keyrings") && parent != Path::new("/usr/share/keyrings") {
        bail!("APT repo key path must be a direct child of /etc/apt/keyrings or /usr/share/keyrings");
    }
    let name = path.file_name().and_then(|name| name.to_str()).context("APT repo key path has no filename")?;
    if !matches!(path.extension().and_then(|extension| extension.to_str()), Some("asc" | "gpg"))
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        bail!("APT repo key path must name a safe .asc or .gpg file");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AptArchitecture {
    Amd64,
    Arm64,
}

pub fn select_repo_codename(key: DistroMapKey, platform: &Platform) -> &str {
    let exact = match platform.identity {
        PlatformIdentity::Linux { distro, .. } => key == DistroMapKey::from_distro(distro),
        PlatformIdentity::Macos => false,
    };
    if key == DistroMapKey::Default || exact { &platform.distro_codename } else { &platform.base_codename }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BinaryFormat {
    Deb,
    AppImage,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryPackage {
    pub name: String,
    pub format: BinaryFormat,
    pub source: BinarySource,
}

impl BinaryPackage {
    fn validate(&self, index: usize) -> Result<()> {
        let path = format!("packages.linux.binaries[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        self.source.validate(&format!("{path}.source"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase", deny_unknown_fields)]
pub enum BinarySource {
    GitHub { repo: String, assets: ArchitectureMap },
    Url { urls: ArchitectureMap },
}

impl BinarySource {
    fn validate(&self, path: &str) -> Result<()> {
        match self {
            Self::GitHub { assets, .. } => assets.validate(&format!("{path}.assets"), "asset pattern"),
            Self::Url { urls } => urls.validate(&format!("{path}.urls"), "URL"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureMap {
    pub x86_64: Option<String>,
    pub aarch64: Option<String>,
}

impl ArchitectureMap {
    fn validate(&self, path: &str, kind: &str) -> Result<()> {
        if self.x86_64.is_none() && self.aarch64.is_none() {
            bail!("{path}: must contain at least one canonical architecture {kind}");
        }
        Ok(())
    }

    pub fn get(&self, architecture: Architecture) -> Option<&str> {
        match architecture {
            Architecture::X86_64 => self.x86_64.as_deref(),
            Architecture::Aarch64 => self.aarch64.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxIntegrations {
    pub docker: Option<DockerIntegration>,
    pub virtualbox: Option<VirtualBoxIntegration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerIntegration {
    pub group: Option<bool>,
    pub logging: Option<DockerLogging>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockerLoggingDriver {
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerLogging {
    pub driver: DockerLoggingDriver,
    pub max_size: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualBoxIntegration {
    pub group: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxDesktop {
    pub gnome: Option<Gnome>,
}

impl LinuxDesktop {
    fn validate(&self) -> Result<()> {
        if let Some(terminal) = self.gnome.as_ref().and_then(|gnome| gnome.terminal.as_ref()) {
            validate_executable(terminal, "desktop.linux.gnome.terminal")?;
        }
        Ok(())
    }

    pub fn has_intent(&self) -> bool {
        self.gnome.as_ref().is_some_and(Gnome::has_intent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Idle {
    pub timeout: Option<DesktopIdleDuration>,
    pub dim: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopIdleDuration(u32);

impl DesktopIdleDuration {
    pub fn seconds(self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for DesktopIdleDuration {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let duration = humantime::parse_duration(&value).map_err(de::Error::custom)?;
        if duration.subsec_nanos() != 0 {
            return Err(de::Error::custom("duration must resolve to a whole number of seconds"));
        }
        let seconds = u32::try_from(duration.as_secs());
        let seconds = seconds.map_err(|_| de::Error::custom("duration exceeds the supported uint32 seconds range"))?;
        Ok(Self(seconds))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gnome {
    pub terminal: Option<String>,
    pub idle: Option<Idle>,
    pub extensions: Option<Vec<String>>,
    pub dash_to_dock: Option<bool>,
    pub rounded_window_corners: Option<bool>,
}

impl Gnome {
    pub(crate) fn has_intent(&self) -> bool {
        self.terminal.is_some()
            || self.idle.as_ref().is_some_and(|idle| idle.timeout.is_some() || idle.dim.is_some())
            || self.extensions.as_ref().is_some_and(|extensions| !extensions.is_empty())
            || self.dash_to_dock == Some(true)
            || self.rounded_window_corners == Some(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxUpdates {
    pub apt: Option<AptUpgradeCommand>,
    pub flatpak: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AptUpgradeCommand {
    Upgrade,
    FullUpgrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacPackages {
    pub homebrew: Homebrew,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacSystem {
    pub validate_sudo_access: Option<bool>,
    pub xcode: MacXcode,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacXcode {
    pub command_line_tools: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Homebrew {
    pub formulae: Vec<String>,
    pub casks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacDesktop {
    pub dock: Option<MacDock>,
    pub finder: Option<MacFinder>,
    pub keyboard: Option<MacKeyboard>,
    pub trackpad: Option<MacTrackpad>,
}

impl MacDesktop {
    pub(crate) fn has_intent(&self) -> bool {
        let dock = self.dock.as_ref().is_some_and(|d| d.autohide.is_some() || d.show_recent_applications.is_some());
        let finder = self.finder.as_ref();
        let finder = finder.is_some_and(|f| f.show_filename_extensions.is_some() || f.show_hidden_files.is_some());
        let keyboard = self.keyboard.as_ref();
        let keyboard = keyboard.is_some_and(|k| k.key_repeat.is_some() || k.initial_key_repeat.is_some());
        let trackpad = self.trackpad.as_ref().is_some_and(|trackpad| trackpad.tap_to_click.is_some());
        dock || finder || keyboard || trackpad
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacDock {
    pub autohide: Option<bool>,
    pub show_recent_applications: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacFinder {
    pub show_filename_extensions: Option<bool>,
    pub show_hidden_files: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacKeyboard {
    pub key_repeat: Option<i32>,
    pub initial_key_repeat: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacTrackpad {
    pub tap_to_click: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacUpdates {
    pub homebrew: MacHomebrewUpdates,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacHomebrewUpdates {
    pub formulae: Option<bool>,
    pub casks: Option<bool>,
}

fn validate_definition_names(values: &[String], path: &str) -> Result<()> {
    for (index, value) in values.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        validate_definition_name(value, &item_path)?;
    }
    Ok(())
}

fn validate_definition_name(value: &str, path: &str) -> Result<()> {
    let valid = value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !valid {
        bail!(
            "{path}: invalid value {value:?}; must start and end with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', or '-'"
        );
    }
    Ok(())
}

fn validate_executable(value: &str, path: &str) -> Result<()> {
    let valid = value.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'));
    if !valid {
        bail!(
            "{path}: invalid executable basename {value:?}; must start with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', '+', or '-'"
        );
    }
    Ok(())
}
