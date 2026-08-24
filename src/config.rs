//! Define & validate Cozydot config.

use crate::platform::{Arch, DesktopKind, Distro, Family, Platform, PlatformIdentity};
use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde::{Deserialize, Deserializer, de};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Version {
    #[serde(rename = "1.0.0")]
    V1_0_0,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: Version,
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
        // packages
        self.packages.linux.validate()?;

        // tools
        if let Some(go) = self.tools.go.as_deref() {
            let valid = Regex::new(r"^(latest|[0-9]+\.[0-9]+\.[0-9]+)$")?.is_match(go);
            ensure!(valid, "tools.go: expected `latest` or an exact version such as `1.24.6`");
        }
        if !self.tools.cargo.is_empty() && self.tools.rust.is_none() {
            bail!("tools.cargo: requires tools.rust");
        }
        if !self.tools.npm.is_empty() && self.tools.node.is_none() {
            bail!("tools.npm: requires tools.node");
        }

        // fonts
        validate_definition_names(&self.fonts.nerd, "fonts.nerd")?;

        // dotfiles
        self.dotfiles.validate()?;

        // desktop
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
        let has_gnome_intent = theme.is_some() || linux_desktop.is_some_and(LinuxDesktop::has_intent);
        if platform.desktop != DesktopKind::Gnome && has_gnome_intent {
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
    pub macos: MacosSystem,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebianSystem {
    #[serde(default)]
    pub sudo_group: bool,
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
pub struct MacosSystem {
    #[serde(default)]
    pub validate_sudo_access: bool,
    pub xcode: Xcode,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xcode {
    #[serde(default)]
    pub command_line_tools: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    pub linux: LinuxPackages,
    pub macos: MacosPackages,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxPackages {
    pub apt: Option<AptPackages>,
    #[serde(default)]
    pub flatpak: Vec<String>,
    #[serde(default)]
    pub binaries: Vec<BinaryPackage>,
}

impl LinuxPackages {
    pub fn validate(&self) -> Result<()> {
        if let Some(apt) = &self.apt {
            apt.validate()?;
        }
        for (index, binary) in self.binaries.iter().enumerate() {
            binary.validate(index)?;
            if self.binaries[..index].iter().any(|earlier| earlier.name == binary.name) {
                bail!("packages.linux.binaries[{index}].name: duplicate binary name {:?}", binary.name);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AptPackages {
    #[serde(default)]
    pub install: Vec<String>,
    #[serde(default)]
    pub repos: Vec<AptRepoConfig>,
}

impl AptPackages {
    fn validate(&self) -> Result<()> {
        for (index, repo) in self.repos.iter().enumerate() {
            repo.validate(index)?;
            if self.repos[..index].iter().any(|earlier| earlier.name == repo.name) {
                bail!("packages.linux.apt.repos[{index}].name: duplicate repo name {:?}", repo.name);
            }
            if self.repos[..index].iter().any(|earlier| earlier.key_path == repo.key_path) {
                bail!(
                    "packages.linux.apt.repos[{index}].key_path: destination {:?} collides with an earlier repo",
                    repo.key_path
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistroKey {
    Default,
    Ubuntu,
    LinuxMint,
    Pop,
    Debian,
}

impl DistroKey {
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

pub fn select_distro_uri(uris: &BTreeMap<DistroKey, String>, identity: PlatformIdentity) -> Option<(DistroKey, &str)> {
    let PlatformIdentity::Linux { distro, family } = identity else { return None };
    // prefer the exact distro, then its base family, then the default URI
    for key in [DistroKey::from_distro(distro), DistroKey::from_family(family), DistroKey::Default] {
        if let Some(uri) = uris.get(&key) {
            return Some((key, uri.as_str()));
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AptRepoConfig {
    pub name: String,
    pub key_url: String,
    pub key_path: String,
    pub uris: BTreeMap<DistroKey, String>,
    pub suite: String,
    pub components: Vec<String>,
    pub arch: Option<Vec<AptArch>>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub packages: Vec<String>,
}

impl AptRepoConfig {
    fn validate(&self, index: usize) -> Result<()> {
        let path = format!("packages.linux.apt.repos[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        ensure!(!self.uris.is_empty(), "{path}.uris: must be a non-empty mapping");
        ensure!(!self.key_url.chars().any(char::is_control), "{path}.key_url: must contain no control characters");
        // limit privileged writes to direct children of APT keyring directories
        let key_path = Path::new(&self.key_path);
        let parent = key_path.parent().context("APT repo key path has no parent")?;
        if parent != Path::new("/etc/apt/keyrings") && parent != Path::new("/usr/share/keyrings") {
            bail!("APT repo key path must be a direct child of /etc/apt/keyrings or /usr/share/keyrings");
        }
        let name = key_path.file_name().and_then(|name| name.to_str()).context("APT repo key path has no filename")?;
        if !Regex::new(r"^[A-Za-z0-9._-]+\.(asc|gpg)$")?.is_match(name) {
            bail!("APT repo key path must name a safe .asc or .gpg file");
        }
        ensure!(!self.suite.is_empty(), "{path}.suite: must not be empty");
        if self.components.is_empty() || self.components.iter().any(String::is_empty) {
            bail!("{path}.components: must contain only non-empty values");
        }
        if self.arch.as_ref().is_some_and(Vec::is_empty) {
            bail!("{path}.arch: must not be empty when present");
        }
        let contains_control = |value: &str| value.chars().any(char::is_control);
        let has_control = self.uris.values().any(|value| contains_control(value))
            || contains_control(&self.suite)
            || self.components.iter().any(|value| contains_control(value));
        if has_control {
            bail!("{path}: source values must fit on one line and contain no control characters");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AptArch {
    Amd64,
    Arm64,
}

pub fn select_repo_codename(key: DistroKey, platform: &Platform) -> &str {
    let exact = match platform.identity {
        PlatformIdentity::Linux { distro, .. } => key == DistroKey::from_distro(distro),
        PlatformIdentity::Macos => false,
    };
    // exact/default mappings track the host codename; family mappings track the base codename
    if key == DistroKey::Default || exact { &platform.distro_codename } else { &platform.base_codename }
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
    GitHub { repo: String, assets: ArchMap },
    Url { urls: ArchMap },
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
pub struct ArchMap {
    pub x86_64: Option<String>,
    pub aarch64: Option<String>,
}

impl ArchMap {
    fn validate(&self, path: &str, kind: &str) -> Result<()> {
        if self.x86_64.is_none() && self.aarch64.is_none() {
            bail!("{path}: must contain at least one canonical architecture {kind}");
        }
        Ok(())
    }

    pub fn get(&self, arch: Arch) -> Option<&str> {
        match arch {
            Arch::X86_64 => self.x86_64.as_deref(),
            Arch::Aarch64 => self.aarch64.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacosPackages {
    pub homebrew: Homebrew,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Homebrew {
    pub formulae: Vec<String>,
    pub casks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    pub rust: Option<String>,
    pub node: Option<String>,
    pub python: Option<String>,
    pub go: Option<String>,
    #[serde(default)]
    pub cargo: Vec<String>,
    #[serde(default)]
    pub npm: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    #[serde(default)]
    pub nerd: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dotfiles {
    #[serde(default)]
    pub replace: bool,
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
    pub vscode: VsCode,
    pub linux: LinuxIntegrations,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VsCode {
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxIntegrations {
    pub docker: Option<Docker>,
    pub virtualbox: Option<VirtualBox>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Docker {
    #[serde(default)]
    pub group: bool,
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
pub struct VirtualBox {
    #[serde(default)]
    pub group: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Desktop {
    pub theme: Option<Theme>,
    pub linux: Option<LinuxDesktop>,
    pub macos: Option<MacosDesktop>,
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
            let valid = Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._+-]*$")?.is_match(terminal);
            if !valid {
                let path = "desktop.linux.gnome.terminal";
                bail!("{path}: {terminal:?} must start alphanumeric and contain only alphanumerics or `._+-`");
            }
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
    pub timeout: Option<IdleDuration>,
    pub dim: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleDuration(u32);

impl IdleDuration {
    pub fn seconds(self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for IdleDuration {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let duration = humantime::parse_duration(&value).map_err(de::Error::custom)?;
        // GNOME stores idle-delay as uint32 seconds
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
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub dash_to_dock: bool,
    #[serde(default)]
    pub rounded_window_corners: bool,
}

impl Gnome {
    pub(crate) fn has_intent(&self) -> bool {
        self.terminal.is_some()
            || self.idle.as_ref().is_some_and(|idle| idle.timeout.is_some() || idle.dim.is_some())
            || !self.extensions.is_empty()
            || self.dash_to_dock
            || self.rounded_window_corners
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacosDesktop {
    pub dock: Option<Dock>,
    pub finder: Option<Finder>,
    pub keyboard: Option<Keyboard>,
    pub trackpad: Option<Trackpad>,
}

impl MacosDesktop {
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
pub struct Dock {
    pub autohide: Option<bool>,
    pub show_recent_applications: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finder {
    pub show_filename_extensions: Option<bool>,
    pub show_hidden_files: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyboard {
    pub key_repeat: Option<i32>,
    pub initial_key_repeat: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trackpad {
    pub tap_to_click: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Updates {
    pub packages: PackageUpdates,
    pub tools: ToolUpdates,
    #[serde(default)]
    pub fonts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdates {
    pub linux: LinuxUpdates,
    pub macos: MacosUpdates,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxUpdates {
    pub apt: Option<AptUpgrade>,
    #[serde(default)]
    pub flatpak: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AptUpgrade {
    Upgrade,
    FullUpgrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacosUpdates {
    pub homebrew: HomebrewUpdates,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomebrewUpdates {
    #[serde(default)]
    pub formulae: bool,
    #[serde(default)]
    pub casks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUpdates {
    #[serde(default)]
    pub rust: bool,
    #[serde(default)]
    pub node: bool,
    #[serde(default)]
    pub python: bool,
    #[serde(default)]
    pub go: bool,
    #[serde(default)]
    pub cargo: bool,
    #[serde(default)]
    pub npm: bool,
}

fn validate_definition_names(values: &[String], path: &str) -> Result<()> {
    for (index, value) in values.iter().enumerate() {
        validate_definition_name(value, &format!("{path}[{index}]"))?;
    }
    Ok(())
}

fn validate_definition_name(value: &str, path: &str) -> Result<()> {
    let valid = Regex::new(r"^[A-Za-z0-9]([A-Za-z0-9._-]*[A-Za-z0-9])?$")?.is_match(value);
    if !valid {
        bail!("{path}: {value:?} must start/end alphanumeric and contain only alphanumerics or `._-`");
    }
    Ok(())
}
