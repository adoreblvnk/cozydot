use crate::platform::{Architecture, Platform};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Deserializer, de};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigVersion;

impl<'de> Deserialize<'de> for ConfigVersion {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value == "1.0.0" {
            Ok(Self)
        } else {
            Err(de::Error::custom(format!(
                "unsupported configuration version {value:?}; only version \"1.0.0\" is supported"
            )))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: ConfigVersion,
    pub shared: SharedConfig,
    pub os: OsConfig,
}

impl Config {
    fn deserialize(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::deserialize_str(&text)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let load = || -> Result<Self> {
            let config = Self::deserialize(path)?;
            config.validate()?;
            Ok(config)
        };
        load().with_context(|| format!("validate {}", path.display()))
    }

    fn deserialize_str(text: &str) -> Result<Self> {
        let deserializer = yaml_serde::Deserializer::from_str(text);
        serde_path_to_error::deserialize(deserializer).map_err(|error| {
            let path = error.path().to_string();
            let path = if path == "." { "config" } else { path.as_str() };
            anyhow::anyhow!("{path}: {}", error.inner())
        })
    }

    pub fn validate_for_platform(&self, platform: &Platform) -> Result<()> {
        self.validate()?;
        if platform.is_macos() {
            if platform.architecture != Architecture::DarwinArm64 {
                bail!("unsupported macOS architecture; only Apple Silicon (arm64) is supported");
            }
            if self.macos().system.rosetta == Some(true) && platform.architecture != Architecture::DarwinArm64 {
                bail!("os.macos.system.rosetta: Rosetta requires Apple Silicon macOS");
            }
            return Ok(());
        }
        let identity = resolve_platform_identity(platform)?;
        let (distro, upstream) = (identity.distro, identity.upstream);
        let desktop = DesktopKind::from_platform(&platform.desktop)?;

        if let Some(require) = self.os.linux.system.require.as_ref() {
            if require.distros.as_ref().is_some_and(|allowed| !allowed.is_empty() && !allowed.contains(&distro)) {
                bail!("os.linux.system.require.distros: detected distribution {:?} is not allowed", platform.distro);
            }
            if require.desktops.as_ref().is_some_and(|allowed| !allowed.is_empty() && !allowed.contains(&desktop)) {
                bail!("os.linux.system.require.desktops: detected desktop {:?} is not allowed", platform.desktop);
            }
        }

        if let Some(apt) = self.os.linux.packages.apt.as_ref() {
            apt.validate_ownership(distro, upstream)?;
        }

        if let Some(configured) = &self.os.linux.desktop
            && !matches!(desktop, DesktopKind::Gnome | DesktopKind::Cinnamon)
        {
            if configured.has_neutral_intent() {
                bail!(
                    "os.linux.desktop: theme, terminal, and idle settings require GNOME or Cinnamon; detected {:?}",
                    platform.desktop
                );
            }
            if configured.gnome.as_ref().is_some_and(Gnome::has_intent) {
                bail!(
                    "os.linux.desktop.gnome: requires GNOME or Cinnamon so GNOME-only settings can be applied or skipped; detected {:?}",
                    platform.desktop
                );
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        self.os.linux.packages.validate()?;
        self.shared.fonts.validate()?;
        self.shared.dotfiles.validate("shared.dotfiles")?;
        self.os.linux.dotfiles.validate("os.linux.dotfiles")?;
        self.os.macos.dotfiles.validate("os.macos.dotfiles")?;
        if let Some(desktop) = &self.os.linux.desktop {
            desktop.validate()?;
        }
        if self.shared.packages.cargo.as_ref().is_some_and(|values| !values.is_empty())
            && self.shared.tools.rust.is_none()
        {
            bail!("shared.packages.cargo: requires shared.tools.rust");
        }
        if self.shared.packages.npm.as_ref().is_some_and(|values| !values.is_empty())
            && self.shared.tools.node.is_none()
        {
            bail!("shared.packages.npm: requires shared.tools.node");
        }
        if let Some(go) = self.shared.tools.go.as_deref() {
            let exact = go.split('.').count() == 3
                && go.split('.').all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
            if go != "latest" && !exact {
                bail!("shared.tools.go: expected `latest` or an exact version such as `1.24.6`");
            }
        }
        Ok(())
    }

    pub fn macos(&self) -> &MacOsConfig {
        &self.os.macos
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedConfig {
    pub tools: Tools,
    pub packages: SharedPackages,
    pub fonts: Fonts,
    pub dotfiles: Dotfiles,
    pub integrations: SharedIntegrations,
    pub updates: SharedUpdates,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedPackages {
    pub cargo: Option<Vec<String>>,
    pub npm: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedIntegrations {
    pub vscode: VsCodeIntegration,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedUpdates {
    pub tools: ToolUpdates,
    pub packages: PackageUpdates,
    pub fonts: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OsConfig {
    pub linux: LinuxConfig,
    pub macos: MacOsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxConfig {
    pub system: System,
    pub packages: Packages,
    pub dotfiles: Dotfiles,
    pub integrations: Integrations,
    pub desktop: Option<Desktop>,
    pub updates: Option<LinuxUpdates>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxUpdates {
    pub apt: Option<AptUpdate>,
    pub flatpak: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacOsConfig {
    pub system: MacSystem,
    pub homebrew: Homebrew,
    pub dotfiles: Dotfiles,
    pub desktop: MacDesktop,
    pub updates: MacUpdates,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacSystem {
    pub ensure_admin: Option<bool>,
    pub xcode: MacXcode,
    pub rosetta: Option<bool>,
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
    pub appearance: Option<Theme>,
    pub dock: Option<MacDock>,
    pub finder: Option<MacFinder>,
    pub keyboard: Option<MacKeyboard>,
    pub trackpad: Option<MacTrackpad>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Distro {
    Ubuntu,
    Linuxmint,
    Pop,
    Debian,
}

impl Distro {
    fn parse_platform(value: &str) -> Result<Self> {
        match value {
            "ubuntu" => Ok(Self::Ubuntu),
            "linuxmint" => Ok(Self::Linuxmint),
            "pop" => Ok(Self::Pop),
            "debian" => Ok(Self::Debian),
            _ => bail!("os.linux.system.require.distros: unsupported detected distribution {value:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Ubuntu,
    Debian,
}

#[derive(Debug, Clone, Copy)]
pub struct PlatformIdentity {
    pub distro: Distro,
    pub upstream: Family,
}

pub fn resolve_platform_identity(platform: &Platform) -> Result<PlatformIdentity> {
    let distro = Distro::parse_platform(&platform.distro)?;
    let upstream = match platform.upstream.as_str() {
        "ubuntu" => Family::Ubuntu,
        "debian" => Family::Debian,
        value => bail!("os.linux.system.require.distros: unsupported platform upstream family {value:?}"),
    };
    let valid = match distro {
        Distro::Ubuntu | Distro::Pop => upstream == Family::Ubuntu,
        Distro::Debian => upstream == Family::Debian,
        Distro::Linuxmint => true,
    };
    if !valid {
        bail!(
            "os.linux.system.require.distros: detected distribution {:?} is inconsistent with upstream family {:?}",
            platform.distro,
            platform.upstream
        );
    }
    Ok(PlatformIdentity { distro, upstream })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopKind {
    None,
    Gnome,
    Cinnamon,
}

impl DesktopKind {
    fn from_platform(value: &str) -> Result<Self> {
        match value {
            "none" => Ok(Self::None),
            "gnome" => Ok(Self::Gnome),
            "cinnamon" => Ok(Self::Cinnamon),
            _ => bail!("os.linux.system.require.desktops: unsupported detected desktop {value:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistroMapKey {
    Default,
    Ubuntu,
    Linuxmint,
    Pop,
    Debian,
}

impl DistroMapKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Ubuntu => "ubuntu",
            Self::Linuxmint => "linuxmint",
            Self::Pop => "pop",
            Self::Debian => "debian",
        }
    }

    fn from_distro(distro: Distro) -> Self {
        match distro {
            Distro::Ubuntu => Self::Ubuntu,
            Distro::Linuxmint => Self::Linuxmint,
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

pub fn select_distro_map<T>(
    map: &BTreeMap<DistroMapKey, T>,
    distro: Distro,
    upstream: Family,
) -> Option<(DistroMapKey, &T)> {
    let exact = DistroMapKey::from_distro(distro);
    let family = DistroMapKey::from_family(upstream);
    map.get(&exact)
        .map(|value| (exact, value))
        .or_else(|| map.get(&family).map(|value| (family, value)))
        .or_else(|| map.get(&DistroMapKey::Default).map(|value| (DistroMapKey::Default, value)))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub require: Option<PlatformRequirements>,
    pub ensure_admin: Option<bool>,
    pub apt: Option<SystemApt>,
    pub ubuntu: Option<UbuntuSystem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformRequirements {
    pub distros: Option<Vec<Distro>>,
    pub desktops: Option<Vec<DesktopKind>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemApt {
    pub unattended_upgrades: Option<EnabledDisabled>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnabledDisabled {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UbuntuSystem {
    pub snap: Option<EnabledDisabled>,
    #[serde(default)]
    pub codecs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    pub apt: Option<AptPackages>,
    pub flatpak: Option<Vec<String>>,
    pub binaries: Option<Vec<BinaryPackage>>,
}

impl Packages {
    pub fn validate(&self) -> Result<()> {
        if let Some(apt) = &self.apt {
            apt.validate()?;
        }
        if let Some(binaries) = &self.binaries {
            let mut names = HashSet::new();
            for (index, binary) in binaries.iter().enumerate() {
                binary.validate(index)?;
                if !names.insert(binary.name.as_str()) {
                    bail!("os.linux.packages.binaries[{index}].name: duplicate binary name {:?}", binary.name);
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
    pub repositories: Option<Vec<Repository>>,
}

impl AptPackages {
    fn validate(&self) -> Result<()> {
        if let Some(repositories) = &self.repositories {
            let mut names = HashSet::new();
            let mut key_paths = HashSet::new();
            for (index, repository) in repositories.iter().enumerate() {
                repository.validate(index)?;
                if !names.insert(repository.name.as_str()) {
                    bail!(
                        "os.linux.packages.apt.repositories[{index}].name: duplicate repository name {:?}",
                        repository.name
                    );
                }
                if !key_paths.insert(repository.key_path.as_str()) {
                    bail!(
                        "os.linux.packages.apt.repositories[{index}].key_path: destination {:?} collides with an earlier repository",
                        repository.key_path
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_ownership(&self, distro: Distro, upstream: Family) -> Result<()> {
        let Some(repositories) = self.repositories.as_ref() else {
            return Ok(());
        };
        let direct = self.install.as_deref().unwrap_or_default().iter().map(String::as_str).collect::<HashSet<_>>();
        let applicable = repositories
            .iter()
            .enumerate()
            .filter(|(_, repository)| select_distro_map(&repository.urls, distro, upstream).is_some())
            .collect::<Vec<_>>();
        let repository_packages = applicable
            .iter()
            .flat_map(|(_, repository)| repository.packages.iter().map(String::as_str))
            .collect::<HashSet<_>>();

        for (index, repository) in applicable {
            if let Some(package) = repository.packages.iter().find(|package| direct.contains(package.as_str())) {
                bail!(
                    "os.linux.packages.apt.repositories[{index}].packages: package {package:?} is also a direct APT package"
                );
            }
            let Some((distro_key, conflicts)) =
                repository.conflicts.as_ref().and_then(|conflicts| select_distro_map(conflicts, distro, upstream))
            else {
                continue;
            };
            if let Some(package) = conflicts.iter().find(|package| direct.contains(package.as_str())) {
                bail!(
                    "os.linux.packages.apt.repositories[{index}].conflicts.{}: package {package:?} is also a direct APT package",
                    distro_key.as_str()
                );
            }
            if let Some(package) = conflicts.iter().find(|package| repository_packages.contains(package.as_str())) {
                bail!(
                    "os.linux.packages.apt.repositories[{index}].conflicts.{}: package {package:?} is also an applicable repository package",
                    distro_key.as_str()
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub name: String,
    pub key: String,
    pub key_path: String,
    pub urls: BTreeMap<DistroMapKey, String>,
    pub suite: Option<String>,
    pub components: Option<Vec<String>>,
    pub path: Option<String>,
    pub conflicts: Option<BTreeMap<DistroMapKey, Vec<String>>>,
    #[serde(default)]
    pub packages: Vec<String>,
}

impl Repository {
    fn validate(&self, index: usize) -> Result<()> {
        let path = format!("os.linux.packages.apt.repositories[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        validate_non_empty_map(&self.urls, &format!("{path}.urls"))?;
        match (&self.suite, &self.components, &self.path) {
            (Some(_), Some(components), None) => {
                if components.is_empty() {
                    bail!("{path}.components: required with suite");
                }
            }
            (None, None, Some(_)) => {}
            _ => bail!("{path}: requires exactly suite with non-empty components, or path"),
        }
        Ok(())
    }
}

pub fn selected_repository_codename(key: DistroMapKey, platform: &Platform, distro: Distro) -> &str {
    if key == DistroMapKey::Default || key == DistroMapKey::from_distro(distro) {
        &platform.distro_codename
    } else {
        &platform.base_codename
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BinaryFormat {
    Deb,
    Appimage,
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
        let path = format!("os.linux.packages.binaries[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        self.source.validate(&format!("{path}.source"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase", deny_unknown_fields)]
pub enum BinarySource {
    Github { repository: String, assets: ArchitectureMap },
    Url { urls: Box<ArchitectureMap> },
}

impl BinarySource {
    fn validate(&self, path: &str) -> Result<()> {
        match self {
            Self::Github { assets, .. } => assets.validate(&format!("{path}.assets"), "selector"),
            Self::Url { urls } => urls.validate(&format!("{path}.urls"), "URL"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureMap {
    pub amd64: Option<String>,
    pub arm64: Option<String>,
    pub arm32: Option<String>,
}

impl ArchitectureMap {
    fn validate(&self, path: &str, value_kind: &str) -> Result<()> {
        if self.amd64.is_none() && self.arm64.is_none() && self.arm32.is_none() {
            bail!("{path}: must contain at least one canonical architecture {value_kind}");
        }
        Ok(())
    }

    pub fn get(&self, architecture: Architecture) -> Option<&str> {
        match architecture {
            Architecture::Amd64 => self.amd64.as_deref(),
            Architecture::Arm64 | Architecture::DarwinArm64 => self.arm64.as_deref(),
            Architecture::Arm32 => self.arm32.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    pub rust: Option<String>,
    pub go: Option<String>,
    pub node: Option<String>,
    pub python: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    pub nerd: Option<Vec<String>>,
}

impl Fonts {
    fn validate(&self) -> Result<()> {
        validate_string_list(self.nerd.as_deref(), "shared.fonts.nerd", validate_definition_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dotfiles {
    pub packages: Vec<String>,
}

impl Dotfiles {
    fn validate(&self, path: &str) -> Result<()> {
        validate_string_values(&self.packages, &format!("{path}.packages"), validate_dotfile_package)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Integrations {
    pub docker: Option<DockerIntegration>,
    pub virtualbox: Option<VirtualBoxIntegration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerIntegration {
    pub add_user_to_group: Option<bool>,
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
    pub add_user_to_group: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VsCodeIntegration {
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Desktop {
    pub theme: Option<Theme>,
    pub terminal: Option<String>,
    pub idle: Option<Idle>,
    pub gnome: Option<Gnome>,
}

impl Desktop {
    fn validate(&self) -> Result<()> {
        if let Some(terminal) = &self.terminal {
            validate_executable(terminal, "os.linux.desktop.terminal")?;
        }
        Ok(())
    }

    pub fn has_neutral_intent(&self) -> bool {
        self.theme.is_some()
            || self.terminal.is_some()
            || self.idle.as_ref().is_some_and(|idle| idle.timeout.is_some() || idle.dim.is_some())
    }

    pub fn has_intent(&self) -> bool {
        self.has_neutral_intent() || self.gnome.as_ref().is_some_and(Gnome::has_intent)
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
        u32::try_from(duration.as_secs())
            .map(Self)
            .map_err(|_| de::Error::custom("duration exceeds the supported uint32 seconds range"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gnome {
    pub extensions: Option<Vec<String>>,
    pub dock: Option<bool>,
    pub rounded_corners: Option<bool>,
}

impl Gnome {
    fn has_intent(&self) -> bool {
        self.extensions.as_ref().is_some_and(|extensions| !extensions.is_empty())
            || self.dock == Some(true)
            || self.rounded_corners == Some(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AptUpdate {
    Standard,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUpdates {
    pub rust: Option<bool>,
    pub go: Option<bool>,
    pub node: Option<bool>,
    pub python: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdates {
    pub cargo: Option<bool>,
    pub npm: Option<bool>,
}

fn validate_non_empty_map<K, V>(map: &BTreeMap<K, V>, path: &str) -> Result<()> {
    if map.is_empty() {
        bail!("{path}: must be a non-empty mapping");
    }
    Ok(())
}

fn validate_string_list(values: Option<&[String]>, path: &str, validator: fn(&str, &str) -> Result<()>) -> Result<()> {
    let Some(values) = values else {
        return Ok(());
    };
    validate_string_values(values, path, validator)
}

fn validate_string_values(values: &[String], path: &str, validator: fn(&str, &str) -> Result<()>) -> Result<()> {
    for (index, value) in values.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        validator(value, &item_path)?;
    }
    Ok(())
}

fn validate_definition_name(value: &str, path: &str) -> Result<()> {
    let re = Regex::new(r"^[a-zA-Z0-9](?:[a-zA-Z0-9._-]*[a-zA-Z0-9])?$").unwrap();
    if !re.is_match(value) {
        bail!(
            "{path}: invalid value {value:?}; must start and end with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', or '-'"
        );
    }
    Ok(())
}

fn validate_dotfile_package(value: &str, path: &str) -> Result<()> {
    if matches!(value, "." | "..") {
        bail!("{path}: must denote exactly one child directory, not {value:?}");
    }
    validate_definition_name(value, path)
}

fn validate_executable(value: &str, path: &str) -> Result<()> {
    let re = Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9._+-]*$").unwrap();
    if !re.is_match(value) {
        bail!(
            "{path}: invalid executable basename {value:?}; must start with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', '+', or '-'"
        );
    }
    Ok(())
}
