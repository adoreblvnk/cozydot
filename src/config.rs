use crate::platform::{Architecture, Platform};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Deserializer, de};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
};
use url::{Host, Url};

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
    pub system: Option<System>,
    pub packages: Option<Packages>,
    pub tools: Option<Tools>,
    pub fonts: Option<Fonts>,
    pub dotfiles: Option<Dotfiles>,
    pub integrations: Option<Integrations>,
    pub desktop: Option<Desktop>,
    pub updates: Option<Updates>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        let deserializer = yaml_serde::Deserializer::from_str(text);
        let config: Self = serde_path_to_error::deserialize(deserializer).map_err(|error| {
            let path = error.path().to_string();
            let path = if path == "." { "config" } else { path.as_str() };
            anyhow::anyhow!("{path}: {}", error.inner())
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("validate {}", path.display()))
    }

    pub fn validate_for_platform(&self, platform: &Platform) -> Result<()> {
        self.validate()?;
        let identity = resolve_platform_identity(platform)?;
        let (distro, upstream) = (identity.distro, identity.upstream);
        let desktop = DesktopKind::from_platform(&platform.desktop)?;

        if let Some(require) = self.system.as_ref().and_then(|system| system.require.as_ref()) {
            if require.distros.as_ref().is_some_and(|allowed| !allowed.is_empty() && !allowed.contains(&distro)) {
                map_distro_error(platform, distro)?;
            }
            if require.desktops.as_ref().is_some_and(|allowed| !allowed.is_empty() && !allowed.contains(&desktop)) {
                bail!("system.require.desktops: detected desktop {:?} is not allowed", platform.desktop);
            }
        }

        if let Some(sources) =
            self.system.as_ref().and_then(|system| system.apt.as_ref()).and_then(|apt| apt.sources.as_ref())
        {
            sources.validate_for_platform(platform, distro, upstream)?;
        }

        for (index, repository) in self
            .packages
            .as_ref()
            .and_then(|packages| packages.apt.as_ref())
            .and_then(|apt| apt.repositories.as_ref())
            .into_iter()
            .flatten()
            .enumerate()
        {
            repository.validate_for_platform(index, platform, distro, upstream)?;
        }

        if let Some(configured) = &self.desktop
            && !matches!(desktop, DesktopKind::Gnome | DesktopKind::Cinnamon)
        {
            if configured.has_neutral_intent() {
                bail!(
                    "desktop: theme, terminal, and idle settings require GNOME or Cinnamon; detected {:?}",
                    platform.desktop
                );
            }
            if configured.gnome.as_ref().is_some_and(Gnome::has_intent) {
                bail!(
                    "desktop.gnome: requires GNOME or Cinnamon so GNOME-only settings can be applied or skipped; detected {:?}",
                    platform.desktop
                );
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if let Some(system) = &self.system {
            system.validate()?;
        }
        if let Some(packages) = &self.packages {
            packages.validate()?;
        }
        if let Some(fonts) = &self.fonts {
            fonts.validate()?;
        }
        if let Some(dotfiles) = &self.dotfiles {
            dotfiles.validate()?;
        }
        if let Some(size) = self
            .integrations
            .as_ref()
            .and_then(|integrations| integrations.docker.as_ref())
            .and_then(|docker| docker.logging.as_ref())
            .and_then(|logging| logging.max_size.as_deref())
        {
            validate_docker_size(size, "integrations.docker.logging.max_size")?;
        }
        if let Some(desktop) = &self.desktop {
            desktop.validate()?;
        }
        if let Some(updates) = &self.updates {
            updates.validate(self)?;
        }
        if self.packages.as_ref().and_then(|packages| packages.cargo.as_ref()).is_some_and(|values| !values.is_empty())
            && self.tools.as_ref().and_then(|tools| tools.rust.as_ref()).is_none()
        {
            bail!("packages.cargo: requires tools.rust");
        }
        if self.packages.as_ref().and_then(|packages| packages.npm.as_ref()).is_some_and(|values| !values.is_empty())
            && self.tools.as_ref().and_then(|tools| tools.node.as_ref()).is_none()
        {
            bail!("packages.npm: requires tools.node");
        }
        Ok(())
    }
}

fn map_distro_error(platform: &Platform, _distro: Distro) -> Result<()> {
    bail!("system.require.distros: detected distribution {:?} is not allowed", platform.distro);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Distro {
    Ubuntu,
    Linuxmint,
    Pop,
    Zorin,
    Deepin,
    Debian,
    Kali,
    Tails,
}

impl Distro {
    fn parse_platform(value: &str) -> Result<Self> {
        match value {
            "ubuntu" => Ok(Self::Ubuntu),
            "linuxmint" => Ok(Self::Linuxmint),
            "pop" => Ok(Self::Pop),
            "zorin" => Ok(Self::Zorin),
            "deepin" => Ok(Self::Deepin),
            "debian" => Ok(Self::Debian),
            "kali" => Ok(Self::Kali),
            "tails" => Ok(Self::Tails),
            _ => bail!("system.require.distros: unsupported detected distribution {value:?}"),
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
        value => bail!("system.require.distros: unsupported platform upstream family {value:?}"),
    };
    let valid = match distro {
        Distro::Ubuntu | Distro::Pop | Distro::Zorin => upstream == Family::Ubuntu,
        Distro::Debian | Distro::Kali | Distro::Tails | Distro::Deepin => upstream == Family::Debian,
        Distro::Linuxmint => true,
    };
    if !valid {
        bail!(
            "system.require.distros: detected distribution {:?} is inconsistent with upstream family {:?}",
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
            _ => bail!("system.require.desktops: unsupported detected desktop {value:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub require: Option<PlatformRequirements>,
    pub ensure_admin: Option<bool>,
    pub apt: Option<SystemApt>,
    pub ubuntu: Option<UbuntuSystem>,
}

impl System {
    fn validate(&self) -> Result<()> {
        if let Some(apt) = &self.apt {
            apt.validate()?;
        }
        Ok(())
    }
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
    pub sources: Option<OfficialSources>,
    pub unattended_upgrades: Option<EnabledDisabled>,
}

impl SystemApt {
    fn validate(&self) -> Result<()> {
        if let Some(sources) = &self.sources {
            sources.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceMode {
    Preserve,
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficialSources {
    pub mode: SourceMode,
    pub components: Option<BTreeMap<DistroMapKey, Vec<AptComponent>>>,
}

impl OfficialSources {
    fn validate(&self) -> Result<()> {
        match (&self.mode, &self.components) {
            (SourceMode::Managed, Some(components)) => {
                if components.is_empty() {
                    bail!("system.apt.sources.components: required when mode is managed");
                }
                for (key, values) in components {
                    if values.is_empty() {
                        bail!("system.apt.sources.components.{}: required when mode is managed", key.as_str());
                    }
                }
            }
            (SourceMode::Managed, None) => {
                bail!("system.apt.sources.components: required when mode is managed")
            }
            (SourceMode::Preserve, Some(_)) => {
                bail!("system.apt.sources.components: forbidden when mode is preserve")
            }
            (SourceMode::Preserve, None) => {}
        }
        Ok(())
    }

    pub fn validate_for_platform(&self, platform: &Platform, distro: Distro, upstream: Family) -> Result<()> {
        if self.mode == SourceMode::Preserve {
            return Ok(());
        }
        if !matches!(distro, Distro::Ubuntu | Distro::Debian | Distro::Kali) {
            bail!(
                "system.apt.sources.mode: managed is unsupported for distribution {:?}; use preserve",
                platform.distro
            );
        }
        let identity = PlatformIdentity { distro, upstream };
        self.resolve_managed(platform, identity)?;
        Ok(())
    }

    pub fn resolve_managed(
        &self,
        platform: &Platform,
        identity: PlatformIdentity,
    ) -> Result<Option<crate::platform::ManagedAptSources>> {
        if self.mode == SourceMode::Preserve {
            return Ok(None);
        }
        let components = self.components.as_ref().context("managed APT sources require components")?;
        let (_, selected) = select_distro_map(components, identity.distro, identity.upstream).ok_or_else(|| {
            anyhow::anyhow!(
                "system.apt.sources.components: no entry for distribution {:?}, upstream {:?}, or default",
                platform.distro,
                platform.upstream
            )
        })?;
        let names = selected.iter().map(AptComponent::as_str).collect::<Vec<_>>();
        platform.managed_apt_sources(&names).map(Some)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AptComponent {
    Main,
    Restricted,
    Universe,
    Multiverse,
    Contrib,
    NonFree,
    NonFreeFirmware,
}

impl AptComponent {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Restricted => "restricted",
            Self::Universe => "universe",
            Self::Multiverse => "multiverse",
            Self::Contrib => "contrib",
            Self::NonFree => "non-free",
            Self::NonFreeFirmware => "non-free-firmware",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnabledDisabled {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstalledState {
    Installed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UbuntuSystem {
    pub snap: Option<EnabledDisabled>,
    pub codecs: Option<InstalledState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    pub apt: Option<AptPackages>,
    pub flatpak: Option<Vec<String>>,
    pub cargo: Option<Vec<String>>,
    pub npm: Option<Vec<String>>,
    pub binaries: Option<Vec<BinaryPackage>>,
}

impl Packages {
    pub fn validate(&self) -> Result<()> {
        if let Some(apt) = &self.apt {
            apt.validate()?;
        }
        if let Some(binaries) = &self.binaries {
            let mut names = HashSet::new();
            let mut command_owners = BTreeMap::new();
            for (index, binary) in binaries.iter().enumerate() {
                binary.validate(index)?;
                if !names.insert(binary.name.as_str()) {
                    bail!("packages.binaries[{index}].name: duplicate binary name {:?}", binary.name);
                }
                for (command_index, command) in binary.commands.iter().enumerate() {
                    let command_path = format!("packages.binaries[{index}].commands[{command_index}]");
                    if let Some(owner_path) = command_owners.insert(command.as_str(), command_path.clone()) {
                        bail!("{command_path}: command {command:?} is already claimed by {owner_path}");
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AptPackages {
    pub remove: Option<Vec<String>>,
    pub install: Option<Vec<String>>,
    pub repositories: Option<Vec<Repository>>,
}

impl AptPackages {
    fn validate(&self) -> Result<()> {
        let mut installed = HashSet::new();
        for package in self.install.iter().flatten() {
            installed.insert(package.as_str());
        }
        if let Some(repositories) = &self.repositories {
            let mut names = HashSet::new();
            let mut key_paths = HashSet::new();
            for (index, repository) in repositories.iter().enumerate() {
                repository.validate(index)?;
                if !names.insert(repository.name.as_str()) {
                    bail!("packages.apt.repositories[{index}].name: duplicate repository name {:?}", repository.name);
                }
                if !key_paths.insert(repository.key_path.as_str()) {
                    bail!(
                        "packages.apt.repositories[{index}].key_path: destination {:?} collides with an earlier repository",
                        repository.key_path
                    );
                }
                installed.extend(repository.packages.iter().map(String::as_str));
            }
        }
        for (index, package) in self.remove.iter().flatten().enumerate() {
            if installed.contains(package.as_str()) {
                bail!("packages.apt.remove[{index}]: package {package:?} is also configured for installation");
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
    Linuxmint,
    Pop,
    Zorin,
    Deepin,
    Debian,
    Kali,
    Tails,
}

impl DistroMapKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Ubuntu => "ubuntu",
            Self::Linuxmint => "linuxmint",
            Self::Pop => "pop",
            Self::Zorin => "zorin",
            Self::Deepin => "deepin",
            Self::Debian => "debian",
            Self::Kali => "kali",
            Self::Tails => "tails",
        }
    }

    fn from_distro(distro: Distro) -> Self {
        match distro {
            Distro::Ubuntu => Self::Ubuntu,
            Distro::Linuxmint => Self::Linuxmint,
            Distro::Pop => Self::Pop,
            Distro::Zorin => Self::Zorin,
            Distro::Deepin => Self::Deepin,
            Distro::Debian => Self::Debian,
            Distro::Kali => Self::Kali,
            Distro::Tails => Self::Tails,
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
pub struct Repository {
    pub name: String,
    pub key: HttpsUrl,
    pub key_path: String,
    pub urls: BTreeMap<DistroMapKey, HttpsUrl>,
    pub suite: Option<String>,
    pub components: Option<Vec<AptToken>>,
    pub path: Option<String>,
    pub packages: Vec<String>,
}

impl Repository {
    fn validate(&self, index: usize) -> Result<()> {
        let path = format!("packages.apt.repositories[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        validate_non_empty_map(&self.urls, &format!("{path}.urls"))?;
        validate_key_path(&self.key_path, &format!("{path}.key_path"))?;
        match (&self.suite, &self.components, &self.path) {
            (Some(suite), Some(components), None) => {
                validate_suite(suite, &format!("{path}.suite"))?;
                if components.is_empty() {
                    bail!("{path}.components: required with suite");
                }
                for (component_index, component) in components.iter().enumerate() {
                    let component_path = format!("{path}.components[{component_index}]");
                    validate_apt_token(component.as_str(), &component_path)?;
                    if component.as_str() == "system" {
                        bail!("{component_path}: value {:?} is reserved for the suite field", component.as_str());
                    }
                }
            }
            (None, None, Some(exact_path)) => validate_repository_path(exact_path, &format!("{path}.path"))?,
            _ => bail!("{path}: requires exactly suite with non-empty components, or path"),
        }
        Ok(())
    }

    pub fn validate_for_platform(
        &self,
        index: usize,
        platform: &Platform,
        distro: Distro,
        upstream: Family,
    ) -> Result<()> {
        let identity = PlatformIdentity { distro, upstream };
        let resolved = self.resolve_for_platform(index, platform, identity)?;
        if self.suite.as_deref() == Some("system") {
            validate_apt_token(
                resolved.suite.as_ref().context("system repository suite did not resolve")?.as_str(),
                &format!("packages.apt.repositories[{index}].suite resolved codename"),
            )?;
        }
        Ok(())
    }

    pub fn resolve_for_platform(
        &self,
        index: usize,
        platform: &Platform,
        identity: PlatformIdentity,
    ) -> Result<ResolvedRepository<'_>> {
        let (key, source_url) = select_distro_map(&self.urls, identity.distro, identity.upstream).ok_or_else(|| {
            anyhow::anyhow!(
                "packages.apt.repositories[{index}].urls: no URL for distribution {:?}, upstream {:?}, or default",
                platform.distro,
                platform.upstream
            )
        })?;
        let suite = match self.suite.as_deref() {
            Some("system") => Some(AptToken(selected_repository_codename(key, platform, identity.distro).to_owned())),
            Some(value) => Some(AptToken(value.to_owned())),
            None => None,
        };
        Ok(ResolvedRepository { source_url, suite })
    }
}

pub struct ResolvedRepository<'a> {
    pub source_url: &'a HttpsUrl,
    pub suite: Option<AptToken>,
}

fn selected_repository_codename(key: DistroMapKey, platform: &Platform, distro: Distro) -> &str {
    if key == DistroMapKey::Default || key == DistroMapKey::from_distro(distro) {
        &platform.distro_codename
    } else {
        &platform.base_codename
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AptToken(String);

impl AptToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for AptToken {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self(value))
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
    pub commands: Vec<String>,
    pub source: BinarySource,
}

impl BinaryPackage {
    fn validate(&self, index: usize) -> Result<()> {
        let path = format!("packages.binaries[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        validate_string_values(&self.commands, &format!("{path}.commands"), validate_executable)?;
        self.source.validate(&format!("{path}.source"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase", deny_unknown_fields)]
pub enum BinarySource {
    Github { repository: String, assets: AssetMap },
    Url { urls: Box<ArchitectureUrls>, sha256: Box<ArchitectureHashes> },
}

impl BinarySource {
    fn validate(&self, path: &str) -> Result<()> {
        match self {
            Self::Github { repository, assets } => {
                validate_github_repository(repository, &format!("{path}.repository"))?;
                assets.validate(&format!("{path}.assets"))
            }
            Self::Url { urls, sha256 } => {
                urls.validate(&format!("{path}.urls"))?;
                sha256.validate(&format!("{path}.sha256"))?;
                if urls.keys() != sha256.keys() {
                    bail!("{path}: urls and sha256 must contain exactly the same architecture keys");
                }
                Ok(())
            }
        }
    }

    pub fn is_github(&self) -> bool {
        matches!(self, Self::Github { .. })
    }

    pub fn resolve_native(&self, architecture: Architecture) -> Option<ResolvedNativeBinary<'_>> {
        match self {
            Self::Github { repository, assets } => {
                assets.get(architecture).map(|selector| ResolvedNativeBinary::Github { repository, selector })
            }
            Self::Url { urls, sha256 } => urls
                .get(architecture)
                .zip(sha256.get(architecture))
                .map(|(url, sha256)| ResolvedNativeBinary::Url { url, sha256 }),
        }
    }
}

pub enum ResolvedNativeBinary<'a> {
    Github { repository: &'a str, selector: &'a str },
    Url { url: &'a HttpsUrl, sha256: &'a Sha256 },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetMap {
    pub amd64: Option<String>,
    pub arm64: Option<String>,
    pub arm32: Option<String>,
}

impl AssetMap {
    fn validate(&self, path: &str) -> Result<()> {
        if self.values().iter().all(|(_, value)| value.is_none()) {
            bail!("{path}: must contain at least one canonical architecture selector");
        }
        for (architecture, selector) in self.values() {
            if let Some(selector) = selector {
                validate_asset_regex(selector, &format!("{path}.{architecture}"))?;
            }
        }
        Ok(())
    }

    fn values(&self) -> [(&'static str, Option<&String>); 3] {
        [("amd64", self.amd64.as_ref()), ("arm64", self.arm64.as_ref()), ("arm32", self.arm32.as_ref())]
    }

    fn get(&self, architecture: Architecture) -> Option<&str> {
        match architecture {
            Architecture::Amd64 => self.amd64.as_deref(),
            Architecture::Arm64 => self.arm64.as_deref(),
            Architecture::Arm32 => self.arm32.as_deref(),
        }
    }
}

fn validate_asset_regex(value: &str, path: &str) -> Result<()> {
    if value.is_empty() || !value.starts_with('^') || !value.ends_with('$') {
        bail!("{path}: asset regex must be non-empty and anchored with '^' and '$'");
    }
    Regex::new(value).with_context(|| format!("{path}: invalid asset regex {value:?}"))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256(String);

impl Sha256 {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self, path: &str) -> Result<()> {
        let value = &self.0;
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            bail!("{path}: invalid SHA-256 {value:?}; must be exactly 64 lowercase hexadecimal characters");
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Sha256 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureUrls {
    pub amd64: Option<HttpsUrl>,
    pub arm64: Option<HttpsUrl>,
    pub arm32: Option<HttpsUrl>,
}

impl ArchitectureUrls {
    fn validate(&self, path: &str) -> Result<()> {
        if self.keys().is_empty() {
            bail!("{path}: must contain at least one canonical architecture URL");
        }
        Ok(())
    }

    fn keys(&self) -> Vec<Architecture> {
        architecture_keys(self.amd64.is_some(), self.arm64.is_some(), self.arm32.is_some())
    }

    fn get(&self, architecture: Architecture) -> Option<&HttpsUrl> {
        match architecture {
            Architecture::Amd64 => self.amd64.as_ref(),
            Architecture::Arm64 => self.arm64.as_ref(),
            Architecture::Arm32 => self.arm32.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureHashes {
    pub amd64: Option<Sha256>,
    pub arm64: Option<Sha256>,
    pub arm32: Option<Sha256>,
}

impl ArchitectureHashes {
    fn validate(&self, path: &str) -> Result<()> {
        if self.keys().is_empty() {
            bail!("{path}: must contain at least one canonical architecture hash");
        }
        for (architecture, hash) in
            [("amd64", self.amd64.as_ref()), ("arm64", self.arm64.as_ref()), ("arm32", self.arm32.as_ref())]
        {
            if let Some(hash) = hash {
                hash.validate(&format!("{path}.{architecture}"))?;
            }
        }
        Ok(())
    }

    fn keys(&self) -> Vec<Architecture> {
        architecture_keys(self.amd64.is_some(), self.arm64.is_some(), self.arm32.is_some())
    }

    fn get(&self, architecture: Architecture) -> Option<&Sha256> {
        match architecture {
            Architecture::Amd64 => self.amd64.as_ref(),
            Architecture::Arm64 => self.arm64.as_ref(),
            Architecture::Arm32 => self.arm32.as_ref(),
        }
    }
}

fn architecture_keys(amd64: bool, arm64: bool, arm32: bool) -> Vec<Architecture> {
    [(Architecture::Amd64, amd64), (Architecture::Arm64, arm64), (Architecture::Arm32, arm32)]
        .into_iter()
        .filter_map(|(architecture, present)| present.then_some(architecture))
        .collect()
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
        validate_string_list(self.nerd.as_deref(), "fonts.nerd", validate_definition_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dotfiles {
    pub packages: Vec<String>,
}

impl Dotfiles {
    fn validate(&self) -> Result<()> {
        validate_string_values(&self.packages, "dotfiles.packages", validate_dotfile_package)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Integrations {
    pub docker: Option<DockerIntegration>,
    pub virtualbox: Option<VirtualBoxIntegration>,
    pub vscode: Option<VsCodeIntegration>,
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
            validate_executable(terminal, "desktop.terminal")?;
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Updates {
    pub apt: Option<AptUpdate>,
    pub flatpak: Option<bool>,
    pub tools: Option<ToolUpdates>,
    pub packages: Option<PackageUpdates>,
    pub fonts: Option<bool>,
}

impl Updates {
    fn validate(&self, config: &Config) -> Result<()> {
        if self.flatpak == Some(true)
            && !config
                .packages
                .as_ref()
                .and_then(|packages| packages.flatpak.as_ref())
                .is_some_and(|values| !values.is_empty())
        {
            bail!("updates.flatpak: requires configured packages.flatpak targets");
        }
        if self.fonts == Some(true)
            && !config.fonts.as_ref().and_then(|fonts| fonts.nerd.as_ref()).is_some_and(|values| !values.is_empty())
        {
            bail!("updates.fonts: requires configured fonts.nerd targets");
        }
        if let Some(tools) = &self.tools {
            tools.validate(config.tools.as_ref())?;
        }
        if let Some(packages) = &self.packages {
            packages.validate(config.packages.as_ref())?;
        }
        Ok(())
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
}

impl ToolUpdates {
    fn validate(&self, tools: Option<&Tools>) -> Result<()> {
        if self.rust == Some(true) && tools.and_then(|tools| tools.rust.as_ref()).is_none() {
            bail!("updates.tools.rust: requires a configured Rust selector");
        }
        if self.go == Some(true) && tools.and_then(|tools| tools.go.as_deref()) != Some("latest") {
            bail!("updates.tools.go: requires tools.go: latest");
        }
        if self.node == Some(true)
            && !tools.and_then(|tools| tools.node.as_deref()).is_some_and(|value| matches!(value, "lts" | "latest"))
        {
            bail!("updates.tools.node: requires a configured moving Node selector");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdates {
    pub cargo: Option<bool>,
    pub npm: Option<bool>,
    pub binaries: Option<bool>,
}

impl PackageUpdates {
    fn validate(&self, packages: Option<&Packages>) -> Result<()> {
        if self.cargo == Some(true)
            && !packages.and_then(|packages| packages.cargo.as_ref()).is_some_and(|values| !values.is_empty())
        {
            bail!("updates.packages.cargo: requires configured packages.cargo targets");
        }
        if self.npm == Some(true)
            && !packages.and_then(|packages| packages.npm.as_ref()).is_some_and(|values| !values.is_empty())
        {
            bail!("updates.packages.npm: requires configured packages.npm targets");
        }
        if self.binaries == Some(true)
            && !packages
                .and_then(|packages| packages.binaries.as_ref())
                .is_some_and(|binaries| binaries.iter().any(|binary| binary.source.is_github()))
        {
            bail!("updates.packages.binaries: requires at least one configured GitHub binary target");
        }
        Ok(())
    }
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

fn validate_key_path(value: &str, path: &str) -> Result<()> {
    if value.as_bytes().contains(&0) {
        bail!("{path}: key path must not contain a null byte");
    }
    let p = Path::new(value);
    if !p.is_absolute() {
        bail!("{path}: key path must be absolute");
    }
    let parent = p.parent().ok_or_else(|| anyhow::anyhow!("{path}: key path has no parent"))?;
    if parent != Path::new("/etc/apt/keyrings") && parent != Path::new("/usr/share/keyrings") {
        bail!("{path}: key path must be a direct child of /etc/apt/keyrings/ or /usr/share/keyrings/");
    }
    let file_name =
        p.file_name().and_then(|f| f.to_str()).ok_or_else(|| anyhow::anyhow!("{path}: key path has no file name"))?;
    if parent.join(file_name).to_str() != Some(value) {
        bail!("{path}: key path must use its canonical direct-child spelling");
    }
    if !file_name.ends_with(".asc") && !file_name.ends_with(".gpg") {
        bail!("{path}: key path extension must be .asc or .gpg");
    }
    let stem =
        p.file_stem().and_then(|s| s.to_str()).ok_or_else(|| anyhow::anyhow!("{path}: key path has no file stem"))?;
    validate_definition_name(stem, &format!("{path} file stem"))?;
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

fn validate_suite(value: &str, path: &str) -> Result<()> {
    if value == "system" || value == "*" {
        return Ok(());
    }
    validate_apt_token(value, path)
}

fn validate_apt_token(value: &str, path: &str) -> Result<()> {
    let re = Regex::new(r"^(?:\*|[a-z0-9][a-z0-9._+-]*)$").unwrap();
    if !re.is_match(value) {
        bail!("{path}: invalid APT token {value:?}; must be one lowercase token or the complete literal '*'");
    }
    Ok(())
}

fn validate_repository_path(value: &str, path: &str) -> Result<()> {
    if value == "./" {
        return Ok(());
    }
    if !value.ends_with('/') || value.starts_with('/') || value.contains('\\') {
        bail!("{path}: invalid repository path {value:?}; must be './' or a safe relative path ending in '/'");
    }
    let body = &value[..value.len() - 1];
    if body.is_empty() {
        bail!("{path}: invalid repository path {value:?}; must contain at least one safe relative path segment");
    }
    for segment in body.split('/') {
        if matches!(segment, "" | "." | "..") || validate_definition_name(segment, path).is_err() {
            bail!("{path}: invalid repository path {value:?}; contains invalid relative path segment {segment:?}");
        }
    }
    Ok(())
}

fn validate_github_repository(value: &str, path: &str) -> Result<()> {
    let re = Regex::new(r"^[a-zA-Z0-9-]+/[a-zA-Z0-9_.-]+$").unwrap();
    if !re.is_match(value) {
        bail!("{path}: invalid GitHub repository {value:?}; must be an owner/repository coordinate");
    }
    Ok(())
}

fn validate_docker_size(value: &str, path: &str) -> Result<()> {
    let re = Regex::new(r"^[1-9][0-9]*[kmg]$").unwrap();
    if !re.is_match(value) {
        bail!("{path}: invalid Docker size {value:?}; must be a positive decimal integer followed by k, m, or g");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsUrl(Url);

impl HttpsUrl {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        validate_non_empty_url(value)?;
        if value.chars().any(char::is_whitespace)
            || value.chars().any(char::is_control)
            || value.contains('\\')
            || has_substitution(value)
        {
            bail!("invalid HTTPS URL {value:?}; must be literal and contain no whitespace or substitutions");
        }
        let parsed =
            Url::parse(value).with_context(|| format!("invalid HTTPS URL {value:?}; must be a valid absolute URL"))?;
        let (raw_scheme, remainder) = value.split_once("://").unwrap_or_default();
        let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
        let (raw_host, empty_port) = if let Some(rest) = host_port.strip_prefix('[') {
            let closing = rest.find(']').map(|index| index + 1);
            let raw_host = closing.map_or(host_port, |index| &host_port[..=index]);
            let suffix = closing.map_or("", |index| &host_port[index + 1..]);
            (raw_host, suffix == ":")
        } else if let Some((host, port)) = host_port.rsplit_once(':') {
            (host, port.is_empty())
        } else {
            (host_port, false)
        };
        let invalid_host = parsed.host().is_none_or(|host| match host {
            Host::Ipv4(address) => raw_host != address.to_string(),
            Host::Ipv6(_) => false,
            Host::Domain(domain) => !valid_domain_host(domain),
        });
        if raw_scheme != "https"
            || parsed.scheme() != "https"
            || authority.is_empty()
            || authority.contains('%')
            || empty_port
            || invalid_host
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || authority.contains('@')
            || parsed.fragment().is_some()
        {
            bail!("invalid HTTPS URL {value:?}; must use HTTPS with a non-empty host and no credentials or fragment");
        }
        Ok(Self(parsed))
    }
}

impl<'de> Deserialize<'de> for HttpsUrl {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

fn validate_non_empty_url(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("invalid HTTPS URL {value:?}; must be a non-empty string");
    }
    Ok(())
}

fn valid_domain_host(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            label.len() <= 63
                && label.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
                && label.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
                && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn has_substitution(value: &str) -> bool {
    value.contains('$') || value.contains("{{") || value.contains("{%")
}
