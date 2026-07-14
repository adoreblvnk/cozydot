use crate::{
    domain::HttpsUrl,
    platform::{Architecture, Platform},
};
use anyhow::{bail, Context, Result};
use serde::{de, Deserialize, Deserializer};
use serde_yaml::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fmt, fs,
    path::Path,
};
use yaml_rust2::{
    parser::{Event, MarkedEventReceiver, Parser},
    scanner::{Marker, Scanner, TokenType},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigVersion;

impl<'de> Deserialize<'de> for ConfigVersion {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_string(deserializer)?;
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
        reject_yaml_extensions(text)?;
        preflight_document(text)?;
        let deserializer = serde_yaml::Deserializer::from_str(text);
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
        let (distro, upstream) = validate_platform_identity(platform)?;
        let desktop = DesktopKind::from_platform(&platform.desktop)?;

        if let Some(require) = self
            .system
            .as_ref()
            .and_then(|system| system.require.as_ref())
        {
            if require
                .distros
                .as_ref()
                .is_some_and(|allowed| !allowed.contains(&distro))
            {
                bail!(
                    "system.require.distros: detected distribution {:?} is not allowed",
                    platform.distro
                );
            }
            if require
                .desktops
                .as_ref()
                .is_some_and(|allowed| !allowed.contains(&desktop))
            {
                bail!(
                    "system.require.desktops: detected desktop {:?} is not allowed",
                    platform.desktop
                );
            }
        }

        if let Some(sources) = self
            .system
            .as_ref()
            .and_then(|system| system.apt.as_ref())
            .and_then(|apt| apt.sources.as_ref())
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

        for (index, binary) in self
            .packages
            .as_ref()
            .and_then(|packages| packages.binaries.as_ref())
            .into_iter()
            .flatten()
            .enumerate()
        {
            binary
                .source
                .require_architecture(platform.architecture)
                .with_context(|| {
                    format!(
                        "packages.binaries[{index}].source.{}",
                        platform.architecture.canonical()
                    )
                })?;
        }

        if let Some(configured) = &self.desktop {
            if configured.has_neutral_intent()
                && !matches!(desktop, DesktopKind::Gnome | DesktopKind::Cinnamon)
            {
                bail!(
                    "desktop: theme, terminal, and idle settings require GNOME or Cinnamon; detected {:?}",
                    platform.desktop
                );
            }
            if configured.gnome.is_some() && desktop != DesktopKind::Gnome {
                bail!(
                    "desktop.gnome: requires resolved GNOME desktop; detected {:?}",
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
        if let Some(tools) = &self.tools {
            tools.validate()?;
        }
        if let Some(fonts) = &self.fonts {
            fonts.validate()?;
        }
        if let Some(dotfiles) = &self.dotfiles {
            dotfiles.validate()?;
        }
        if let Some(integrations) = &self.integrations {
            integrations.validate()?;
        }
        if let Some(desktop) = &self.desktop {
            desktop.validate()?;
        }
        if let Some(updates) = &self.updates {
            updates.validate(self)?;
        }
        if self
            .packages
            .as_ref()
            .and_then(|packages| packages.cargo.as_ref())
            .is_some()
            && self
                .tools
                .as_ref()
                .and_then(|tools| tools.rust.as_ref())
                .is_none()
        {
            bail!("packages.cargo: requires tools.rust");
        }
        if self
            .packages
            .as_ref()
            .and_then(|packages| packages.npm.as_ref())
            .is_some()
            && self
                .tools
                .as_ref()
                .and_then(|tools| tools.node.as_ref())
                .is_none()
        {
            bail!("packages.npm: requires tools.node");
        }
        Ok(())
    }
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
enum Family {
    Ubuntu,
    Debian,
}

fn validate_platform_identity(platform: &Platform) -> Result<(Distro, Family)> {
    let distro = Distro::parse_platform(&platform.distro)?;
    let upstream = match platform.upstream.as_str() {
        "ubuntu" => Family::Ubuntu,
        "debian" => Family::Debian,
        value => bail!("system.require.distros: unsupported platform upstream family {value:?}"),
    };
    let valid = match distro {
        Distro::Ubuntu | Distro::Pop | Distro::Zorin => upstream == Family::Ubuntu,
        Distro::Debian | Distro::Kali | Distro::Tails | Distro::Deepin => {
            upstream == Family::Debian
        }
        Distro::Linuxmint => true,
    };
    if !valid {
        bail!(
            "system.require.distros: detected distribution {:?} is inconsistent with upstream family {:?}",
            platform.distro,
            platform.upstream
        );
    }
    Ok((distro, upstream))
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
        require_effective(
            self.require.is_some()
                || self.ensure_admin.is_some()
                || self.apt.is_some()
                || self.ubuntu.is_some(),
            "system",
        )?;
        if let Some(require) = &self.require {
            require.validate()?;
        }
        true_only(self.ensure_admin, "system.ensure_admin")?;
        if let Some(apt) = &self.apt {
            apt.validate()?;
        }
        if let Some(ubuntu) = &self.ubuntu {
            ubuntu.validate()?;
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

impl PlatformRequirements {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.distros.is_some() || self.desktops.is_some(),
            "system.require",
        )?;
        validate_non_empty_unique(self.distros.as_deref(), "system.require.distros")?;
        validate_non_empty_unique(self.desktops.as_deref(), "system.require.desktops")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemApt {
    pub sources: Option<OfficialSources>,
    pub unattended_upgrades: Option<EnabledDisabled>,
}

impl SystemApt {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.sources.is_some() || self.unattended_upgrades.is_some(),
            "system.apt",
        )?;
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
                validate_non_empty_map(components, "system.apt.sources.components")?;
                for (key, values) in components {
                    validate_non_empty_unique(
                        Some(values),
                        &format!("system.apt.sources.components.{}", key.as_str()),
                    )?;
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

    fn validate_for_platform(
        &self,
        platform: &Platform,
        distro: Distro,
        upstream: Family,
    ) -> Result<()> {
        if self.mode == SourceMode::Preserve {
            return Ok(());
        }
        if !matches!(distro, Distro::Ubuntu | Distro::Debian | Distro::Kali) {
            bail!(
                "system.apt.sources.mode: managed is unsupported for distribution {:?}; use preserve",
                platform.distro
            );
        }
        let components = self
            .components
            .as_ref()
            .expect("validated managed components");
        let (_, selected) = select_distro_map(components, distro, upstream).ok_or_else(|| {
            anyhow::anyhow!(
                "system.apt.sources.components: no entry for distribution {:?}, upstream {:?}, or default",
                platform.distro,
                platform.upstream
            )
        })?;
        let names = selected
            .iter()
            .map(AptComponent::as_str)
            .collect::<Vec<_>>();
        platform.managed_apt_sources(&names)?;
        Ok(())
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

impl UbuntuSystem {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.snap.is_some() || self.codecs.is_some(),
            "system.ubuntu",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    pub apt: Option<AptPackages>,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub flatpak: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub cargo: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub npm: Option<Vec<String>>,
    pub binaries: Option<Vec<BinaryPackage>>,
}

impl Packages {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.apt.is_some()
                || self.flatpak.is_some()
                || self.cargo.is_some()
                || self.npm.is_some()
                || self.binaries.is_some(),
            "packages",
        )?;
        if let Some(apt) = &self.apt {
            apt.validate()?;
        }
        validate_string_list(
            self.flatpak.as_deref(),
            "packages.flatpak",
            validate_flatpak_id,
        )?;
        validate_string_list(
            self.cargo.as_deref(),
            "packages.cargo",
            validate_cargo_package,
        )?;
        validate_string_list(self.npm.as_deref(), "packages.npm", validate_npm_package)?;
        if let Some(binaries) = &self.binaries {
            if binaries.is_empty() {
                bail!("packages.binaries: must be a non-empty sequence");
            }
            let mut names = HashSet::new();
            let mut command_owners = BTreeMap::new();
            for (index, binary) in binaries.iter().enumerate() {
                binary.validate(index)?;
                if !names.insert(binary.name.as_str()) {
                    bail!(
                        "packages.binaries[{index}].name: duplicate binary name {:?}",
                        binary.name
                    );
                }
                for (command_index, command) in binary.commands.iter().enumerate() {
                    let command_path =
                        format!("packages.binaries[{index}].commands[{command_index}]");
                    if let Some(owner_path) =
                        command_owners.insert(command.as_str(), command_path.clone())
                    {
                        bail!(
                            "{command_path}: command {command:?} is already claimed by {owner_path}"
                        );
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
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub remove: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub install: Option<Vec<String>>,
    pub repositories: Option<Vec<Repository>>,
}

impl AptPackages {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.remove.is_some() || self.install.is_some() || self.repositories.is_some(),
            "packages.apt",
        )?;
        validate_string_list(
            self.remove.as_deref(),
            "packages.apt.remove",
            validate_debian_package,
        )?;
        validate_string_list(
            self.install.as_deref(),
            "packages.apt.install",
            validate_debian_package,
        )?;
        let mut installed = HashSet::new();
        for package in self.install.iter().flatten() {
            installed.insert(package.as_str());
        }
        if let Some(repositories) = &self.repositories {
            if repositories.is_empty() {
                bail!("packages.apt.repositories: must be a non-empty sequence");
            }
            let mut names = HashSet::new();
            let mut stems = HashSet::new();
            for (index, repository) in repositories.iter().enumerate() {
                repository.validate(index)?;
                if !names.insert(repository.name.as_str()) {
                    bail!(
                        "packages.apt.repositories[{index}].name: duplicate repository name {:?}",
                        repository.name
                    );
                }
                let stem = repository.filename_stem();
                if !stems.insert(stem.clone()) {
                    bail!("packages.apt.repositories[{index}].name: filename stem {stem:?} collides with an earlier repository");
                }
                for (package_index, package) in repository.packages.iter().enumerate() {
                    if !installed.insert(package) {
                        bail!("packages.apt.repositories[{index}].packages[{package_index}]: duplicate APT installation ownership for {package:?}");
                    }
                }
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
    fn as_str(self) -> &'static str {
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

fn select_distro_map<T>(
    map: &BTreeMap<DistroMapKey, T>,
    distro: Distro,
    upstream: Family,
) -> Option<(DistroMapKey, &T)> {
    let exact = DistroMapKey::from_distro(distro);
    let family = DistroMapKey::from_family(upstream);
    map.get(&exact)
        .map(|value| (exact, value))
        .or_else(|| map.get(&family).map(|value| (family, value)))
        .or_else(|| {
            map.get(&DistroMapKey::Default)
                .map(|value| (DistroMapKey::Default, value))
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    #[serde(deserialize_with = "deserialize_string")]
    pub name: String,
    pub key: HttpsUrl,
    pub urls: BTreeMap<DistroMapKey, HttpsUrl>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub suite: Option<String>,
    pub components: Option<Vec<AptToken>>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub path: Option<String>,
    #[serde(deserialize_with = "deserialize_strings")]
    pub packages: Vec<String>,
}

impl Repository {
    pub fn filename_stem(&self) -> String {
        let mut result = String::new();
        let mut separator = false;
        for byte in self.name.bytes() {
            if byte.is_ascii_alphanumeric() {
                if separator && !result.is_empty() {
                    result.push('-');
                }
                result.push((byte as char).to_ascii_lowercase());
                separator = false;
            } else {
                separator = true;
            }
        }
        result
    }

    fn validate(&self, index: usize) -> Result<()> {
        let path = format!("packages.apt.repositories[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        validate_non_empty_map(&self.urls, &format!("{path}.urls"))?;
        validate_string_values(
            &self.packages,
            &format!("{path}.packages"),
            validate_debian_package,
        )?;
        match (&self.suite, &self.components, &self.path) {
            (Some(suite), Some(components), None) => {
                validate_suite(suite, &format!("{path}.suite"))?;
                validate_non_empty_unique(Some(components), &format!("{path}.components"))?;
                for (component_index, component) in components.iter().enumerate() {
                    let component_path = format!("{path}.components[{component_index}]");
                    validate_apt_token(component.as_str(), &component_path)?;
                    if component.as_str() == "system" {
                        bail!(
                            "{component_path}: value {:?} is reserved for the suite field",
                            component.as_str()
                        );
                    }
                }
            }
            (None, None, Some(exact_path)) => {
                validate_repository_path(exact_path, &format!("{path}.path"))?
            }
            _ => bail!("{path}: requires exactly suite with non-empty components, or path"),
        }
        Ok(())
    }

    fn validate_for_platform(
        &self,
        index: usize,
        platform: &Platform,
        distro: Distro,
        upstream: Family,
    ) -> Result<()> {
        let (key, _) = select_distro_map(&self.urls, distro, upstream).ok_or_else(|| {
            anyhow::anyhow!(
                "packages.apt.repositories[{index}].urls: no URL for distribution {:?}, upstream {:?}, or default",
                platform.distro,
                platform.upstream
            )
        })?;
        if self.suite.as_deref() == Some("system") {
            if key == DistroMapKey::Default {
                bail!("packages.apt.repositories[{index}].suite: system cannot use a default URL because it has no repository-family codename");
            }
            let codename = if key == DistroMapKey::from_distro(distro) {
                &platform.distro_codename
            } else {
                &platform.base_codename
            };
            validate_apt_token(
                codename,
                &format!("packages.apt.repositories[{index}].suite resolved codename"),
            )?;
        }
        Ok(())
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
        let value = deserialize_string(deserializer)?;
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
    #[serde(deserialize_with = "deserialize_string")]
    pub name: String,
    pub format: BinaryFormat,
    #[serde(deserialize_with = "deserialize_strings")]
    pub commands: Vec<String>,
    pub source: BinarySource,
}

impl BinaryPackage {
    fn validate(&self, index: usize) -> Result<()> {
        let path = format!("packages.binaries[{index}]");
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        validate_string_values(
            &self.commands,
            &format!("{path}.commands"),
            validate_executable,
        )?;
        self.source.validate(&format!("{path}.source"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase", deny_unknown_fields)]
pub enum BinarySource {
    Github {
        #[serde(deserialize_with = "deserialize_string")]
        repository: String,
        assets: AssetMap,
    },
    Url {
        urls: ArchitectureUrls,
        sha256: ArchitectureHashes,
    },
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
                    bail!(
                        "{path}: urls and sha256 must contain exactly the same architecture keys"
                    );
                }
                Ok(())
            }
        }
    }

    fn require_architecture(&self, architecture: Architecture) -> Result<()> {
        let present = match self {
            Self::Github { assets, .. } => assets.contains(architecture),
            Self::Url { urls, sha256 } => {
                urls.contains(architecture) && sha256.contains(architecture)
            }
        };
        if !present {
            bail!("missing native architecture selector");
        }
        Ok(())
    }

    fn is_github(&self) -> bool {
        matches!(self, Self::Github { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetMap {
    pub amd64: Option<AssetSelector>,
    pub arm64: Option<AssetSelector>,
    pub arm32: Option<AssetSelector>,
    pub riscv64: Option<AssetSelector>,
}

impl AssetMap {
    fn validate(&self, path: &str) -> Result<()> {
        if self.values().iter().all(|(_, value)| value.is_none()) {
            bail!("{path}: must contain at least one canonical architecture selector");
        }
        for (architecture, selector) in self.values() {
            if let Some(selector) = selector {
                selector.validate(&format!("{path}.{architecture}"))?;
            }
        }
        Ok(())
    }

    fn values(&self) -> [(&'static str, Option<&AssetSelector>); 4] {
        [
            ("amd64", self.amd64.as_ref()),
            ("arm64", self.arm64.as_ref()),
            ("arm32", self.arm32.as_ref()),
            ("riscv64", self.riscv64.as_ref()),
        ]
    }

    fn contains(&self, architecture: Architecture) -> bool {
        match architecture {
            Architecture::Amd64 => self.amd64.is_some(),
            Architecture::Arm64 => self.arm64.is_some(),
            Architecture::Arm32 => self.arm32.is_some(),
            Architecture::Riscv64 => self.riscv64.is_some(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSelector {
    #[serde(deserialize_with = "deserialize_string")]
    pub include: String,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub exclude: Option<Vec<String>>,
    pub sha256: Option<Sha256>,
}

impl AssetSelector {
    fn validate(&self, path: &str) -> Result<()> {
        validate_wildcard(&self.include, &format!("{path}.include"))?;
        if let Some(exclude) = &self.exclude {
            validate_string_values(exclude, &format!("{path}.exclude"), validate_wildcard)?;
        }
        if let Some(sha256) = &self.sha256 {
            sha256.validate(&format!("{path}.sha256"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256(String);

impl Sha256 {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self, path: &str) -> Result<()> {
        let value = &self.0;
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
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
        deserialize_string(deserializer).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureUrls {
    pub amd64: Option<HttpsUrl>,
    pub arm64: Option<HttpsUrl>,
    pub arm32: Option<HttpsUrl>,
    pub riscv64: Option<HttpsUrl>,
}

impl ArchitectureUrls {
    fn validate(&self, path: &str) -> Result<()> {
        if self.keys().is_empty() {
            bail!("{path}: must contain at least one canonical architecture URL");
        }
        Ok(())
    }

    fn keys(&self) -> Vec<Architecture> {
        architecture_keys(
            self.amd64.is_some(),
            self.arm64.is_some(),
            self.arm32.is_some(),
            self.riscv64.is_some(),
        )
    }

    fn contains(&self, architecture: Architecture) -> bool {
        self.keys().contains(&architecture)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureHashes {
    pub amd64: Option<Sha256>,
    pub arm64: Option<Sha256>,
    pub arm32: Option<Sha256>,
    pub riscv64: Option<Sha256>,
}

impl ArchitectureHashes {
    fn validate(&self, path: &str) -> Result<()> {
        if self.keys().is_empty() {
            bail!("{path}: must contain at least one canonical architecture hash");
        }
        for (architecture, hash) in [
            ("amd64", self.amd64.as_ref()),
            ("arm64", self.arm64.as_ref()),
            ("arm32", self.arm32.as_ref()),
            ("riscv64", self.riscv64.as_ref()),
        ] {
            if let Some(hash) = hash {
                hash.validate(&format!("{path}.{architecture}"))?;
            }
        }
        Ok(())
    }

    fn keys(&self) -> Vec<Architecture> {
        architecture_keys(
            self.amd64.is_some(),
            self.arm64.is_some(),
            self.arm32.is_some(),
            self.riscv64.is_some(),
        )
    }

    fn contains(&self, architecture: Architecture) -> bool {
        self.keys().contains(&architecture)
    }
}

fn architecture_keys(amd64: bool, arm64: bool, arm32: bool, riscv64: bool) -> Vec<Architecture> {
    [
        (Architecture::Amd64, amd64),
        (Architecture::Arm64, arm64),
        (Architecture::Arm32, arm32),
        (Architecture::Riscv64, riscv64),
    ]
    .into_iter()
    .filter_map(|(architecture, present)| present.then_some(architecture))
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub rust: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub go: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub node: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub python: Option<String>,
}

impl Tools {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.rust.is_some()
                || self.go.is_some()
                || self.node.is_some()
                || self.python.is_some(),
            "tools",
        )?;
        if let Some(value) = &self.rust {
            validate_rust_selector(value, "tools.rust")?;
        }
        if let Some(value) = &self.go {
            if value != "latest" {
                validate_numeric_version(value, "tools.go", 2, 3)?;
            }
        }
        if let Some(value) = &self.node {
            if !matches!(value.as_str(), "lts" | "latest") {
                validate_numeric_version(value, "tools.node", 1, 3)?;
            }
        }
        if let Some(value) = &self.python {
            validate_numeric_version(value, "tools.python", 2, 3)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub nerd: Option<Vec<String>>,
}

impl Fonts {
    fn validate(&self) -> Result<()> {
        require_effective(self.nerd.is_some(), "fonts")?;
        validate_string_list(self.nerd.as_deref(), "fonts.nerd", validate_definition_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dotfiles {
    #[serde(deserialize_with = "deserialize_strings")]
    pub packages: Vec<String>,
}

impl Dotfiles {
    fn validate(&self) -> Result<()> {
        validate_string_values(
            &self.packages,
            "dotfiles.packages",
            validate_dotfile_package,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Integrations {
    pub docker: Option<DockerIntegration>,
    pub virtualbox: Option<VirtualBoxIntegration>,
    pub vscode: Option<VsCodeIntegration>,
}

impl Integrations {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.docker.is_some() || self.virtualbox.is_some() || self.vscode.is_some(),
            "integrations",
        )?;
        if let Some(docker) = &self.docker {
            docker.validate()?;
        }
        if let Some(virtualbox) = &self.virtualbox {
            virtualbox.validate()?;
        }
        if let Some(vscode) = &self.vscode {
            vscode.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerIntegration {
    pub add_user_to_group: Option<bool>,
    pub logging: Option<DockerLogging>,
}

impl DockerIntegration {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.add_user_to_group.is_some() || self.logging.is_some(),
            "integrations.docker",
        )?;
        true_only(
            self.add_user_to_group,
            "integrations.docker.add_user_to_group",
        )?;
        if let Some(logging) = &self.logging {
            logging.validate()?;
        }
        Ok(())
    }
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
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub max_size: Option<String>,
}

impl DockerLogging {
    fn validate(&self) -> Result<()> {
        if let Some(size) = &self.max_size {
            validate_docker_size(size, "integrations.docker.logging.max_size")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualBoxIntegration {
    pub add_user_to_group: Option<bool>,
}

impl VirtualBoxIntegration {
    fn validate(&self) -> Result<()> {
        require_effective(self.add_user_to_group.is_some(), "integrations.virtualbox")?;
        true_only(
            self.add_user_to_group,
            "integrations.virtualbox.add_user_to_group",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VsCodeIntegration {
    #[serde(deserialize_with = "deserialize_strings")]
    pub extensions: Vec<String>,
}

impl VsCodeIntegration {
    fn validate(&self) -> Result<()> {
        validate_string_values(
            &self.extensions,
            "integrations.vscode.extensions",
            validate_vscode_id,
        )
    }
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
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub terminal: Option<String>,
    pub idle: Option<Idle>,
    pub gnome: Option<Gnome>,
}

impl Desktop {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.theme.is_some()
                || self.terminal.is_some()
                || self.idle.is_some()
                || self.gnome.is_some(),
            "desktop",
        )?;
        if let Some(terminal) = &self.terminal {
            validate_executable(terminal, "desktop.terminal")?;
        }
        if let Some(idle) = &self.idle {
            idle.validate()?;
        }
        if let Some(gnome) = &self.gnome {
            gnome.validate()?;
        }
        Ok(())
    }

    fn has_neutral_intent(&self) -> bool {
        self.theme.is_some() || self.terminal.is_some() || self.idle.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Idle {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub timeout: Option<String>,
    pub dim: Option<bool>,
}

impl Idle {
    fn validate(&self) -> Result<()> {
        require_effective(self.timeout.is_some() || self.dim.is_some(), "desktop.idle")?;
        if let Some(timeout) = &self.timeout {
            validate_duration(timeout, "desktop.idle.timeout")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gnome {
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub extensions: Option<Vec<String>>,
    pub dock: Option<bool>,
    pub rounded_corners: Option<bool>,
}

impl Gnome {
    fn validate(&self) -> Result<()> {
        require_effective(
            self.extensions.is_some() || self.dock.is_some() || self.rounded_corners.is_some(),
            "desktop.gnome",
        )?;
        validate_string_list(
            self.extensions.as_deref(),
            "desktop.gnome.extensions",
            validate_gnome_uuid,
        )?;
        true_only(self.dock, "desktop.gnome.dock")?;
        true_only(self.rounded_corners, "desktop.gnome.rounded_corners")
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
        require_effective(
            self.apt.is_some()
                || self.flatpak.is_some()
                || self.tools.is_some()
                || self.packages.is_some()
                || self.fonts.is_some(),
            "updates",
        )?;
        true_only(self.flatpak, "updates.flatpak")?;
        true_only(self.fonts, "updates.fonts")?;
        if self.flatpak.is_some()
            && config
                .packages
                .as_ref()
                .and_then(|packages| packages.flatpak.as_ref())
                .is_none()
        {
            bail!("updates.flatpak: requires configured packages.flatpak targets");
        }
        if self.fonts.is_some()
            && config
                .fonts
                .as_ref()
                .and_then(|fonts| fonts.nerd.as_ref())
                .is_none()
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
        require_effective(
            self.rust.is_some() || self.go.is_some() || self.node.is_some(),
            "updates.tools",
        )?;
        true_only(self.rust, "updates.tools.rust")?;
        true_only(self.go, "updates.tools.go")?;
        true_only(self.node, "updates.tools.node")?;
        if self.rust.is_some()
            && !tools
                .and_then(|tools| tools.rust.as_deref())
                .is_some_and(|value| matches!(value, "stable" | "beta" | "nightly"))
        {
            bail!("updates.tools.rust: requires a configured moving Rust selector");
        }
        if self.go.is_some() && tools.and_then(|tools| tools.go.as_deref()) != Some("latest") {
            bail!("updates.tools.go: requires tools.go: latest");
        }
        if self.node.is_some()
            && !tools
                .and_then(|tools| tools.node.as_deref())
                .is_some_and(|value| matches!(value, "lts" | "latest"))
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
        require_effective(
            self.cargo.is_some() || self.npm.is_some() || self.binaries.is_some(),
            "updates.packages",
        )?;
        true_only(self.cargo, "updates.packages.cargo")?;
        true_only(self.npm, "updates.packages.npm")?;
        true_only(self.binaries, "updates.packages.binaries")?;
        if self.cargo.is_some()
            && packages
                .and_then(|packages| packages.cargo.as_ref())
                .is_none()
        {
            bail!("updates.packages.cargo: requires configured packages.cargo targets");
        }
        if self.npm.is_some()
            && packages
                .and_then(|packages| packages.npm.as_ref())
                .is_none()
        {
            bail!("updates.packages.npm: requires configured packages.npm targets");
        }
        if self.binaries.is_some()
            && !packages
                .and_then(|packages| packages.binaries.as_ref())
                .is_some_and(|binaries| binaries.iter().any(|binary| binary.source.is_github()))
        {
            bail!(
                "updates.packages.binaries: requires at least one configured GitHub binary target"
            );
        }
        Ok(())
    }
}

fn preflight_document(text: &str) -> Result<()> {
    let value: Value = serde_yaml::from_str(text).context("parse YAML preflight")?;
    let root = value
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("config: expected a YAML mapping"))?;
    let version_key = Value::String("version".into());
    let version = root.get(&version_key).ok_or_else(|| {
        anyhow::anyhow!(
            "version: missing required configuration version; only version \"1.0.0\" is supported"
        )
    })?;
    match version {
        Value::String(value) if value == "1.0.0" => {}
        Value::String(value) => bail!("version: unsupported configuration version {value:?}; only version \"1.0.0\" is supported"),
        other => bail!("version: unsupported configuration version value {other:?}; expected YAML string \"1.0.0\""),
    }
    reject_nulls(&value, "config")
}

fn reject_nulls(value: &Value, path: &str) -> Result<()> {
    match value {
        Value::Null => bail!("{path}: explicit null is invalid; omit the field instead"),
        Value::Sequence(values) => {
            for (index, value) in values.iter().enumerate() {
                reject_nulls(value, &format!("{path}[{index}]"))?;
            }
        }
        Value::Mapping(values) => {
            for (key, value) in values {
                let key = key.as_str().unwrap_or("<non-string-key>");
                let child = if path == "config" {
                    key.to_owned()
                } else {
                    format!("{path}.{key}")
                };
                reject_nulls(value, &child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn reject_yaml_extensions(text: &str) -> Result<()> {
    for token in Scanner::new(text.chars()) {
        let extension = match token.1 {
            TokenType::VersionDirective(..) | TokenType::TagDirective(..) => {
                Some("YAML directives")
            }
            TokenType::Tag(..) => Some("YAML tags"),
            TokenType::Anchor(..) => Some("YAML anchors"),
            TokenType::Alias(..) => Some("YAML aliases"),
            _ => None,
        };
        if let Some(extension) = extension {
            bail!(
                "line {}, column {}: {extension} are not supported by configuration version 1.0.0",
                token.0.line() + 1,
                token.0.col() + 1
            );
        }
    }

    #[derive(Default)]
    struct Receiver {
        documents: usize,
        error: Option<anyhow::Error>,
    }
    impl MarkedEventReceiver for Receiver {
        fn on_event(&mut self, event: Event, marker: Marker) {
            if self.error.is_some() {
                return;
            }
            let extension = match event {
                Event::DocumentStart => {
                    self.documents += 1;
                    (self.documents > 1).then_some("multiple YAML documents")
                }
                Event::Alias(_) => Some("YAML aliases"),
                Event::Scalar(_, _, anchor, tag)
                | Event::SequenceStart(anchor, tag)
                | Event::MappingStart(anchor, tag) => {
                    if anchor != 0 {
                        Some("YAML anchors")
                    } else if tag.is_some() {
                        Some("YAML tags")
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(extension) = extension {
                self.error = Some(anyhow::anyhow!("line {}, column {}: {extension} are not supported by configuration version 1.0.0", marker.line() + 1, marker.col() + 1));
            }
        }
    }
    let mut receiver = Receiver::default();
    Parser::new_from_str(text)
        .load(&mut receiver, true)
        .context("parse YAML extension preflight")?;
    if let Some(error) = receiver.error {
        return Err(error);
    }
    Ok(())
}

fn deserialize_string<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(StrictStringVisitor)
}

fn deserialize_optional_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_string(deserializer).map(Some)
}

fn deserialize_strings<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Visitor;
    impl<'de> de::Visitor<'de> for Visitor {
        type Value = Vec<String>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a sequence of YAML strings")
        }
        fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
            while let Some(value) = sequence.next_element_seed(StrictStringSeed)? {
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Visitor)
}

fn deserialize_optional_strings<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_strings(deserializer).map(Some)
}

struct StrictStringVisitor;
impl de::Visitor<'_> for StrictStringVisitor {
    type Value = String;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a YAML string")
    }
    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(value.to_owned())
    }
    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(value)
    }
}
struct StrictStringSeed;
impl<'de> de::DeserializeSeed<'de> for StrictStringSeed {
    type Value = String;
    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_string(deserializer)
    }
}

fn require_effective(effective: bool, path: &str) -> Result<()> {
    if !effective {
        bail!("{path}: must contain at least one effective child; omit the mapping instead");
    }
    Ok(())
}

fn true_only(value: Option<bool>, path: &str) -> Result<()> {
    if value == Some(false) {
        bail!("{path}: false is redundant and invalid; omit the field instead");
    }
    Ok(())
}

fn validate_non_empty_map<K, V>(map: &BTreeMap<K, V>, path: &str) -> Result<()> {
    if map.is_empty() {
        bail!("{path}: must be a non-empty mapping");
    }
    Ok(())
}

fn validate_non_empty_unique<T: Eq + std::hash::Hash + fmt::Debug>(
    values: Option<&[T]>,
    path: &str,
) -> Result<()> {
    let Some(values) = values else {
        return Ok(());
    };
    if values.is_empty() {
        bail!("{path}: must be a non-empty sequence");
    }
    let mut seen = HashSet::new();
    for (index, value) in values.iter().enumerate() {
        if !seen.insert(value) {
            bail!("{path}[{index}]: duplicate value {value:?}");
        }
    }
    Ok(())
}

fn validate_string_list(
    values: Option<&[String]>,
    path: &str,
    validator: fn(&str, &str) -> Result<()>,
) -> Result<()> {
    let Some(values) = values else {
        return Ok(());
    };
    validate_string_values(values, path, validator)
}

fn validate_string_values(
    values: &[String],
    path: &str,
    validator: fn(&str, &str) -> Result<()>,
) -> Result<()> {
    if values.is_empty() {
        bail!("{path}: must be a non-empty sequence");
    }
    let mut seen = HashSet::new();
    for (index, value) in values.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        validator(value, &item_path)?;
        if !seen.insert(value.as_str()) {
            bail!("{item_path}: duplicate value {value:?}");
        }
    }
    Ok(())
}

fn validate_definition_name(value: &str, path: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes
        .first()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || bytes
            .last()
            .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(byte))
    {
        bail!("{path}: invalid value {value:?}; must start and end with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', or '-'");
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
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        bail!("{path}: invalid executable basename {value:?}; must start with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', '+', or '-'");
    }
    Ok(())
}

fn validate_debian_package(value: &str, path: &str) -> Result<()> {
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"+.-".contains(&byte)
        })
    {
        bail!("{path}: invalid Debian package name {value:?}; must be unversioned");
    }
    Ok(())
}

fn validate_cargo_package(value: &str, path: &str) -> Result<()> {
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        bail!("{path}: invalid Cargo package name {value:?}; must be unversioned");
    }
    Ok(())
}

fn validate_npm_package(value: &str, path: &str) -> Result<()> {
    let valid_part = |part: &str| {
        let mut bytes = part.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
    };
    let valid = if let Some(scoped) = value.strip_prefix('@') {
        let mut parts = scoped.split('/');
        valid_part(parts.next().unwrap_or_default())
            && valid_part(parts.next().unwrap_or_default())
            && parts.next().is_none()
    } else {
        !value.contains('/') && valid_part(value)
    };
    if !valid {
        bail!("{path}: invalid npm package name {value:?}; must be an unversioned lowercase name or @scope/name");
    }
    Ok(())
}

fn validate_flatpak_id(value: &str, path: &str) -> Result<()> {
    let components = value.split('.').collect::<Vec<_>>();
    let valid = |component: &&str| {
        let mut bytes = component.bytes();
        bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    };
    if components.len() < 3 || !components.iter().all(valid) {
        bail!("{path}: invalid Flatpak application ID {value:?}; must be canonical");
    }
    Ok(())
}

fn validate_vscode_id(value: &str, path: &str) -> Result<()> {
    let valid = |part: &str| {
        let mut bytes = part.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    let mut parts = value.split('.');
    if !valid(parts.next().unwrap_or_default())
        || !valid(parts.next().unwrap_or_default())
        || parts.next().is_some()
    {
        bail!("{path}: invalid VS Code extension identifier {value:?}; must be lowercase publisher.extension");
    }
    Ok(())
}

fn validate_gnome_uuid(value: &str, path: &str) -> Result<()> {
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    let mut parts = value.split('@');
    if !valid(parts.next().unwrap_or_default())
        || !valid(parts.next().unwrap_or_default())
        || parts.next().is_some()
    {
        bail!("{path}: invalid GNOME extension UUID {value:?}; must contain exactly one '@'");
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
    if value == "*" {
        return Ok(());
    }
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._+-".contains(&byte)
        })
    {
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
    let valid_owner = |owner: &str| {
        !owner.is_empty()
            && owner
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && owner
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
            && owner
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    };
    let valid_repo = |repo: &str| {
        !repo.is_empty()
            && repo
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            && repo
                .bytes()
                .any(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    };
    let mut parts = value.split('/');
    if !valid_owner(parts.next().unwrap_or_default())
        || !valid_repo(parts.next().unwrap_or_default())
        || parts.next().is_some()
    {
        bail!(
            "{path}: invalid GitHub repository {value:?}; must be an owner/repository coordinate"
        );
    }
    Ok(())
}

fn validate_wildcard(value: &str, path: &str) -> Result<()> {
    if value.is_empty()
        || !value.contains(['*', '?'])
        || value.contains(['/', '\\', '[', ']', '{', '}', '$', '`'])
        || value.chars().any(char::is_control)
    {
        bail!("{path}: invalid asset selector {value:?}; must be a whole-filename wildcard using only '*' and '?' operators");
    }
    Ok(())
}

fn validate_rust_selector(value: &str, path: &str) -> Result<()> {
    if matches!(value, "stable" | "beta" | "nightly") {
        return Ok(());
    }
    if let Some(date) = value.strip_prefix("nightly-") {
        let parts = date.split('-').collect::<Vec<_>>();
        if parts.len() == 3
            && parts[0].len() == 4
            && parts[1].len() == 2
            && parts[2].len() == 2
            && parts
                .iter()
                .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
            && valid_calendar_date(&parts)
        {
            return Ok(());
        }
    }
    validate_numeric_version(value, path, 2, 3)
}

fn valid_calendar_date(parts: &[&str]) -> bool {
    let (Ok(year), Ok(month), Ok(day)) = (
        parts[0].parse::<u16>(),
        parts[1].parse::<u8>(),
        parts[2].parse::<u8>(),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year != 0 && (1..=days).contains(&day)
}

fn validate_numeric_version(value: &str, path: &str, min: usize, max: usize) -> Result<()> {
    let parts = value.split('.').collect::<Vec<_>>();
    if !(min..=max).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        bail!("{path}: invalid selector {value:?}; expected {min} to {max} numeric components");
    }
    Ok(())
}

fn validate_duration(value: &str, path: &str) -> Result<()> {
    let number = value
        .strip_suffix('s')
        .or_else(|| value.strip_suffix('m'))
        .or_else(|| value.strip_suffix('h'));
    if number
        .is_none_or(|number| number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()))
    {
        bail!("{path}: invalid duration {value:?}; must be a non-negative decimal integer followed by s, m, or h");
    }
    Ok(())
}

fn validate_docker_size(value: &str, path: &str) -> Result<()> {
    let number = value
        .strip_suffix('k')
        .or_else(|| value.strip_suffix('m'))
        .or_else(|| value.strip_suffix('g'));
    if number.is_none_or(|number| {
        number.is_empty()
            || !number.bytes().all(|byte| byte.is_ascii_digit())
            || number.bytes().all(|byte| byte == b'0')
    }) {
        bail!("{path}: invalid Docker size {value:?}; must be a positive decimal integer followed by k, m, or g");
    }
    Ok(())
}
