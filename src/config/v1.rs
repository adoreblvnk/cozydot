use crate::platform::{Architecture, Platform};
use anyhow::{bail, Context, Result};
use serde::{de, Deserialize, Deserializer};
use std::{collections::HashSet, fmt, fs, path::Path};
use url::{Host, Url};
use yaml_rust2::{
    parser::{Event, MarkedEventReceiver, Parser},
    scanner::{Marker, Scanner, TokenType},
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigV1 {
    #[serde(deserialize_with = "deserialize_schema")]
    pub schema: u8,
    pub system: Option<System>,
    pub packages: Option<Packages>,
    pub tools: Option<Tools>,
    pub fonts: Option<Fonts>,
    pub dotfiles: Option<Dotfiles>,
    pub integrations: Option<Integrations>,
    pub desktop: Option<Desktop>,
    pub updates: Option<Updates>,
}

impl ConfigV1 {
    pub fn parse(text: &str) -> Result<Self> {
        reject_yaml_extensions(text)?;
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
        if let Some(configured) = self
            .system
            .as_ref()
            .and_then(|system| system.distro.as_ref())
            .and_then(Distro::configured_id)
        {
            if configured != platform.distro {
                bail!(
                    "system.distro: configured distribution {configured:?} does not match resolved platform distribution {:?}",
                    platform.distro
                );
            }
        }
        if let Some(configured) = self
            .system
            .as_ref()
            .and_then(|system| system.desktop.as_ref())
            .and_then(DesktopKind::configured_id)
        {
            if configured != platform.desktop {
                bail!(
                    "system.desktop: configured desktop {configured:?} does not match resolved platform desktop {:?}",
                    platform.desktop
                );
            }
        }
        let upstream = upstream_for_distro(&platform.distro)?;
        let detected_upstream = upstream_from_id(&platform.upstream)?;
        if upstream != detected_upstream {
            bail!(
                "system.distro: platform distro {:?} does not belong to upstream family {:?}",
                platform.distro,
                platform.upstream
            );
        }
        if let Some(apt) = self.system.as_ref().and_then(|system| system.apt.as_ref()) {
            apt.validate(Some(upstream))?;
        }
        if let Some(packages) = &self.packages {
            packages.validate_repository_urls_for_distro(&platform.distro)?;
            packages.validate_native_selectors(platform.architecture)?;
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if let Some(system) = &self.system {
            system.validate()?;
        }
        if let Some(packages) = &self.packages {
            let distro = self
                .system
                .as_ref()
                .and_then(|system| system.distro.as_ref())
                .and_then(Distro::configured_id);
            packages.validate(distro)?;
        }
        if let Some(tools) = &self.tools {
            tools.validate()?;
        }
        if self
            .packages
            .as_ref()
            .and_then(|packages| packages.npm.as_ref())
            .is_some_and(|packages| !packages.is_empty())
            && self
                .tools
                .as_ref()
                .and_then(|tools| tools.node.as_ref())
                .is_none()
        {
            bail!("packages.npm: requires tools.node");
        }
        if let Some(fonts) = &self.fonts {
            validate_unique_strings(fonts.nerd.as_deref(), "fonts.nerd")?;
        }
        if let Some(dotfiles) = &self.dotfiles {
            validate_required_unique_strings(&dotfiles.packages, "dotfiles.packages")?;
            for (index, package) in dotfiles.packages.iter().enumerate() {
                validate_directory_name(package, &format!("dotfiles.packages[{index}]"))?;
            }
        }
        if let Some(integrations) = &self.integrations {
            integrations.validate()?;
        }
        if let Some(desktop) = &self.desktop {
            desktop.validate()?;
        }
        Ok(())
    }
}

fn deserialize_schema<'de, D>(deserializer: D) -> std::result::Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    struct SchemaVisitor;

    impl de::Visitor<'_> for SchemaVisitor {
        type Value = u8;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(
                "integer 1 (legacy configurations must be rewritten; run 'cozydot init' to re-initialize)",
            )
        }

        fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value == 1 {
                Ok(1)
            } else {
                Err(E::custom(format!(
                    "unsupported schema version {value}; only schema 1 is supported; rewrite legacy configurations or run 'cozydot init'"
                )))
            }
        }

        fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value == 1 {
                Ok(1)
            } else {
                Err(E::custom(format!(
                    "unsupported schema version {value}; only schema 1 is supported; rewrite legacy configurations or run 'cozydot init'"
                )))
            }
        }
    }

    deserializer.deserialize_any(SchemaVisitor)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Distro {
    Auto,
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
    fn upstream(&self) -> Option<Upstream> {
        match self {
            Self::Auto => None,
            Self::Ubuntu | Self::Linuxmint | Self::Pop | Self::Zorin | Self::Deepin => {
                Some(Upstream::Ubuntu)
            }
            Self::Debian | Self::Kali | Self::Tails => Some(Upstream::Debian),
        }
    }

    fn configured_id(&self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Ubuntu => Some("ubuntu"),
            Self::Linuxmint => Some("linuxmint"),
            Self::Pop => Some("pop"),
            Self::Zorin => Some("zorin"),
            Self::Deepin => Some("deepin"),
            Self::Debian => Some("debian"),
            Self::Kali => Some("kali"),
            Self::Tails => Some("tails"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Upstream {
    Ubuntu,
    Debian,
}

fn upstream_for_distro(distro: &str) -> Result<Upstream> {
    match distro {
        "ubuntu" | "linuxmint" | "pop" | "zorin" | "deepin" => Ok(Upstream::Ubuntu),
        "debian" | "kali" | "tails" => Ok(Upstream::Debian),
        _ => bail!(
            "system.distro: unsupported detected distribution {distro:?}; supported distributions are ubuntu, linuxmint, pop, zorin, deepin, debian, kali, and tails"
        ),
    }
}

fn upstream_from_id(upstream: &str) -> Result<Upstream> {
    match upstream {
        "ubuntu" => Ok(Upstream::Ubuntu),
        "debian" => Ok(Upstream::Debian),
        _ => bail!(
            "system.distro: unsupported platform upstream family {upstream:?}; supported families are ubuntu and debian"
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopKind {
    Auto,
    None,
    Gnome,
    Cinnamon,
}

impl DesktopKind {
    fn configured_id(&self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::None => Some("none"),
            Self::Gnome => Some("gnome"),
            Self::Cinnamon => Some("cinnamon"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub distro: Option<Distro>,
    pub desktop: Option<DesktopKind>,
    pub ensure_admin: Option<bool>,
    pub apt: Option<SystemApt>,
    pub ubuntu: Option<UbuntuSystem>,
}

impl System {
    fn validate(&self) -> Result<()> {
        if let Some(apt) = &self.apt {
            apt.validate(self.distro.as_ref().and_then(Distro::upstream))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AptSources {
    Preserve,
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AptComponent {
    Main,
    Contrib,
    NonFree,
    NonFreeFirmware,
    Restricted,
    Universe,
    Multiverse,
}

impl AptComponent {
    fn name(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Contrib => "contrib",
            Self::NonFree => "non-free",
            Self::NonFreeFirmware => "non-free-firmware",
            Self::Restricted => "restricted",
            Self::Universe => "universe",
            Self::Multiverse => "multiverse",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemApt {
    pub sources: Option<AptSources>,
    pub components: Option<Vec<AptComponent>>,
    pub unattended_upgrades: Option<bool>,
}

impl SystemApt {
    fn validate(&self, upstream: Option<Upstream>) -> Result<()> {
        let Some(components) = &self.components else {
            return Ok(());
        };
        if !matches!(self.sources, Some(AptSources::Managed)) {
            bail!("system.apt.components: valid only with system.apt.sources: managed");
        }
        if components.is_empty() {
            bail!("system.apt.components: must be a non-empty sequence");
        }
        validate_unique_by(components, "system.apt.components", AptComponent::name)?;
        let supported: &[&str] = match upstream {
            Some(Upstream::Ubuntu) => &["main", "restricted", "universe", "multiverse"],
            Some(Upstream::Debian) => &["main", "contrib", "non-free", "non-free-firmware"],
            None => return Ok(()),
        };
        for (index, component) in components.iter().enumerate() {
            if !supported.contains(&component.name()) {
                bail!(
                    "system.apt.components[{index}]: component {:?} is unsupported by the configured distribution family",
                    component.name()
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UbuntuSystem {
    pub snap: Option<bool>,
    pub codecs: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub remove: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub apt: Option<Vec<String>>,
    pub repositories: Option<Vec<Repository>>,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub flatpak: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub cargo: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub npm: Option<Vec<String>>,
    pub direct: Option<Vec<DirectPackage>>,
}

impl Packages {
    fn validate(&self, distro: Option<&str>) -> Result<()> {
        validate_package_list(
            self.remove.as_deref(),
            "packages.remove",
            validate_debian_package,
        )?;
        validate_package_list(self.apt.as_deref(), "packages.apt", validate_debian_package)?;
        validate_package_list(
            self.flatpak.as_deref(),
            "packages.flatpak",
            validate_flatpak_id,
        )?;
        validate_package_list(
            self.cargo.as_deref(),
            "packages.cargo",
            validate_cargo_package,
        )?;
        validate_package_list(self.npm.as_deref(), "packages.npm", validate_npm_package)?;
        self.validate_repositories()?;
        if let Some(distro) = distro {
            self.validate_repository_urls_for_distro(distro)?;
        }
        self.validate_direct()?;
        Ok(())
    }

    fn validate_repository_urls_for_distro(&self, distro: &str) -> Result<()> {
        for (index, repository) in self.repositories.iter().flatten().enumerate() {
            repository
                .source
                .urls
                .select(distro)
                .with_context(|| format!("packages.repositories[{index}].source.urls"))?;
        }
        Ok(())
    }

    fn validate_repositories(&self) -> Result<()> {
        let Some(repositories) = &self.repositories else {
            return Ok(());
        };
        let mut names = HashSet::new();
        let mut stems = HashSet::new();
        for (index, repository) in repositories.iter().enumerate() {
            let path = format!("packages.repositories[{index}]");
            repository.validate(&path)?;
            if !names.insert(repository.name.as_str()) {
                bail!(
                    "{path}.name: duplicate repository name {:?}",
                    repository.name
                );
            }
            let stem = repository.sanitized_name();
            if stem.is_empty() {
                bail!("{path}.name: produces an empty repository filename stem");
            }
            if !stems.insert(stem.clone()) {
                bail!("{path}.name: sanitized repository filename stem {stem:?} collides with an earlier repository");
            }
        }
        Ok(())
    }

    fn validate_direct(&self) -> Result<()> {
        let Some(packages) = &self.direct else {
            return Ok(());
        };
        let mut names = HashSet::new();
        for (index, package) in packages.iter().enumerate() {
            let path = format!("packages.direct[{index}]");
            package.validate(&path)?;
            if !names.insert(package.name.as_str()) {
                bail!(
                    "{path}.name: duplicate direct-package name {:?}",
                    package.name
                );
            }
        }
        Ok(())
    }

    fn validate_native_selectors(&self, architecture: Architecture) -> Result<()> {
        for (index, package) in self.direct.iter().flatten().enumerate() {
            if package.source.assets.get(architecture).is_none() {
                bail!(
                    "packages.direct[{index}].source.assets.{}: missing native asset selector for package {:?}",
                    architecture.canonical(),
                    package.name
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    #[serde(deserialize_with = "deserialize_string")]
    pub name: String,
    pub key: HttpsUrl,
    pub source: RepositorySource,
    #[serde(deserialize_with = "deserialize_strings")]
    pub packages: Vec<String>,
}

impl Repository {
    pub fn sanitized_name(&self) -> String {
        let mut stem = String::new();
        let mut separator = false;
        for byte in self.name.bytes() {
            if byte.is_ascii_alphanumeric() {
                if separator && !stem.is_empty() {
                    stem.push('-');
                }
                stem.push((byte as char).to_ascii_lowercase());
                separator = false;
            } else {
                separator = true;
            }
        }
        stem
    }

    fn validate(&self, path: &str) -> Result<()> {
        validate_literal(&self.name, &format!("{path}.name"))?;
        self.source.validate(&format!("{path}.source"))?;
        validate_required_unique_strings(&self.packages, &format!("{path}.packages"))?;
        for (index, package) in self.packages.iter().enumerate() {
            validate_debian_package(package, &format!("{path}.packages[{index}]"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySource {
    pub urls: RepositoryUrls,
    pub suite: ConfiguredRepositorySuite,
    pub components: Vec<AptSourceToken>,
}

impl RepositorySource {
    fn validate(&self, path: &str) -> Result<()> {
        self.urls.validate(&format!("{path}.urls"))?;
        validate_required_unique_apt_source_tokens(&self.components, &format!("{path}.components"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AptSourceToken(String);

impl AptSourceToken {
    pub fn parse(value: &str) -> Result<Self> {
        let mut bytes = value.bytes();
        let valid = bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'+' | b'-')
            });
        if !valid {
            bail!(
                "must be one lowercase APT source token starting with a letter or digit and containing only letters, digits, '.', '_', '+', or '-'"
            );
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AptSourceToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for AptSourceToken {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'de> Deserialize<'de> for AptSourceToken {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_string(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfiguredRepositorySuite {
    System,
    Fixed(AptSourceToken),
}

impl<'de> Deserialize<'de> for ConfiguredRepositorySuite {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_string(deserializer)?;
        if value == "system" {
            Ok(Self::System)
        } else {
            AptSourceToken::parse(&value)
                .map(Self::Fixed)
                .map_err(de::Error::custom)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryUrls {
    #[serde(default)]
    pub default: Option<HttpsUrl>,
    #[serde(default)]
    pub ubuntu: Option<HttpsUrl>,
    #[serde(default)]
    pub linuxmint: Option<HttpsUrl>,
    #[serde(default)]
    pub pop: Option<HttpsUrl>,
    #[serde(default)]
    pub zorin: Option<HttpsUrl>,
    #[serde(default)]
    pub deepin: Option<HttpsUrl>,
    #[serde(default)]
    pub debian: Option<HttpsUrl>,
    #[serde(default)]
    pub kali: Option<HttpsUrl>,
    #[serde(default)]
    pub tails: Option<HttpsUrl>,
}

impl RepositoryUrls {
    pub fn select(&self, distro: &str) -> Result<&str> {
        self.select_url(distro).map(HttpsUrl::as_str)
    }

    pub fn select_url(&self, distro: &str) -> Result<&HttpsUrl> {
        let selected = match distro {
            "ubuntu" => self.ubuntu.as_ref(),
            "linuxmint" => self.linuxmint.as_ref(),
            "pop" => self.pop.as_ref(),
            "zorin" => self.zorin.as_ref(),
            "deepin" => self.deepin.as_ref(),
            "debian" => self.debian.as_ref(),
            "kali" => self.kali.as_ref(),
            "tails" => self.tails.as_ref(),
            _ => bail!("repository source URL selection: unsupported distro {distro:?}"),
        };
        selected.or(self.default.as_ref()).ok_or_else(|| {
            anyhow::anyhow!(
                "repository source URL selection: no URL for distro {distro:?} and no default URL"
            )
        })
    }

    fn validate(&self, path: &str) -> Result<()> {
        let urls = [
            self.default.as_ref(),
            self.ubuntu.as_ref(),
            self.linuxmint.as_ref(),
            self.pop.as_ref(),
            self.zorin.as_ref(),
            self.deepin.as_ref(),
            self.debian.as_ref(),
            self.kali.as_ref(),
            self.tails.as_ref(),
        ];
        if urls.iter().all(|value| value.is_none()) {
            bail!("{path}: must contain default and/or a supported distro key");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsUrl(Url);

impl HttpsUrl {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    fn parse(value: &str) -> Result<Self> {
        validate_non_empty(value, "URL")?;
        if value.chars().any(char::is_whitespace)
            || value.chars().any(char::is_control)
            || value.contains('\\')
            || has_substitution(value)
        {
            bail!("must be a literal HTTPS URL without whitespace or substitutions");
        }
        let parsed = Url::parse(value).context("must be a valid absolute HTTPS URL")?;
        let (raw_scheme, remainder) = value.split_once("://").unwrap_or_default();
        let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
        let host_port = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
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
            bail!("must use HTTPS with a non-empty host and no credentials or fragment");
        }
        Ok(Self(parsed))
    }
}

impl<'de> Deserialize<'de> for HttpsUrl {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_string(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DirectFormat {
    Deb,
    Appimage,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectPackage {
    #[serde(deserialize_with = "deserialize_string")]
    pub name: String,
    pub format: DirectFormat,
    #[serde(deserialize_with = "deserialize_strings")]
    pub provides: Vec<String>,
    pub source: GithubSource,
}

impl DirectPackage {
    fn validate(&self, path: &str) -> Result<()> {
        validate_definition_name(&self.name, &format!("{path}.name"))?;
        validate_required_unique_strings(&self.provides, &format!("{path}.provides"))?;
        for (index, executable) in self.provides.iter().enumerate() {
            validate_executable(executable, &format!("{path}.provides[{index}]"))?;
        }
        self.source.validate(&format!("{path}.source"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubSource {
    #[serde(rename = "type")]
    pub kind: GithubSourceType,
    #[serde(deserialize_with = "deserialize_string")]
    pub repository: String,
    pub assets: AssetSelectors,
}

impl GithubSource {
    fn validate(&self, path: &str) -> Result<()> {
        validate_github_repository(&self.repository, &format!("{path}.repository"))?;
        self.assets.validate(&format!("{path}.assets"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GithubSourceType {
    Github,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSelectors {
    pub amd64: Option<AssetSelector>,
    pub arm64: Option<AssetSelector>,
    pub arm32: Option<AssetSelector>,
    pub riscv64: Option<AssetSelector>,
}

impl AssetSelectors {
    pub fn get(&self, architecture: Architecture) -> Option<&AssetSelector> {
        match architecture {
            Architecture::Amd64 => self.amd64.as_ref(),
            Architecture::Arm64 => self.arm64.as_ref(),
            Architecture::Arm32 => self.arm32.as_ref(),
            Architecture::Riscv64 => self.riscv64.as_ref(),
        }
    }

    fn validate(&self, path: &str) -> Result<()> {
        let selectors = [
            ("amd64", self.amd64.as_ref()),
            ("arm64", self.arm64.as_ref()),
            ("arm32", self.arm32.as_ref()),
            ("riscv64", self.riscv64.as_ref()),
        ];
        if selectors.iter().all(|(_, selector)| selector.is_none()) {
            bail!("{path}: must contain at least one canonical architecture selector");
        }
        for (architecture, selector) in selectors {
            if let Some(selector) = selector {
                selector.validate(&format!("{path}.{architecture}"))?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSelector {
    #[serde(deserialize_with = "deserialize_string")]
    pub include: String,
    #[serde(deserialize_with = "deserialize_strings")]
    pub exclude: Vec<String>,
}

impl AssetSelector {
    fn validate(&self, path: &str) -> Result<()> {
        validate_wildcard(&self.include, &format!("{path}.include"))?;
        for (index, pattern) in self.exclude.iter().enumerate() {
            validate_wildcard(pattern, &format!("{path}.exclude[{index}]"))?;
        }
        validate_unique_strings(Some(&self.exclude), &format!("{path}.exclude"))
    }
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
        if let Some(version) = &self.rust {
            validate_rust_version(version, "tools.rust")?;
        }
        if let Some(version) = &self.go {
            if version != "latest" {
                validate_numeric_version(version, "tools.go", 2, 3)?;
            }
        }
        if let Some(version) = &self.node {
            if version != "latest" && version != "lts" {
                validate_numeric_version(version, "tools.node", 1, 3)?;
            }
        }
        if let Some(version) = &self.python {
            validate_numeric_version(version, "tools.python", 2, 3)?;
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dotfiles {
    #[serde(deserialize_with = "deserialize_strings")]
    pub packages: Vec<String>,
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
        if let Some(docker) = &self.docker {
            docker.validate()?;
        }
        if let Some(vscode) = &self.vscode {
            validate_unique_strings(
                vscode.extensions.as_deref(),
                "integrations.vscode.extensions",
            )?;
            for (index, extension) in vscode.extensions.iter().flatten().enumerate() {
                validate_vscode_extension(
                    extension,
                    &format!("integrations.vscode.extensions[{index}]"),
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerIntegration {
    pub add_user_to_group: Option<bool>,
    pub local_log_driver: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub max_log_size: Option<String>,
}

impl DockerIntegration {
    fn validate(&self) -> Result<()> {
        if let Some(size) = &self.max_log_size {
            if self.local_log_driver != Some(true) {
                bail!("integrations.docker.max_log_size: requires integrations.docker.local_log_driver: true");
            }
            validate_positive_size(size, "integrations.docker.max_log_size")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualBoxIntegration {
    pub add_user_to_group: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VsCodeIntegration {
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub extensions: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
        if let Some(terminal) = &self.terminal {
            validate_executable(terminal, "desktop.terminal")?;
        }
        if let Some(idle) = &self.idle {
            if let Some(timeout) = &idle.timeout {
                validate_duration(timeout, "desktop.idle.timeout")?;
            }
        }
        if let Some(gnome) = &self.gnome {
            validate_unique_strings(gnome.extensions.as_deref(), "desktop.gnome.extensions")?;
            for (index, extension) in gnome.extensions.iter().flatten().enumerate() {
                validate_gnome_uuid(extension, &format!("desktop.gnome.extensions[{index}]"))?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Idle {
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub timeout: Option<String>,
    pub dim: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gnome {
    #[serde(default, deserialize_with = "deserialize_optional_strings")]
    pub extensions: Option<Vec<String>>,
    pub dock: Option<bool>,
    pub rounded_corners: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Updates {
    pub apt: Option<AptUpdate>,
    pub flatpak: Option<bool>,
    pub tools: Option<ToolUpdates>,
    pub packages: Option<PackageUpdates>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AptUpdate {
    Off,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageUpdates {
    pub cargo: Option<bool>,
    pub npm: Option<bool>,
    pub direct: Option<bool>,
}

fn validate_non_empty(value: &str, path: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{path}: must be a non-empty string");
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
    struct OptionalStringVisitor;

    impl<'de> de::Visitor<'de> for OptionalStringVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a YAML string or null")
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserialize_string(deserializer).map(Some)
        }
    }

    deserializer.deserialize_option(OptionalStringVisitor)
}

fn deserialize_strings<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringsVisitor;

    impl<'de> de::Visitor<'de> for StringsVisitor {
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

    deserializer.deserialize_seq(StringsVisitor)
}

fn deserialize_optional_strings<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionalStringsVisitor;

    impl<'de> de::Visitor<'de> for OptionalStringsVisitor {
        type Value = Option<Vec<String>>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a sequence of YAML strings or null")
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserialize_strings(deserializer).map(Some)
        }
    }

    deserializer.deserialize_option(OptionalStringsVisitor)
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
                "line {}, column {}: {extension} are not supported by schema v1",
                token.0.line() + 1,
                token.0.col() + 1
            );
        }
    }

    #[derive(Default)]
    struct ExtensionReceiver {
        documents: usize,
        error: Option<anyhow::Error>,
    }

    impl MarkedEventReceiver for ExtensionReceiver {
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
                self.error = Some(anyhow::anyhow!(
                    "line {}, column {}: {extension} are not supported by schema v1",
                    marker.line() + 1,
                    marker.col() + 1
                ));
            }
        }
    }

    let mut receiver = ExtensionReceiver::default();
    Parser::new_from_str(text)
        .load(&mut receiver, true)
        .context("parse YAML extension preflight")?;
    if let Some(error) = receiver.error {
        return Err(error);
    }
    Ok(())
}

fn validate_literal(value: &str, path: &str) -> Result<()> {
    validate_non_empty(value, path)?;
    if value.contains(['\n', '\r']) || has_substitution(value) {
        bail!("{path}: must be a literal value without interpolation or substitution");
    }
    Ok(())
}

fn validate_unique_strings(values: Option<&[String]>, path: &str) -> Result<()> {
    let Some(values) = values else {
        return Ok(());
    };
    let mut seen = HashSet::new();
    for (index, value) in values.iter().enumerate() {
        validate_literal(value, &format!("{path}[{index}]"))?;
        if !seen.insert(value.as_str()) {
            bail!("{path}[{index}]: duplicate value {value:?}");
        }
    }
    Ok(())
}

fn validate_required_unique_strings(values: &[String], path: &str) -> Result<()> {
    if values.is_empty() {
        bail!("{path}: must be a non-empty sequence");
    }
    validate_unique_strings(Some(values), path)
}

fn validate_required_unique_apt_source_tokens(values: &[AptSourceToken], path: &str) -> Result<()> {
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

fn validate_unique_by<T, F>(values: &[T], path: &str, key: F) -> Result<()>
where
    F: Fn(&T) -> &'static str,
{
    let mut seen = HashSet::new();
    for (index, value) in values.iter().enumerate() {
        let value = key(value);
        if !seen.insert(value) {
            bail!("{path}[{index}]: duplicate value {value:?}");
        }
    }
    Ok(())
}

fn valid_domain_host(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            label.len() <= 63
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn validate_github_repository(value: &str, path: &str) -> Result<()> {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || !valid_github_owner(owner)
        || !valid_github_repository_name(repository)
    {
        bail!("{path}: must be an owner/repository coordinate");
    }
    Ok(())
}

fn valid_github_owner(value: &str) -> bool {
    !value.is_empty()
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn valid_github_repository_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        && value
            .bytes()
            .any(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

fn validate_wildcard(value: &str, path: &str) -> Result<()> {
    validate_non_empty(value, path)?;
    if !value.contains(['*', '?'])
        || value.contains(['/', '\\', '[', ']', '{', '}', '$', '(', ')', '`'])
        || value.chars().any(char::is_control)
        || has_substitution(value)
    {
        bail!("{path}: must be an anchored filename wildcard using only '*' and '?' operators, without paths or substitutions");
    }
    Ok(())
}

fn validate_rust_version(value: &str, path: &str) -> Result<()> {
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
    let Ok(year) = parts[0].parse::<u16>() else {
        return false;
    };
    let Ok(month) = parts[1].parse::<u8>() else {
        return false;
    };
    let Ok(day) = parts[2].parse::<u8>() else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    year != 0 && (1..=days).contains(&day)
}

fn validate_numeric_version(
    value: &str,
    path: &str,
    min_parts: usize,
    max_parts: usize,
) -> Result<()> {
    let parts = value.split('.').collect::<Vec<_>>();
    if !(min_parts..=max_parts).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        bail!("{path}: has an invalid version; expected {min_parts} to {max_parts} numeric components");
    }
    Ok(())
}

fn validate_positive_size(value: &str, path: &str) -> Result<()> {
    let number = value
        .strip_suffix('k')
        .or_else(|| value.strip_suffix('m'))
        .or_else(|| value.strip_suffix('g'));
    if number.is_none_or(|number| {
        number.is_empty()
            || !number.bytes().all(|byte| byte.is_ascii_digit())
            || number.bytes().all(|byte| byte == b'0')
    }) {
        bail!("{path}: must be a positive integer followed by k, m, or g");
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
        bail!("{path}: must be a non-negative integer followed by exactly one of s, m, or h");
    }
    Ok(())
}

fn validate_executable(value: &str, path: &str) -> Result<()> {
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        bail!("{path}: must start with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', '+', or '-'");
    }
    Ok(())
}

fn validate_definition_name(value: &str, path: &str) -> Result<()> {
    validate_literal(value, path)?;
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        bail!("{path}: must start with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', or '-'");
    }
    Ok(())
}

fn validate_package_list(
    values: Option<&[String]>,
    path: &str,
    validate_entry: fn(&str, &str) -> Result<()>,
) -> Result<()> {
    validate_unique_strings(values, path)?;
    for (index, value) in values.into_iter().flatten().enumerate() {
        validate_entry(value, &format!("{path}[{index}]"))?;
    }
    Ok(())
}

fn validate_debian_package(value: &str, path: &str) -> Result<()> {
    validate_literal(value, path)?;
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit())
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"+.-".contains(&byte)
        })
    {
        bail!("{path}: must be a Debian package name starting with a lowercase ASCII letter or digit and containing only lowercase ASCII letters, digits, '+', '.', or '-'");
    }
    Ok(())
}

fn validate_cargo_package(value: &str, path: &str) -> Result<()> {
    validate_literal(value, path)?;
    let mut bytes = value.bytes();
    if bytes
        .next()
        .is_none_or(|byte| !byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        bail!("{path}: must be an unversioned Cargo package name containing only ASCII alphanumerics, '_', or '-' and starting alphanumeric");
    }
    Ok(())
}

fn validate_npm_package(value: &str, path: &str) -> Result<()> {
    validate_literal(value, path)?;
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
        bail!("{path}: must be an unversioned lowercase npm name or @scope/name");
    }
    Ok(())
}

fn validate_flatpak_id(value: &str, path: &str) -> Result<()> {
    validate_literal(value, path)?;
    let segments = value.split('.').collect::<Vec<_>>();
    let valid_segment = |segment: &&str| {
        let mut bytes = segment.bytes();
        bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    };
    if segments.len() < 3 || !segments.iter().all(valid_segment) {
        bail!("{path}: must be a canonical Flatpak ID with at least three dot-separated ASCII identifier segments");
    }
    Ok(())
}

fn validate_directory_name(value: &str, path: &str) -> Result<()> {
    validate_literal(value, path)?;
    if matches!(value, "." | "..") || value.contains(['/', '\\']) {
        bail!("{path}: must be one directory name below the dotfiles root, not a path");
    }
    Ok(())
}

fn validate_vscode_extension(value: &str, path: &str) -> Result<()> {
    let mut parts = value.split('.');
    if !valid_identifier(parts.next().unwrap_or_default())
        || !valid_identifier(parts.next().unwrap_or_default())
        || parts.next().is_some()
    {
        bail!("{path}: must be a publisher.extension identifier");
    }
    Ok(())
}

fn validate_gnome_uuid(value: &str, path: &str) -> Result<()> {
    let mut parts = value.split('@');
    if !valid_identifier(parts.next().unwrap_or_default())
        || !valid_identifier(parts.next().unwrap_or_default())
        || parts.next().is_some()
    {
        bail!("{path}: must be an exact GNOME extension UUID containing one '@'");
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_ .".contains(&byte))
        && !value.contains(' ')
}

fn has_substitution(value: &str) -> bool {
    value.contains('$') || value.contains("{{") || value.contains("{%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_stem_uses_contract_sanitization() {
        let repository = Repository {
            name: "--GitHub__CLI!!".into(),
            key: HttpsUrl::parse("https://example.com/key").unwrap(),
            source: RepositorySource {
                urls: RepositoryUrls {
                    default: Some(HttpsUrl::parse("https://example.com/repo").unwrap()),
                    ubuntu: None,
                    linuxmint: None,
                    pop: None,
                    zorin: None,
                    deepin: None,
                    debian: None,
                    kali: None,
                    tails: None,
                },
                suite: ConfiguredRepositorySuite::Fixed(AptSourceToken::parse("stable").unwrap()),
                components: vec![AptSourceToken::parse("main").unwrap()],
            },
            packages: vec!["gh".into()],
        };
        assert_eq!(repository.sanitized_name(), "github-cli");
    }

    #[test]
    fn distro_url_precedes_default() {
        let urls = RepositoryUrls {
            default: Some(HttpsUrl::parse("https://default.example").unwrap()),
            ubuntu: Some(HttpsUrl::parse("https://ubuntu.example").unwrap()),
            linuxmint: None,
            pop: None,
            zorin: None,
            deepin: None,
            debian: None,
            kali: None,
            tails: None,
        };
        assert_eq!(urls.select("ubuntu").unwrap(), "https://ubuntu.example/");
        assert_eq!(urls.select("debian").unwrap(), "https://default.example/");
    }
}
