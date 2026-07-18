use super::*;

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
    pub(super) fn validate(&self) -> Result<()> {
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
    pub(super) fn as_str(self) -> &'static str {
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

pub(super) fn select_distro_map<T>(
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

    pub(super) fn validate_for_platform(
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
                resolved
                    .suite
                    .as_ref()
                    .context("system repository suite did not resolve")?
                    .as_str(),
                &format!("packages.apt.repositories[{index}].suite resolved codename"),
            )?;
        }
        Ok(())
    }

    pub(crate) fn resolve_for_platform(
        &self,
        index: usize,
        platform: &Platform,
        identity: PlatformIdentity,
    ) -> Result<ResolvedRepository<'_>> {
        let (key, source_url) = select_distro_map(&self.urls, identity.distro, identity.upstream)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "packages.apt.repositories[{index}].urls: no URL for distribution {:?}, upstream {:?}, or default",
                    platform.distro,
                    platform.upstream
                )
            })?;
        let suite = match self.suite.as_deref() {
            Some("system") => {
                let codename = selected_repository_codename(key, platform, identity.distro)
                    .ok_or_else(|| anyhow::anyhow!("packages.apt.repositories[{index}].suite: system cannot use a default URL because it has no repository-family codename"))?;
                Some(AptToken(codename.to_owned()))
            }
            Some(value) => Some(AptToken(value.to_owned())),
            None => None,
        };
        Ok(ResolvedRepository { source_url, suite })
    }
}

pub(crate) struct ResolvedRepository<'a> {
    pub source_url: &'a HttpsUrl,
    pub suite: Option<AptToken>,
}

fn selected_repository_codename(
    key: DistroMapKey,
    platform: &Platform,
    distro: Distro,
) -> Option<&str> {
    if key == DistroMapKey::Default || key == DistroMapKey::from_distro(distro) {
        Some(&platform.distro_codename)
    } else {
        Some(&platform.base_codename)
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
        urls: Box<ArchitectureUrls>,
        sha256: Box<ArchitectureHashes>,
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

    pub(super) fn require_architecture(&self, architecture: Architecture) -> Result<()> {
        self.resolve_native(architecture).map(|_| ())
    }

    pub(super) fn is_github(&self) -> bool {
        matches!(self, Self::Github { .. })
    }

    pub(crate) fn resolve_native(
        &self,
        architecture: Architecture,
    ) -> Result<ResolvedNativeBinary<'_>> {
        match self {
            Self::Github { repository, assets } => Ok(ResolvedNativeBinary::Github {
                repository,
                selector: assets
                    .get(architecture)
                    .ok_or_else(|| anyhow::anyhow!("missing native architecture selector"))?,
            }),
            Self::Url { urls, sha256 } => Ok(ResolvedNativeBinary::Url {
                url: urls
                    .get(architecture)
                    .ok_or_else(|| anyhow::anyhow!("missing native architecture selector"))?,
                sha256: sha256
                    .get(architecture)
                    .ok_or_else(|| anyhow::anyhow!("missing native architecture selector"))?,
            }),
        }
    }
}

pub(crate) enum ResolvedNativeBinary<'a> {
    Github {
        repository: &'a str,
        selector: &'a str,
    },
    Url {
        url: &'a HttpsUrl,
        sha256: &'a Sha256,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetMap {
    pub amd64: Option<String>,
    pub arm64: Option<String>,
    pub arm32: Option<String>,
    pub riscv64: Option<String>,
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

    fn values(&self) -> [(&'static str, Option<&String>); 4] {
        [
            ("amd64", self.amd64.as_ref()),
            ("arm64", self.arm64.as_ref()),
            ("arm32", self.arm32.as_ref()),
            ("riscv64", self.riscv64.as_ref()),
        ]
    }

    fn get(&self, architecture: Architecture) -> Option<&str> {
        match architecture {
            Architecture::Amd64 => self.amd64.as_deref(),
            Architecture::Arm64 => self.arm64.as_deref(),
            Architecture::Arm32 => self.arm32.as_deref(),
            Architecture::Riscv64 => self.riscv64.as_deref(),
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

    fn get(&self, architecture: Architecture) -> Option<&HttpsUrl> {
        match architecture {
            Architecture::Amd64 => self.amd64.as_ref(),
            Architecture::Arm64 => self.arm64.as_ref(),
            Architecture::Arm32 => self.arm32.as_ref(),
            Architecture::Riscv64 => self.riscv64.as_ref(),
        }
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

    fn get(&self, architecture: Architecture) -> Option<&Sha256> {
        match architecture {
            Architecture::Amd64 => self.amd64.as_ref(),
            Architecture::Arm64 => self.arm64.as_ref(),
            Architecture::Arm32 => self.arm32.as_ref(),
            Architecture::Riscv64 => self.riscv64.as_ref(),
        }
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
