//! Define & validate cozydot config.

use crate::platform::{Arch, DesktopKind, Distro, Family, Platform, PlatformIdentity};
use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Version {
    #[serde(rename = "1")]
    V1,
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
            let is_semver = semver::Version::parse(go).is_ok_and(|v| v.pre.is_empty() && v.build.is_empty());
            ensure!(go == "latest" || is_semver, "tools.go: expected `latest` or an exact version such as `1.24.6`");
        }
        ensure!(self.tools.cargo.is_empty() || self.tools.rust.is_some(), "tools.cargo: requires tools.rust");
        ensure!(self.tools.npm.is_empty() || self.tools.node.is_some(), "tools.npm: requires tools.node");

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

pub type ArchMap = BTreeMap<Arch, String>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase", deny_unknown_fields)]
pub enum BinarySource {
    GitHub { repo: String, assets: ArchMap },
    Url { urls: ArchMap },
}

impl BinarySource {
    fn validate(&self, path: &str) -> Result<()> {
        match self {
            Self::GitHub { assets, .. } => ensure!(
                !assets.is_empty(),
                "{path}.assets: must contain at least one canonical architecture asset pattern"
            ),
            Self::Url { urls } => {
                ensure!(!urls.is_empty(), "{path}.urls: must contain at least one canonical architecture URL")
            }
        }
        Ok(())
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
    /// `skills` CLI sources such as `owner/repo@skill`
    #[serde(default)]
    pub skills: Vec<String>,
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
            ensure!(
                valid,
                "desktop.linux.gnome.terminal: {terminal:?} must start alphanumeric and contain only alphanumerics or `._+-`"
            );
        }
        if let Some(timeout) = self.gnome.as_ref().and_then(|g| g.idle.as_ref()).and_then(|i| i.timeout.as_deref()) {
            let duration = humantime::parse_duration(timeout).context("desktop.linux.gnome.idle.timeout")?;
            ensure!(duration.subsec_nanos() == 0, "desktop.linux.gnome.idle.timeout: must resolve to whole seconds");
            ensure!(
                u32::try_from(duration.as_secs()).is_ok(),
                "desktop.linux.gnome.idle.timeout: exceeds uint32 range"
            );
        }
        Ok(())
    }

    pub fn has_intent(&self) -> bool {
        self.gnome.as_ref().is_some_and(Gnome::has_intent)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Idle {
    pub timeout: Option<String>,
    pub dim: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gnome {
    /// Window titlebar button layout
    pub button_layout: Option<String>,
    #[serde(default)]
    pub dash_to_dock: bool,
    #[serde(default)]
    pub extensions: Vec<String>,
    pub files: Option<GnomeFiles>,
    pub idle: Option<Idle>,
    pub keyboard: Option<GnomeKeyboard>,
    #[serde(default)]
    pub rounded_window_corners: bool,
    pub terminal: Option<String>,
}

impl Gnome {
    pub(crate) fn has_intent(&self) -> bool {
        self.button_layout.is_some()
            || self.dash_to_dock
            || !self.extensions.is_empty()
            || self.files.is_some()
            || self.idle.is_some()
            || self.keyboard.is_some()
            || self.rounded_window_corners
            || self.terminal.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GnomeFiles {
    /// Show hidden files in Files & GTK file dialogs
    pub show_hidden_files: Option<bool>,
    /// Keep directories on top when sorting files
    pub sort_folders_first: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GnomeKeyboard {
    /// Delay until repeat in milliseconds
    pub delay: Option<u32>,
    /// Key repeat interval in milliseconds
    pub repeat_interval: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacosDesktop {
    pub dialogs: Option<Dialogs>,
    pub dock: Option<Dock>,
    pub finder: Option<Finder>,
    pub keyboard: Option<Keyboard>,
    pub screenshots: Option<Screenshots>,
    pub trackpad: Option<Trackpad>,
}

impl MacosDesktop {
    pub(crate) fn has_intent(&self) -> bool {
        self.dialogs.as_ref().is_some_and(|d| d != &Dialogs::default())
            || self.dock.as_ref().is_some_and(|d| d != &Dock::default())
            || self.finder.as_ref().is_some_and(|f| f != &Finder::default())
            || self.keyboard.as_ref().is_some_and(|k| k != &Keyboard::default())
            || self.screenshots.as_ref().is_some_and(|s| s != &Screenshots::default())
            || self.trackpad.as_ref().is_some_and(|t| t != &Trackpad::default())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialogs {
    /// Expand save panel by default
    pub expand_save_panel: Option<bool>,
    /// Save new documents to iCloud by default rather than disk
    pub save_to_cloud: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dock {
    /// Automatically hide & show Dock
    pub autohide: Option<bool>,
    /// Automatically rearrange Spaces based on recent use
    pub mru_spaces: Option<bool>,
    /// Show recent applications in Dock
    pub show_recent_applications: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finder {
    /// Create .DS_Store files on network & USB volumes
    pub ds_store_on_external: Option<bool>,
    /// Default search scope: SCcf = current folder / SCev = this Mac / FXml = previous scope
    pub search_scope: Option<SearchScope>,
    /// Show all filename extensions
    pub show_filename_extensions: Option<bool>,
    /// Show hidden files in Finder
    pub show_hidden_files: Option<bool>,
    /// Show path bar at bottom of Finder windows
    pub show_path_bar: Option<bool>,
    /// Show status bar with item count & available space
    pub show_status_bar: Option<bool>,
    /// Keep folders on top when sorting by name
    pub sort_folders_first: Option<bool>,
    /// Warning prompt before changing file extension
    pub warn_on_extension_change: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum SearchScope {
    SCcf,
    SCev,
    FXml,
}

impl SearchScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SCcf => "SCcf",
            Self::SCev => "SCev",
            Self::FXml => "FXml",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyboard {
    /// Smart substitutions: quotes, dashes, period, caps & spelling
    pub auto_substitutions: Option<bool>,
    /// Keyboard navigation to move focus between controls with Tab
    pub keyboard_navigation: Option<bool>,
    /// Delay until repeat in ticks (1 tick = 15ms)
    pub initial_key_repeat: Option<i32>,
    /// Key repeat rate in ticks (1 tick = 15ms)
    pub key_repeat: Option<i32>,
    /// Press & hold for accent menu instead of key repeat
    pub press_and_hold: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screenshots {
    /// Directory where screenshots & screen recordings are saved
    pub location: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trackpad {
    /// Tap trackpad to click
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
    ensure!(valid, "{path}: {value:?} must start/end alphanumeric and contain only alphanumerics or `._-`");
    Ok(())
}
