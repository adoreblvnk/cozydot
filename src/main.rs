use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};

mod json_helpers {
    use anyhow::{bail, Context, Result};
    use serde_json::Value;

    pub fn latest_go(input: &str, requested: &str, arch: &str) -> Result<(String, String, String)> {
        let value: Value = serde_json::from_str(input).context("parse Go release JSON")?;
        let releases = value.as_array().context("Go metadata must be an array")?;
        let version = releases
            .iter()
            .filter_map(|release| release["version"].as_str())
            .filter(|version| stable_go_version(version))
            .map(|version| version.trim_start_matches("go"))
            .find(|version| {
                requested == "latest"
                    || *version == requested
                    || version
                        .strip_prefix(requested)
                        .is_some_and(|rest| rest.starts_with('.'))
            })
            .context("Go metadata has no matching stable release")?;
        let filename = format!("go{version}.linux-{arch}.tar.gz");
        let checksum = releases
            .iter()
            .find(|release| release["version"].as_str() == Some(&format!("go{version}")))
            .and_then(|release| release["files"].as_array())
            .and_then(|files| files.iter().find(|file| file["filename"].as_str() == Some(&filename)))
            .and_then(|file| file["sha256"].as_str())
            .context("Go metadata has no matching archive checksum")?;
        Ok((version.to_owned(), filename, checksum.to_owned()))
    }

    pub fn gnome_version(input: &str, shell_version: &str) -> Result<u64> {
        let value: Value = serde_json::from_str(input).context("parse GNOME extension JSON")?;
        let versions = value["shell_version_map"]
            .as_object()
            .context("GNOME response has no shell_version_map")?;
        let mut candidate = shell_version;
        loop {
            if let Some(version) = versions.get(candidate).and_then(|entry| entry["version"].as_u64()) {
                return Ok(version);
            }
            let Some((parent, _)) = candidate.rsplit_once('.') else {
                bail!("GNOME response has no extension version for shell {shell_version}");
            };
            candidate = parent;
        }
    }

    pub fn gnome_shell_version(input: &str) -> Result<String> {
        input
            .split_whitespace()
            .map(|part| part.trim_matches(|character: char| !character.is_ascii_digit() && character != '.'))
            .find(|part| {
                !part.is_empty()
                    && part
                        .split('.')
                        .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
            })
            .map(str::to_owned)
            .context("GNOME Shell version output has no numeric version")
    }

    fn stable_go_version(value: &str) -> bool {
        let Some(rest) = value.strip_prefix("go") else {
            return false;
        };
        let parts = rest.split('.').collect::<Vec<_>>();
        (parts.len() == 2 || parts.len() == 3)
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    }
}
mod operations;

#[derive(Debug, Parser)]
#[command(
    name = "cozydot",
    version,
    about = "Provision a Linux system from one active configuration"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Create or safely refresh the config and bundled dotfiles
    Init {
        /// Configuration preset to materialize
        #[arg(long, value_enum, default_value = "cozydot")]
        preset: init::Preset,
    },
    /// Apply the active configuration to this host
    Apply,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };
    match command {
        CliCommand::Init { preset } => {
            println!("Initialized cozydot in {}", init::run(preset)?.display());
        }
        CliCommand::Apply => apply()?,
    }
    Ok(())
}

fn apply() -> Result<()> {
    let root = init::config_root()?;
    let path = root.join("cozydot.yaml");
    let config =
        config::Config::load(&path).with_context(|| "active config is missing or invalid; run 'cozydot init' first")?;
    let platform = platform::Platform::detect()?;
    let steps = planner::plan(&config, &platform, &root.join("dotfiles"))?;
    let mut runner = runner::ProcessRunner {
        dry_run: std::env::var_os("COZYDOT_DRY_RUN").is_some(),
    };
    runner::execute(&mut runner, &steps)?;
    Ok(())
}

mod config {

    pub mod model {

        use crate::{
            domain::HttpsUrl,
            platform::{Architecture, Platform},
        };
        use anyhow::{bail, Context, Result};
        use regex::Regex;
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
                let identity = resolve_platform_identity(platform)?;
                let (distro, upstream) = (identity.distro, identity.upstream);
                let desktop = DesktopKind::from_platform(&platform.desktop)?;

                if let Some(require) = self.system.as_ref().and_then(|system| system.require.as_ref()) {
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
                    if configured.has_neutral_intent() && !matches!(desktop, DesktopKind::Gnome | DesktopKind::Cinnamon)
                    {
                        bail!(
                            "desktop: theme, terminal, and idle settings require GNOME or Cinnamon; detected {:?}",
                            platform.desktop
                        );
                    }
                    if configured.gnome.is_some() && !matches!(desktop, DesktopKind::Gnome | DesktopKind::Cinnamon) {
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
                    && self.tools.as_ref().and_then(|tools| tools.rust.as_ref()).is_none()
                {
                    bail!("packages.cargo: requires tools.rust");
                }
                if self
                    .packages
                    .as_ref()
                    .and_then(|packages| packages.npm.as_ref())
                    .is_some()
                    && self.tools.as_ref().and_then(|tools| tools.node.as_ref()).is_none()
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
        pub(crate) enum Family {
            Ubuntu,
            Debian,
        }

        #[derive(Debug, Clone, Copy)]
        pub(crate) struct PlatformIdentity {
            pub distro: Distro,
            pub upstream: Family,
        }

        pub(crate) fn resolve_platform_identity(platform: &Platform) -> Result<PlatformIdentity> {
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
                require_effective(self.distros.is_some() || self.desktops.is_some(), "system.require")?;
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

            fn validate_for_platform(&self, platform: &Platform, distro: Distro, upstream: Family) -> Result<()> {
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

            pub(crate) fn resolve_managed(
                &self,
                platform: &Platform,
                identity: PlatformIdentity,
            ) -> Result<Option<crate::platform::ManagedAptSources>> {
                if self.mode == SourceMode::Preserve {
                    return Ok(None);
                }
                let components = self
                    .components
                    .as_ref()
                    .context("managed APT sources require components")?;
                let (_, selected) =
                    select_distro_map(components, identity.distro, identity.upstream).ok_or_else(|| {
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

        impl UbuntuSystem {
            fn validate(&self) -> Result<()> {
                require_effective(self.snap.is_some() || self.codecs.is_some(), "system.ubuntu")
            }
        }

        mod packages {
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
                    validate_string_list(self.flatpak.as_deref(), "packages.flatpak", validate_flatpak_id)?;
                    validate_string_list(self.cargo.as_deref(), "packages.cargo", validate_cargo_package)?;
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
                                let command_path = format!("packages.binaries[{index}].commands[{command_index}]");
                                if let Some(owner_path) = command_owners.insert(command.as_str(), command_path.clone())
                                {
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
                    validate_string_list(self.remove.as_deref(), "packages.apt.remove", validate_debian_package)?;
                    validate_string_list(self.install.as_deref(), "packages.apt.install", validate_debian_package)?;
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
                                bail!(
                                    "packages.apt.repositories[{index}].name: filename stem {stem:?} collides with an earlier repository"
                                );
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
                            bail!(
                                "packages.apt.remove[{index}]: package {package:?} is also configured for installation"
                            );
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
                    validate_string_values(&self.packages, &format!("{path}.packages"), validate_debian_package)?;
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
                    let (key, source_url) = select_distro_map(&self.urls, identity.distro, identity.upstream).ok_or_else(|| {
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

            fn selected_repository_codename(key: DistroMapKey, platform: &Platform, distro: Distro) -> Option<&str> {
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
                    validate_string_values(&self.commands, &format!("{path}.commands"), validate_executable)?;
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
                                bail!("{path}: urls and sha256 must contain exactly the same architecture keys");
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

                pub(crate) fn resolve_native(&self, architecture: Architecture) -> Result<ResolvedNativeBinary<'_>> {
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
                Github { repository: &'a str, selector: &'a str },
                Url { url: &'a HttpsUrl, sha256: &'a Sha256 },
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
        }

        use packages::select_distro_map;
        pub(crate) use packages::ResolvedNativeBinary;
        pub use packages::{BinaryFormat, BinaryPackage, DistroMapKey, Packages, Repository};

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
                    self.rust.is_some() || self.go.is_some() || self.node.is_some() || self.python.is_some(),
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
                true_only(self.add_user_to_group, "integrations.docker.add_user_to_group")?;
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
                true_only(self.add_user_to_group, "integrations.virtualbox.add_user_to_group")
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
                validate_string_values(&self.extensions, "integrations.vscode.extensions", validate_vscode_id)
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
                    self.theme.is_some() || self.terminal.is_some() || self.idle.is_some() || self.gnome.is_some(),
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
                if self.fonts.is_some() && config.fonts.as_ref().and_then(|fonts| fonts.nerd.as_ref()).is_none() {
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
                if self.cargo.is_some() && packages.and_then(|packages| packages.cargo.as_ref()).is_none() {
                    bail!("updates.packages.cargo: requires configured packages.cargo targets");
                }
                if self.npm.is_some() && packages.and_then(|packages| packages.npm.as_ref()).is_none() {
                    bail!("updates.packages.npm: requires configured packages.npm targets");
                }
                if self.binaries.is_some()
                    && !packages
                        .and_then(|packages| packages.binaries.as_ref())
                        .is_some_and(|binaries| binaries.iter().any(|binary| binary.source.is_github()))
                {
                    bail!("updates.packages.binaries: requires at least one configured GitHub binary target");
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
                anyhow::anyhow!("version: missing required configuration version; only version \"1.0.0\" is supported")
            })?;
            match version {
                Value::String(value) if value == "1.0.0" => {}
                Value::String(value) => {
                    bail!("version: unsupported configuration version {value:?}; only version \"1.0.0\" is supported")
                }
                other => {
                    bail!("version: unsupported configuration version value {other:?}; expected YAML string \"1.0.0\"")
                }
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
                    TokenType::VersionDirective(..) | TokenType::TagDirective(..) => Some("YAML directives"),
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
                        self.error = Some(anyhow::anyhow!(
                            "line {}, column {}: {extension} are not supported by configuration version 1.0.0",
                            marker.line() + 1,
                            marker.col() + 1
                        ));
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

        fn deserialize_optional_string<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
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

        fn deserialize_optional_strings<'de, D>(deserializer: D) -> std::result::Result<Option<Vec<String>>, D::Error>
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
            let re = Regex::new(r"^[a-zA-Z0-9](?:[a-zA-Z0-9._-]*[a-zA-Z0-9])?$").unwrap();
            if !re.is_match(value) {
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
            let re = Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9._+-]*$").unwrap();
            if !re.is_match(value) {
                bail!("{path}: invalid executable basename {value:?}; must start with an ASCII alphanumeric and contain only ASCII alphanumerics, '.', '_', '+', or '-'");
            }
            Ok(())
        }

        fn validate_debian_package(value: &str, path: &str) -> Result<()> {
            let re = Regex::new(r"^[a-z0-9][a-z0-9+.-]*$").unwrap();
            if !re.is_match(value) {
                bail!("{path}: invalid Debian package name {value:?}; must be unversioned");
            }
            Ok(())
        }

        fn validate_cargo_package(value: &str, path: &str) -> Result<()> {
            let re = Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_-]*$").unwrap();
            if !re.is_match(value) {
                bail!("{path}: invalid Cargo package name {value:?}; must be unversioned");
            }
            Ok(())
        }

        fn validate_npm_package(value: &str, path: &str) -> Result<()> {
            let re = Regex::new(r"^(?:@[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*|[a-z0-9][a-z0-9._-]*)$").unwrap();
            if !re.is_match(value) {
                bail!(
                    "{path}: invalid npm package name {value:?}; must be an unversioned lowercase name or @scope/name"
                );
            }
            Ok(())
        }

        fn validate_flatpak_id(value: &str, path: &str) -> Result<()> {
            let re = Regex::new(r"^[a-zA-Z][a-zA-Z0-9_]*(?:\.[a-zA-Z][a-zA-Z0-9_]*){2,}$").unwrap();
            if !re.is_match(value) {
                bail!("{path}: invalid Flatpak application ID {value:?}; must be canonical");
            }
            Ok(())
        }

        fn validate_vscode_id(value: &str, path: &str) -> Result<()> {
            let re = Regex::new(r"^[a-z0-9-]+\.[a-z0-9-]+$").unwrap();
            if !re.is_match(value) {
                bail!("{path}: invalid VS Code extension identifier {value:?}; must be lowercase publisher.extension");
            }
            Ok(())
        }

        fn validate_gnome_uuid(value: &str, path: &str) -> Result<()> {
            let re = Regex::new(r"^[^@]+@[^@]+$").unwrap();
            if !re.is_match(value) {
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
                bail!(
                    "{path}: invalid repository path {value:?}; must contain at least one safe relative path segment"
                );
            }
            for segment in body.split('/') {
                if matches!(segment, "" | "." | "..") || validate_definition_name(segment, path).is_err() {
                    bail!(
                        "{path}: invalid repository path {value:?}; contains invalid relative path segment {segment:?}"
                    );
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
                    && parts.iter().all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
                    && valid_calendar_date(&parts)
                {
                    return Ok(());
                }
            }
            validate_numeric_version(value, path, 2, 3)
        }

        fn valid_calendar_date(parts: &[&str]) -> bool {
            let (Ok(year), Ok(month), Ok(day)) =
                (parts[0].parse::<u16>(), parts[1].parse::<u8>(), parts[2].parse::<u8>())
            else {
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
            let re = Regex::new(r"^[0-9]+[smh]$").unwrap();
            if !re.is_match(value) {
                bail!(
                    "{path}: invalid duration {value:?}; must be a non-negative decimal integer followed by s, m, or h"
                );
            }
            Ok(())
        }

        fn validate_docker_size(value: &str, path: &str) -> Result<()> {
            let re = Regex::new(r"^[1-9][0-9]*[kmg]$").unwrap();
            if !re.is_match(value) {
                bail!(
                    "{path}: invalid Docker size {value:?}; must be a positive decimal integer followed by k, m, or g"
                );
            }
            Ok(())
        }
    }
    pub use model::*;
}

mod domain {

    use anyhow::{bail, Context, Result};
    use serde::{de, Deserialize, Deserializer};
    use std::fmt;
    use url::{Host, Url};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct HttpsUrl(Url);

    impl HttpsUrl {
        pub fn as_str(&self) -> &str {
            self.0.as_str()
        }

        pub(crate) fn parse(value: &str) -> Result<Self> {
            validate_non_empty(value)?;
            if value.chars().any(char::is_whitespace)
                || value.chars().any(char::is_control)
                || value.contains('\\')
                || has_substitution(value)
            {
                bail!("invalid HTTPS URL {value:?}; must be literal and contain no whitespace or substitutions");
            }
            let parsed = Url::parse(value)
                .with_context(|| format!("invalid HTTPS URL {value:?}; must be a valid absolute URL"))?;
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
                bail!(
                    "invalid HTTPS URL {value:?}; must use HTTPS with a non-empty host and no credentials or fragment"
                );
            }
            Ok(Self(parsed))
        }
    }

    impl<'de> Deserialize<'de> for HttpsUrl {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
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

            let value = deserializer.deserialize_any(StrictStringVisitor)?;
            Self::parse(&value).map_err(de::Error::custom)
        }
    }

    fn validate_non_empty(value: &str) -> Result<()> {
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
}

mod init {

    use anyhow::{bail, Context, Result};

    #[derive(Clone, Copy, Debug)]
    pub struct Record {
        pub path: &'static str,
        pub bytes: &'static [u8],
        pub mode: u32,
    }

    #[derive(Clone, Copy, Debug)]
    pub struct PresetRecord {
        pub name: &'static str,
        pub bytes: &'static [u8],
    }

    include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

    pub fn records() -> &'static [Record] {
        RECORDS
    }

    pub fn preset(name: &str) -> Option<&'static PresetRecord> {
        PRESETS.iter().find(|preset| preset.name == name)
    }
    use clap::ValueEnum;
    use sha2::{Digest, Sha256};
    use std::{
        collections::BTreeMap,
        env,
        fs::{self, File},
        io::{self, Write},
        os::unix::fs::PermissionsExt,
        path::{Component, Path, PathBuf},
    };

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
    pub enum Preset {
        #[default]
        Cozydot,
        Full,
        Cli,
        Vm,
    }

    impl Preset {
        fn name(self) -> &'static str {
            match self {
                Self::Cozydot => "cozydot",
                Self::Full => "full",
                Self::Cli => "cli",
                Self::Vm => "vm",
            }
        }
    }

    pub fn run(preset_val: Preset) -> Result<PathBuf> {
        let root = config_root()?;
        let preset_rec = preset(preset_val.name()).context("embedded preset is missing")?;
        let mut records_vec = Vec::with_capacity(records().len() + 1);
        records_vec.push(Record {
            path: "cozydot.yaml",
            bytes: preset_rec.bytes,
            mode: 0o644,
        });
        records_vec.extend_from_slice(records());
        sync(&root, &records_vec)?;
        Ok(root)
    }

    fn sync(root: &Path, records: &[Record]) -> Result<()> {
        ensure_directory_path(root, Path::new(""))?;
        let manifest_path = root.join(".managed-files");
        let pending_path = root.join(".managed-files.pending");
        let mut managed = read_manifest(&manifest_path)?;
        recover_pending(root, &pending_path, &mut managed)?;

        let mut installs = 0usize;
        for record in records {
            let relative = PathBuf::from(record.path);
            validate_relative(&relative)?;
            let destination = root.join(&relative);
            let new_hash = hash_bytes(record.bytes);
            let old_hash = managed.get(&relative).cloned();
            let install = match fs::symlink_metadata(&destination) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => true,
                Err(e) => return Err(e.into()),
                Ok(metadata) if !metadata.file_type().is_file() => false,
                Ok(_) => old_hash
                    .as_ref()
                    .is_some_and(|hash| hash_file(&destination).ok().as_ref() == Some(hash)),
            };
            if !install {
                continue;
            }
            append_pending(&pending_path, old_hash.as_deref(), &new_hash, &relative)?;
            install_file(root, record, &relative)?;
            managed.insert(relative.clone(), new_hash);
            installs += 1;
            if env::var_os("COZYDOT_TEST_FAIL_AFTER_RELATIVE").as_deref() == Some(relative.as_os_str())
                || env::var("COZYDOT_TEST_FAIL_AFTER_INSTALLS").ok().as_deref() == Some(&installs.to_string())
            {
                bail!("injected init failure");
            }
        }
        write_manifest(&manifest_path, &managed)?;
        remove_if_exists(&pending_path)?;
        Ok(())
    }

    pub fn config_root() -> Result<PathBuf> {
        if let Some(path) = env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
            return Ok(PathBuf::from(path).join("cozydot"));
        }
        Ok(PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".config/cozydot"))
    }

    fn install_file(root: &Path, record: &Record, relative: &Path) -> Result<()> {
        let parent = relative.parent().unwrap_or(Path::new(""));
        ensure_directory_path(root, parent)?;
        let destination = root.join(relative);
        let destination_parent = required_parent(&destination)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".cozydot.")
            .tempfile_in(destination_parent)?;
        if env::var("COZYDOT_TEST_FAIL_MANAGED_FILE_AT").ok().as_deref() == Some("cp") {
            bail!("injected copy failure");
        }
        temporary.write_all(record.bytes)?;
        temporary.as_file_mut().sync_all()?;
        temporary
            .as_file_mut()
            .set_permissions(fs::Permissions::from_mode(record.mode))?;
        if env::var("COZYDOT_TEST_FAIL_MANAGED_FILE_AT").ok().as_deref() == Some("signal") {
            bail!("injected signal failure");
        }
        if env::var("COZYDOT_TEST_FAIL_MANAGED_FILE_AT").ok().as_deref() == Some("mv") {
            bail!("injected rename failure");
        }
        temporary.persist(&destination).map_err(|e| e.error)?;
        sync_directory(destination_parent)?;
        Ok(())
    }

    fn ensure_directory_path(root: &Path, relative: &Path) -> Result<()> {
        if root.exists() && fs::symlink_metadata(root)?.file_type().is_symlink() {
            bail!("configuration root is a symlink");
        }
        fs::create_dir_all(root)?;
        let mut current = root.to_path_buf();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                bail!("unsafe destination path");
            };
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    bail!("refusing symlinked config path: {}", current.display())
                }
                Ok(meta) if !meta.is_dir() => {
                    bail!("refusing non-directory config path: {}", current.display())
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn read_manifest(path: &Path) -> Result<BTreeMap<PathBuf, String>> {
        let mut result = BTreeMap::new();
        let text = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(result),
            Err(e) => return Err(e.into()),
        };
        for line in text.lines() {
            let (hash, relative) = line.split_once('\t').context("malformed managed-files record")?;
            let relative = PathBuf::from(relative);
            validate_hash(hash)?;
            validate_relative(&relative)?;
            if result.insert(relative, hash.into()).is_some() {
                bail!("duplicate managed-files record");
            }
        }
        Ok(result)
    }

    fn recover_pending(root: &Path, path: &Path, managed: &mut BTreeMap<PathBuf, String>) -> Result<()> {
        let text = match fs::read_to_string(path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        validate_pending(&text)?;
        for line in text.lines() {
            let fields: Vec<_> = line.split('\t').collect();
            let relative = PathBuf::from(fields[2]);
            let current = hash_file(&root.join(&relative)).ok();
            if current.as_deref() == Some(fields[1]) {
                managed.insert(relative, fields[1].into());
            } else if fields[0] != "-" && current.as_deref() == Some(fields[0]) {
                managed.insert(relative, fields[0].into());
            } else {
                managed.remove(&relative);
            }
        }
        Ok(())
    }

    fn append_pending(path: &Path, old: Option<&str>, new: &str, relative: &Path) -> Result<()> {
        append_pending_with_failure(path, old, new, relative, None)
    }

    fn append_pending_with_failure(
        path: &Path,
        old: Option<&str>,
        new: &str,
        relative: &Path,
        failure: Option<&str>,
    ) -> Result<()> {
        validate_relative(relative)?;
        validate_hash(new)?;
        if let Some(old) = old {
            validate_hash(old)?;
        }
        let mut records = match fs::read_to_string(path) {
            Ok(text) => {
                validate_pending(&text)?;
                text
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        records.push_str(&format!("{}\t{}\t{}\n", old.unwrap_or("-"), new, relative.display()));
        let parent = required_parent(path)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".managed-files.pending.")
            .tempfile_in(parent)?;
        temporary.write_all(records.as_bytes())?;
        temporary.flush()?;
        temporary.as_file_mut().sync_all()?;
        if failure == Some("pre-publish") {
            bail!("injected pending journal failure before publication");
        }
        temporary.persist(path).map_err(|e| e.error)?;
        sync_directory(parent)?;
        if failure == Some("post-publish") {
            bail!("injected pending journal failure after publication");
        }
        Ok(())
    }

    fn validate_pending(text: &str) -> Result<()> {
        for line in text.lines() {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 3 {
                bail!("malformed pending record");
            }
            if fields[0] != "-" {
                validate_hash(fields[0])?;
            }
            validate_hash(fields[1])?;
            validate_relative(Path::new(fields[2]))?;
        }
        Ok(())
    }

    fn write_manifest(path: &Path, managed: &BTreeMap<PathBuf, String>) -> Result<()> {
        let parent = required_parent(path)?;
        let mut temporary = tempfile::Builder::new().prefix(".managed-files.").tempfile_in(parent)?;
        for (relative, hash) in managed {
            writeln!(temporary, "{}\t{}", hash, relative.display())?;
        }
        temporary.as_file_mut().sync_all()?;
        temporary.persist(path).map_err(|e| e.error)?;
        sync_directory(parent)?;
        Ok(())
    }

    fn required_parent(path: &Path) -> Result<&Path> {
        path.parent()
            .with_context(|| format!("path has no parent: {}", path.display()))
    }

    fn sync_directory(path: &Path) -> Result<()> {
        File::open(path)?.sync_all()?;
        Ok(())
    }

    fn validate_hash(hash: &str) -> Result<()> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("invalid SHA-256 record");
        }
        Ok(())
    }
    fn hash_bytes(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    fn hash_file(path: &Path) -> Result<String> {
        Ok(hash_bytes(&fs::read(path)?))
    }
    fn validate_relative(path: &Path) -> Result<()> {
        if path.as_os_str().is_empty()
            || path.components().any(|c| !matches!(c, Component::Normal(_)))
            || path.to_string_lossy().contains(['\t', '\n'])
        {
            bail!("unsafe managed path: {}", path.display());
        }
        Ok(())
    }
    fn remove_if_exists(path: &Path) -> Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

mod runner {

    use crate::operations::{self, Operation, OperationOutcome};
    use anyhow::Result;
    use std::fmt;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ExecutionPhase {
        SystemPrerequisites,
        ManagerBootstraps,
        AdministrativeVerification,
        OfficialAptSources,
        ThirdPartyRepositories,
        AptMetadataRefresh,
        SystemPackageStates,
        AptPurge,
        RepositoryPackages,
        AptPackages,
        FlatpakApplications,
        LanguageToolchains,
        LanguagePackages,
        BinaryPackages,
        Fonts,
        Dotfiles,
        Integrations,
        Desktop,
        Updates,
        FinalVerification,
    }

    impl ExecutionPhase {
        pub fn name(self) -> &'static str {
            match self {
                Self::SystemPrerequisites => "system-prerequisites",
                Self::ManagerBootstraps => "manager-bootstraps",
                Self::AdministrativeVerification => "administrative-verification",
                Self::OfficialAptSources => "official-apt-sources",
                Self::ThirdPartyRepositories => "third-party-repositories",
                Self::AptMetadataRefresh => "apt-metadata-refresh",
                Self::SystemPackageStates => "system-package-states",
                Self::AptPurge => "apt-purge",
                Self::RepositoryPackages => "repository-packages",
                Self::AptPackages => "apt-packages",
                Self::FlatpakApplications => "flatpak-applications",
                Self::LanguageToolchains => "language-toolchains",
                Self::LanguagePackages => "language-packages",
                Self::BinaryPackages => "binary-packages",
                Self::Fonts => "fonts",
                Self::Dotfiles => "dotfiles",
                Self::Integrations => "integrations",
                Self::Desktop => "desktop",
                Self::Updates => "updates",
                Self::FinalVerification => "final-verification",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SkippedAction {
        UbuntuSnap,
        UbuntuCodecs,
    }

    impl SkippedAction {
        fn name(self) -> &'static str {
            match self {
                Self::UbuntuSnap => "ubuntu-snap",
                Self::UbuntuCodecs => "ubuntu-codecs",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SkipReason {
        RequiresUbuntuFamily,
    }

    impl SkipReason {
        fn description(self) -> &'static str {
            match self {
                Self::RequiresUbuntuFamily => "requires Ubuntu family",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ExplicitSkip {
        pub action: SkippedAction,
        pub reason: SkipReason,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum StepKind {
        Phase(ExecutionPhase),
        Operation {
            operation: Box<Operation>,
            label: Option<String>,
        },
        Skip(ExplicitSkip),
        Summary,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Step(StepKind);

    impl Step {
        pub fn workflow(operation: Operation) -> Self {
            Self(StepKind::Operation {
                operation: Box::new(operation),
                label: None,
            })
        }

        pub fn labeled_workflow(operation: Operation, label: impl Into<String>) -> Result<Self> {
            let label = label.into();
            if label.is_empty() || label.chars().any(char::is_control) {
                return Err(anyhow::anyhow!(
                    "runner operation label must be nonempty printable text"
                ));
            }
            Ok(Self(StepKind::Operation {
                operation: Box::new(operation),
                label: Some(label),
            }))
        }

        pub fn phase(phase: ExecutionPhase) -> Self {
            Self(StepKind::Phase(phase))
        }

        pub fn skip(action: SkippedAction, reason: SkipReason) -> Self {
            Self(StepKind::Skip(ExplicitSkip { action, reason }))
        }

        pub fn summary() -> Self {
            Self(StepKind::Summary)
        }

        pub fn kind(&self) -> &StepKind {
            &self.0
        }

        pub(crate) fn operation(&self) -> Option<&Operation> {
            match &self.0 {
                StepKind::Operation { operation, .. } => Some(operation.as_ref()),
                _ => None,
            }
        }

        pub fn display(&self) -> String {
            match &self.0 {
                StepKind::Phase(phase) => format!("phase {}", phase.name()),
                StepKind::Operation { operation, .. } => {
                    format!("workflow {}", operation.display_args().join(" "))
                }
                StepKind::Skip(skip) => {
                    format!("skip {} {}", skip.action.name(), skip.reason.description())
                }
                StepKind::Summary => "summary".into(),
            }
        }

        fn report_name(&self) -> String {
            match &self.0 {
                StepKind::Operation { label: Some(label), .. } => format!("{label}: {}", self.display()),
                _ => self.display(),
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum StepOutcome {
        PhaseStarted,
        Completed,
        LoginRequired,
        Skipped,
        Planned,
        Failed(String),
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct StepReport {
        pub step: Step,
        pub outcome: StepOutcome,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct ExecutionSummary {
        pub completed: usize,
        pub skipped: usize,
        pub login_required: usize,
        pub planned: usize,
        pub failed: usize,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct ExecutionReport {
        pub steps: Vec<StepReport>,
        pub summary: ExecutionSummary,
    }

    impl ExecutionReport {
        pub fn render(&self) -> String {
            let mut lines = self
                .steps
                .iter()
                .map(|report| match (&report.step.0, &report.outcome) {
                    (StepKind::Phase(phase), StepOutcome::PhaseStarted) => {
                        format!("== phase: {}", phase.name())
                    }
                    (StepKind::Operation { .. }, StepOutcome::Completed) => {
                        format!("completed: {}", report.step.report_name())
                    }
                    (StepKind::Operation { .. }, StepOutcome::LoginRequired) => {
                        format!("login-required: {}", report.step.report_name())
                    }
                    (StepKind::Operation { .. }, StepOutcome::Planned) => {
                        format!("planned: {}", report.step.report_name())
                    }
                    (StepKind::Skip(skip), StepOutcome::Skipped) => {
                        format!("skipped: {} ({})", skip.action.name(), skip.reason.description())
                    }
                    (StepKind::Operation { .. }, StepOutcome::Failed(error)) => {
                        format!("failed: {} ({error})", report.step.report_name())
                    }
                    _ => format!("invalid-report: {}", report.step.display()),
                })
                .collect::<Vec<_>>();
            lines.push(format!(
                "summary: {} completed, {} skipped, {} login-required, {} planned, {} failed",
                self.summary.completed,
                self.summary.skipped,
                self.summary.login_required,
                self.summary.planned,
                self.summary.failed,
            ));
            lines.join("\n")
        }

        fn push(&mut self, step: Step, outcome: StepOutcome) {
            match outcome {
                StepOutcome::Completed => self.summary.completed += 1,
                StepOutcome::Skipped => self.summary.skipped += 1,
                StepOutcome::LoginRequired => self.summary.login_required += 1,
                StepOutcome::Planned => self.summary.planned += 1,
                StepOutcome::Failed(_) => self.summary.failed += 1,
                StepOutcome::PhaseStarted => {}
            }
            self.steps.push(StepReport { step, outcome });
        }
    }

    #[derive(Debug)]
    pub struct ExecutionFailure {
        pub report: ExecutionReport,
        source: anyhow::Error,
    }

    impl fmt::Display for ExecutionFailure {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "{}", self.source)
        }
    }

    impl std::error::Error for ExecutionFailure {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(self.source.as_ref())
        }
    }

    pub struct ProcessRunner {
        pub dry_run: bool,
    }

    impl ProcessRunner {
        fn run(&mut self, operation: &Operation) -> Result<OperationOutcome> {
            operations::execute(operation, &[])
        }
    }

    pub fn execute(
        runner: &mut ProcessRunner,
        steps: &[Step],
    ) -> std::result::Result<ExecutionReport, ExecutionFailure> {
        let result = execute_with(steps, runner.dry_run, |operation| runner.run(operation));
        match &result {
            Ok(report) => println!("{}", report.render()),
            Err(failure) => println!("{}", failure.report.render()),
        }
        result
    }

    fn execute_with<F>(
        steps: &[Step],
        dry_run: bool,
        mut run: F,
    ) -> std::result::Result<ExecutionReport, ExecutionFailure>
    where
        F: FnMut(&Operation) -> Result<OperationOutcome>,
    {
        let mut report = ExecutionReport::default();
        for step in steps {
            match step.kind() {
                StepKind::Phase(_) => report.push(step.clone(), StepOutcome::PhaseStarted),
                StepKind::Skip(_) => report.push(step.clone(), StepOutcome::Skipped),
                StepKind::Summary => {}
                StepKind::Operation { .. } if dry_run => report.push(step.clone(), StepOutcome::Planned),
                StepKind::Operation { .. } => match run(step
                    .operation()
                    .expect("matched operation step must contain an operation"))
                {
                    Ok(OperationOutcome::Completed) => report.push(step.clone(), StepOutcome::Completed),
                    Ok(OperationOutcome::LoginRequired) => report.push(step.clone(), StepOutcome::LoginRequired),
                    Err(error) => {
                        let message = format!("{error:#}");
                        report.push(step.clone(), StepOutcome::Failed(message));
                        return Err(ExecutionFailure { report, source: error });
                    }
                },
            }
        }
        Ok(report)
    }
}

mod platform {

    use std::{fs, io::ErrorKind, path::Path, process::Command};

    use anyhow::{bail, Context, Result};

    use self::os_release::OsRelease;

    mod os_release {
        use anyhow::{bail, Context, Result};
        use std::collections::BTreeMap;

        #[derive(Debug, Clone, PartialEq, Eq)]
        pub(super) struct OsRelease {
            fields: BTreeMap<String, String>,
        }

        impl OsRelease {
            pub(super) fn parse(input: &str) -> Result<Self> {
                let mut fields = BTreeMap::new();
                for (index, line) in input.lines().enumerate() {
                    let line_number = index + 1;
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let (key, value) = line
                        .split_once('=')
                        .with_context(|| format!("os-release line {line_number} is not KEY=VALUE"))?;
                    validate_key(key).with_context(|| format!("invalid os-release key on line {line_number}"))?;
                    let value = parse_value(value)
                        .with_context(|| format!("invalid os-release value on line {line_number}"))?;
                    // os-release(5) specifies that readers use the later assignment.
                    fields.insert(key.to_owned(), value);
                }
                Ok(Self { fields })
            }

            pub(super) fn get(&self, key: &str) -> Option<&str> {
                self.fields.get(key).map(String::as_str)
            }
        }

        fn validate_key(key: &str) -> Result<()> {
            let mut bytes = key.bytes();
            if !bytes.next().is_some_and(|byte| byte.is_ascii_uppercase())
                || !bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            {
                bail!("keys must match [A-Z][A-Z0-9_]*");
            }
            Ok(())
        }

        fn parse_value(value: &str) -> Result<String> {
            if value.bytes().any(|byte| byte.is_ascii_control()) {
                bail!("control characters are not allowed");
            }
            match value.as_bytes().first().copied() {
                Some(b'\'') => parse_single_quoted(value),
                Some(b'"') => parse_double_quoted(value),
                _ => parse_unquoted(value),
            }
        }

        fn parse_single_quoted(value: &str) -> Result<String> {
            let body = value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .context("unmatched single quote")?;
            if body.contains('\'') {
                bail!("single-quoted values cannot contain a single quote");
            }
            Ok(body.to_owned())
        }

        fn parse_double_quoted(value: &str) -> Result<String> {
            let body = value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .context("unmatched double quote")?;
            let mut output = String::with_capacity(body.len());
            let mut chars = body.chars();
            while let Some(character) = chars.next() {
                match character {
                    '\\' => {
                        let escaped = chars.next().context("dangling escape")?;
                        if matches!(escaped, '$' | '"' | '\\' | '`') {
                            output.push(escaped);
                        } else {
                            // POSIX double quotes preserve a backslash before other characters.
                            output.push('\\');
                            output.push(escaped);
                        }
                    }
                    '"' => bail!("unescaped double quote"),
                    '$' | '`' => bail!("variable and command expansion are not supported"),
                    _ => output.push(character),
                }
            }
            Ok(output)
        }

        fn parse_unquoted(value: &str) -> Result<String> {
            let mut output = String::with_capacity(value.len());
            let mut chars = value.chars();
            while let Some(character) = chars.next() {
                match character {
                    '\\' => output.push(chars.next().context("dangling escape")?),
                    '\'' | '"' => bail!("quoted and unquoted fragments cannot be concatenated"),
                    '$' | '`' => bail!("variable and command expansion are not supported"),
                    character
                        if character.is_ascii_whitespace()
                            || matches!(
                                character,
                                '|' | '&' | ';' | '(' | ')' | '<' | '>' | '*' | '?' | '[' | ']' | '{' | '}' | '~' | '#'
                            ) =>
                    {
                        bail!("shell-special characters must be quoted or escaped")
                    }
                    _ => output.push(character),
                }
            }
            Ok(output)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Architecture {
        Amd64,
        Arm64,
        Arm32,
        Riscv64,
    }

    impl Architecture {
        pub fn normalize(value: &str) -> Result<Self> {
            match value {
                "x86_64" | "amd64" => Ok(Self::Amd64),
                "aarch64" | "arm64" => Ok(Self::Arm64),
                "arm32" | "armv7" | "armv7l" | "armhf" => Ok(Self::Arm32),
                "riscv64" => Ok(Self::Riscv64),
                _ => bail!("unsupported architecture {value:?}; supported architectures: amd64, arm64, arm32, riscv64"),
            }
        }

        pub fn canonical(self) -> &'static str {
            match self {
                Self::Amd64 => "amd64",
                Self::Arm64 => "arm64",
                Self::Arm32 => "arm32",
                Self::Riscv64 => "riscv64",
            }
        }

        pub fn debian(self) -> &'static str {
            match self {
                Self::Amd64 => "amd64",
                Self::Arm64 => "arm64",
                Self::Arm32 => "armhf",
                Self::Riscv64 => "riscv64",
            }
        }

        pub fn go(self) -> &'static str {
            match self {
                Self::Amd64 => "amd64",
                Self::Arm64 => "arm64",
                Self::Arm32 => "arm",
                Self::Riscv64 => "riscv64",
            }
        }

        pub fn go_archive(self) -> &'static str {
            match self {
                Self::Arm32 => "armv6l",
                other => other.go(),
            }
        }

        pub fn rust_target(self) -> &'static str {
            match self {
                Self::Amd64 => "x86_64-unknown-linux-gnu",
                Self::Arm64 => "aarch64-unknown-linux-gnu",
                Self::Arm32 => "armv7-unknown-linux-gnueabihf",
                Self::Riscv64 => "riscv64gc-unknown-linux-gnu",
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Platform {
        pub distro: String,
        pub upstream: String,
        pub distro_codename: String,
        pub base_codename: String,
        pub desktop: String,
        pub architecture: Architecture,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ManagedAptSources {
        pub distro: String,
        pub release: String,
        pub architecture: Architecture,
        pub components: Vec<String>,
        pub stanzas: Vec<ManagedAptStanza>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ManagedAptStanza {
        pub uri: String,
        pub suites: Vec<String>,
        pub signed_by: String,
    }

    impl ManagedAptSources {
        pub fn render_deb822(&self) -> String {
            let components = self.components.join(" ");
            self.stanzas
                .iter()
                .map(|stanza| {
                    format!(
                        "Types: deb\nURIs: {}\nSuites: {}\nComponents: {}\nArchitectures: {}\nSigned-By: {}\n",
                        stanza.uri,
                        stanza.suites.join(" "),
                        components,
                        self.architecture.debian(),
                        stanza.signed_by,
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    impl Platform {
        pub fn detect() -> Result<Self> {
            let os = read_system_os_release()?;
            let uname = Command::new("uname").arg("-m").output().context("run uname -m")?;
            let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
            let desktop = desktop(std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().as_str());
            Self::from_os_release(&os, desktop, &arch)
        }

        pub fn from_parts(
            distro: String,
            upstream: String,
            codename: String,
            desktop: String,
            arch: &str,
        ) -> Result<Self> {
            Self::from_release_parts(distro, upstream, codename.clone(), codename, desktop, arch)
        }

        pub fn from_release_parts(
            distro: String,
            upstream: String,
            distro_codename: String,
            base_codename: String,
            desktop: String,
            arch: &str,
        ) -> Result<Self> {
            let architecture = Architecture::normalize(arch)?;
            Ok(Self {
                distro,
                upstream,
                distro_codename,
                base_codename,
                desktop: normalize_desktop(&desktop),
                architecture,
            })
        }

        fn from_os_release(os: &OsRelease, desktop: String, arch: &str) -> Result<Self> {
            let distro = os.get("ID").unwrap_or_default().to_owned();
            let upstream: String = upstream(&distro, os.get("ID_LIKE"))?.into();
            let distro_codename = os.get("VERSION_CODENAME").unwrap_or_default().to_owned();
            let base_codename = match upstream.as_str() {
                "ubuntu" => extra_codename(os, "UBUNTU_CODENAME").unwrap_or_else(|| distro_codename.clone()),
                "debian" => extra_codename(os, "DEBIAN_CODENAME").unwrap_or_else(|| distro_codename.clone()),
                _ => unreachable!(),
            };
            Self::from_release_parts(distro, upstream, distro_codename, base_codename, desktop, arch)
        }

        pub fn managed_apt_sources(&self, configured_components: &[&str]) -> Result<ManagedAptSources> {
            if !matches!(self.distro.as_str(), "ubuntu" | "debian" | "kali") {
                bail!(
                    "system.apt.sources: managed is unsupported for distribution {:?}; use preserve",
                    self.distro
                );
            }
            let components = managed_components(self, configured_components)?;
            let architecture = self.architecture;
            if matches!(self.distro.as_str(), "ubuntu" | "debian")
                && (self.distro_codename.is_empty()
                    || !self.distro_codename.bytes().enumerate().all(|(index, byte)| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || index != 0 && matches!(byte, b'.' | b'_' | b'+' | b'-')
                    }))
            {
                bail!("system.apt.sources: managed requires a valid platform codename");
            }
            let (release, stanzas) = match self.distro.as_str() {
                "ubuntu" => {
                    let release = self.distro_codename.as_str();
                    if !matches!(release, "jammy" | "noble" | "questing" | "resolute") {
                        bail!(
                            "system.apt.sources: managed Ubuntu release {:?} is unsupported; supported releases are jammy, noble, questing, and resolute",
                            release
                        );
                    }
                    let main_archive = architecture == Architecture::Amd64
                        || architecture == Architecture::Arm64 && release == "resolute";
                    let keyring = "/usr/share/keyrings/ubuntu-archive-keyring.gpg";
                    let stanzas = if main_archive {
                        vec![
                            ManagedAptStanza {
                                uri: "https://archive.ubuntu.com/ubuntu".into(),
                                suites: vec![
                                    release.into(),
                                    format!("{release}-updates"),
                                    format!("{release}-backports"),
                                ],
                                signed_by: keyring.into(),
                            },
                            ManagedAptStanza {
                                uri: "https://security.ubuntu.com/ubuntu".into(),
                                suites: vec![format!("{release}-security")],
                                signed_by: keyring.into(),
                            },
                        ]
                    } else {
                        vec![ManagedAptStanza {
                            uri: "https://ports.ubuntu.com/ubuntu-ports".into(),
                            suites: vec![
                                release.into(),
                                format!("{release}-updates"),
                                format!("{release}-backports"),
                                format!("{release}-security"),
                            ],
                            signed_by: keyring.into(),
                        }]
                    };
                    (release.to_owned(), stanzas)
                }
                "debian" => {
                    let release = self.distro_codename.as_str();
                    if !matches!(release, "bullseye" | "bookworm" | "trixie") {
                        bail!(
                            "system.apt.sources: managed Debian release {:?} is unsupported; supported releases are bullseye, bookworm, and trixie",
                            release
                        );
                    }
                    if architecture == Architecture::Riscv64 && release != "trixie" {
                        bail!("system.apt.sources: Debian {release} does not support architecture riscv64");
                    }
                    let keyring = "/usr/share/keyrings/debian-archive-keyring.gpg";
                    (
                        release.to_owned(),
                        vec![
                            ManagedAptStanza {
                                uri: "https://deb.debian.org/debian".into(),
                                suites: vec![release.into(), format!("{release}-updates")],
                                signed_by: keyring.into(),
                            },
                            ManagedAptStanza {
                                uri: "https://security.debian.org/debian-security".into(),
                                suites: vec![format!("{release}-security")],
                                signed_by: keyring.into(),
                            },
                        ],
                    )
                }
                "kali" => {
                    if architecture == Architecture::Riscv64 {
                        bail!("system.apt.sources: Kali rolling does not support architecture riscv64");
                    }
                    (
                        "kali-rolling".into(),
                        vec![ManagedAptStanza {
                            uri: "https://http.kali.org/kali".into(),
                            suites: vec!["kali-rolling".into()],
                            signed_by: "/usr/share/keyrings/kali-archive-keyring.gpg".into(),
                        }],
                    )
                }
                _ => unreachable!(),
            };
            Ok(ManagedAptSources {
                distro: self.distro.clone(),
                release,
                architecture,
                components,
                stanzas,
            })
        }
    }
    fn parse_uname_machine(success: bool, stdout: &[u8]) -> Result<String> {
        if !success {
            bail!("uname -m failed");
        }
        let machine = std::str::from_utf8(stdout)
            .context("uname -m output is not UTF-8")?
            .trim();
        if machine.is_empty() {
            bail!("uname -m returned an empty machine architecture");
        }
        Ok(machine.into())
    }
    fn upstream(id: &str, id_like: Option<&str>) -> Result<&'static str> {
        match id {
            "ubuntu" | "pop" | "zorin" => Ok("ubuntu"),
            "debian" | "kali" | "tails" | "deepin" => Ok("debian"),
            "linuxmint" => {
                let families = id_like.unwrap_or_default().split_ascii_whitespace();
                let mut ubuntu = false;
                let mut debian = false;
                for family in families {
                    ubuntu |= family == "ubuntu";
                    debian |= family == "debian";
                }
                match (ubuntu, debian) {
                    (true, _) => Ok("ubuntu"),
                    (false, true) => Ok("debian"),
                    _ => bail!(
                        "unsupported linuxmint base family in ID_LIKE {:?}; expected ubuntu or debian",
                        id_like.unwrap_or_default()
                    ),
                }
            }
            _ => bail!("unsupported distro: {id}"),
        }
    }

    fn managed_components(platform: &Platform, configured: &[&str]) -> Result<Vec<String>> {
        let defaults: &[&str] = if platform.distro == "kali" {
            &["main", "contrib", "non-free", "non-free-firmware"]
        } else {
            &["main"]
        };
        let components = if configured.is_empty() { defaults } else { configured };
        let supported: &[&str] = match platform.distro.as_str() {
            "ubuntu" => &["main", "restricted", "universe", "multiverse"],
            "debian" if platform.distro_codename == "bullseye" => &["main", "contrib", "non-free"],
            "debian" => &["main", "contrib", "non-free", "non-free-firmware"],
            "kali" => &["main", "contrib", "non-free", "non-free-firmware"],
            _ => &[],
        };
        let mut result = Vec::new();
        for (index, component) in components.iter().enumerate() {
            if !supported.contains(component) {
                bail!(
                    "system.apt.components[{index}]: component {component:?} is unsupported for {} {}",
                    platform.distro,
                    platform.distro_codename
                );
            }
            if result.iter().any(|existing| existing == component) {
                bail!("system.apt.components: duplicate component {component:?}");
            }
            result.push((*component).to_owned());
        }
        Ok(result)
    }
    fn desktop(s: &str) -> String {
        normalize_desktop(s)
    }

    fn normalize_desktop(value: &str) -> String {
        value
            .split(':')
            .find_map(|token| {
                let token = token.to_ascii_lowercase();
                if token.contains("gnome") {
                    Some("gnome")
                } else if token.contains("cinnamon") {
                    Some("cinnamon")
                } else {
                    None
                }
            })
            .unwrap_or("none")
            .into()
    }

    fn read_os_release(path: &Path) -> Result<OsRelease> {
        let text = fs::read_to_string(path).with_context(|| format!("read os-release at {}", path.display()))?;
        parse_os_release(path, &text)
    }

    fn parse_os_release(path: &Path, text: &str) -> Result<OsRelease> {
        OsRelease::parse(text).with_context(|| format!("parse os-release at {}", path.display()))
    }

    fn read_system_os_release() -> Result<OsRelease> {
        read_system_os_release_from(Path::new("/etc/os-release"), Path::new("/usr/lib/os-release"))
    }

    fn read_system_os_release_from(etc_path: &Path, usr_path: &Path) -> Result<OsRelease> {
        match fs::read_to_string(etc_path) {
            Ok(text) => parse_os_release(etc_path, &text),
            Err(error) if error.kind() == ErrorKind::NotFound => read_os_release(usr_path),
            Err(error) => Err(error).with_context(|| format!("read os-release at {}", etc_path.display())),
        }
    }

    fn extra_codename(os: &OsRelease, key: &str) -> Option<String> {
        os.get(key).map(str::to_owned)
    }
}

mod planner {
    use crate::{
        config::{
            resolve_platform_identity, AptUpdate, BinaryFormat, Config, EnabledDisabled, InstalledState,
            ResolvedNativeBinary, SourceMode, Theme,
        },
        operations::{
            AptRepositoryOperation, AptRepositoryPath, AptRepositorySourceLayout, AptRepositoryToken, AptUpgradePolicy,
            BinaryPackageFormat, BinaryPackageMode, BinaryPackageOperation, BinaryPackageSelector, BinarySha256,
            BinarySourceOperation, CargoBinstallBootstrapOperation, CargoPackageMode, CargoPackageOperation,
            DesktopEnvironment, DesktopSetting, DesktopSettingOperation, DesktopTheme, DockerLocalLogOperation,
            DotfilesOperation, EnsureAdminOperation, GithubRepository, GnomeDockOperation, GnomeExtensionsOperation,
            GnomeRoundedCornersOperation, GoToolchainOperation, GoToolchainSelector, ManagedAptSourcesOperation,
            NerdFontsMode, NerdFontsOperation, NodeToolchainOperation, NodeToolchainSelector, NpmPackageMode,
            NpmPackageOperation, Operation, PythonToolchainOperation, RustToolchainOperation, RustToolchainSelector,
            ToolMutationMode, UbuntuSnapOperation, UnattendedUpgradesOperation, VsCodeExtensionOperation,
        },
        platform::{Architecture, Platform},
        runner::{self, ExecutionPhase, Step},
    };
    use anyhow::{Context, Result};
    use std::{collections::BTreeSet, path::Path};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum ManagerBootstrap {
        Flatpak,
        Rustup,
        CargoBinstall,
        Fnm,
        Uv,
    }

    pub fn plan(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<Vec<Step>> {
        config.validate_for_platform(platform)?;
        let identity = resolve_platform_identity(platform)?;
        let mut phases = [
            (ExecutionPhase::SystemPrerequisites, Vec::new()),
            (ExecutionPhase::ManagerBootstraps, Vec::new()),
            (ExecutionPhase::AdministrativeVerification, Vec::new()),
            (ExecutionPhase::OfficialAptSources, Vec::new()),
            (ExecutionPhase::ThirdPartyRepositories, Vec::new()),
            (ExecutionPhase::AptMetadataRefresh, Vec::new()),
            (ExecutionPhase::SystemPackageStates, Vec::new()),
            (ExecutionPhase::AptPurge, Vec::new()),
            (ExecutionPhase::RepositoryPackages, Vec::new()),
            (ExecutionPhase::AptPackages, Vec::new()),
            (ExecutionPhase::FlatpakApplications, Vec::new()),
            (ExecutionPhase::LanguageToolchains, Vec::new()),
            (ExecutionPhase::LanguagePackages, Vec::new()),
            (ExecutionPhase::BinaryPackages, Vec::new()),
            (ExecutionPhase::Fonts, Vec::new()),
            (ExecutionPhase::Dotfiles, Vec::new()),
            (ExecutionPhase::Integrations, Vec::new()),
            (ExecutionPhase::Desktop, Vec::new()),
            (ExecutionPhase::Updates, Vec::new()),
            (ExecutionPhase::FinalVerification, Vec::new()),
        ];
        let mut prerequisites = BTreeSet::new();
        let mut managers = BTreeSet::new();
        let mut needs_apt_refresh = false;

        let packages = config.packages.as_ref();
        let apt = packages.and_then(|packages| packages.apt.as_ref());

        if config
            .system
            .as_ref()
            .is_some_and(|system| system.ensure_admin == Some(true))
        {
            push_step(
                &mut phases,
                ExecutionPhase::AdministrativeVerification,
                Step::workflow(Operation::EnsureAdmin(EnsureAdminOperation::new())),
            );
        }

        if let Some(sources) = config
            .system
            .as_ref()
            .and_then(|system| system.apt.as_ref())
            .and_then(|apt| apt.sources.as_ref())
        {
            if sources.mode == SourceMode::Managed {
                let managed = sources
                    .resolve_managed(platform, identity)?
                    .expect("managed source resolution returns an intent");
                push_step(
                    &mut phases,
                    ExecutionPhase::OfficialAptSources,
                    Step::workflow(Operation::ManagedAptSources(ManagedAptSourcesOperation::from_policy(
                        managed,
                    )?)),
                );
            }
        }

        if let Some(repositories) = apt.and_then(|apt| apt.repositories.as_ref()) {
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            prerequisites.insert("gnupg");
            for repository in repositories {
                let operation = plan_repository(repository, platform, identity)?;
                push_step(
                    &mut phases,
                    ExecutionPhase::ThirdPartyRepositories,
                    Step::workflow(Operation::AptRepository(operation.clone())),
                );
                push_step(
                    &mut phases,
                    ExecutionPhase::RepositoryPackages,
                    Step::labeled_workflow(
                        Operation::AptPackages {
                            packages: repository.packages.clone(),
                        },
                        format!("repository {}", repository.name),
                    )?,
                );
                needs_apt_refresh = true;
            }
        }

        plan_system_states(config, platform, &mut phases, &mut needs_apt_refresh);

        if let Some(remove) = apt.and_then(|apt| apt.remove.as_ref()) {
            push_step(
                &mut phases,
                ExecutionPhase::AptPurge,
                Step::workflow(Operation::AptPurge {
                    packages: remove.clone(),
                }),
            );
            needs_apt_refresh = true;
        }
        if let Some(install) = apt.and_then(|apt| apt.install.as_ref()) {
            push_step(
                &mut phases,
                ExecutionPhase::AptPackages,
                Step::workflow(Operation::AptPackages {
                    packages: install.clone(),
                }),
            );
            needs_apt_refresh = true;
        }

        if let Some(applications) = packages.and_then(|packages| packages.flatpak.as_ref()) {
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            managers.insert(ManagerBootstrap::Flatpak);
            push_step(
                &mut phases,
                ExecutionPhase::FlatpakApplications,
                Step::workflow(Operation::FlatpakEnsureApps {
                    refs: applications.clone(),
                }),
            );
        }

        plan_tools(config, platform, &mut phases, &mut prerequisites, &mut managers)?;

        if let Some(cargo) = packages.and_then(|packages| packages.cargo.as_ref()) {
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            managers.insert(ManagerBootstrap::Rustup);
            managers.insert(ManagerBootstrap::CargoBinstall);
            push_step(
                &mut phases,
                ExecutionPhase::LanguagePackages,
                Step::workflow(Operation::CargoPackageSet(CargoPackageOperation::new(
                    cargo.clone(),
                    CargoPackageMode::EnsurePresent,
                )?)),
            );
        }
        if let Some(npm) = packages.and_then(|packages| packages.npm.as_ref()) {
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            managers.insert(ManagerBootstrap::Fnm);
            push_step(
                &mut phases,
                ExecutionPhase::LanguagePackages,
                Step::workflow(Operation::NpmPackageSet(NpmPackageOperation::new(
                    npm.clone(),
                    NpmPackageMode::EnsurePresent,
                )?)),
            );
        }

        if let Some(binaries) = packages.and_then(|packages| packages.binaries.as_ref()) {
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            for binary in binaries {
                let planned = plan_binary(binary, platform.architecture, BinaryPackageMode::EnsurePresent)?;
                match binary.format {
                    BinaryFormat::Deb => {
                        prerequisites.insert("dpkg");
                        needs_apt_refresh = true;
                    }
                    BinaryFormat::Appimage => {}
                }
                push_step(
                    &mut phases,
                    ExecutionPhase::BinaryPackages,
                    Step::workflow(Operation::BinaryPackage(planned)),
                );
            }
        }

        if let Some(fonts) = config.fonts.as_ref().and_then(|fonts| fonts.nerd.as_ref()) {
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            prerequisites.insert("tar");
            prerequisites.insert("xz-utils");
            prerequisites.insert("fontconfig");
            push_step(
                &mut phases,
                ExecutionPhase::Fonts,
                Step::workflow(Operation::NerdFonts(NerdFontsOperation::new(
                    fonts.clone(),
                    NerdFontsMode::EnsurePresent,
                )?)),
            );
        }

        if let Some(dotfiles) = &config.dotfiles {
            prerequisites.insert("stow");
            push_step(
                &mut phases,
                ExecutionPhase::Dotfiles,
                Step::workflow(Operation::Dotfiles(DotfilesOperation::new(
                    dotfiles_root.to_path_buf(),
                    dotfiles.packages.clone(),
                )?)),
            );
        }

        plan_integrations(config, &mut phases)?;
        plan_desktop(config, platform, &mut phases, &mut prerequisites)?;
        plan_updates(config, platform, &mut phases, &mut needs_apt_refresh)?;

        if needs_apt_refresh {
            push_step(
                &mut phases,
                ExecutionPhase::AptMetadataRefresh,
                Step::workflow(Operation::AptMetadataRefresh),
            );
        }

        if managers.contains(&ManagerBootstrap::Flatpak) {
            prerequisites.insert("flatpak");
        }
        if managers.contains(&ManagerBootstrap::Fnm) {
            prerequisites.insert("unzip");
        }
        if managers.contains(&ManagerBootstrap::CargoBinstall) {
            prerequisites.insert("tar");
        }

        if !prerequisites.is_empty() {
            push_step(
                &mut phases,
                ExecutionPhase::SystemPrerequisites,
                Step::workflow(Operation::AptBootstrapPackages {
                    packages: prerequisites.iter().map(|s| (*s).to_owned()).collect(),
                }),
            );
        }

        for manager in &managers {
            let op = match manager {
                ManagerBootstrap::Flatpak => Operation::FlatpakEnsureFlathub,
                ManagerBootstrap::Rustup => Operation::RustupBootstrap,
                ManagerBootstrap::CargoBinstall => {
                    Operation::CargoBinstallBootstrap(CargoBinstallBootstrapOperation::new(platform.architecture))
                }
                ManagerBootstrap::Fnm => Operation::FnmBootstrap,
                ManagerBootstrap::Uv => Operation::UvBootstrap,
            };
            push_step(&mut phases, ExecutionPhase::ManagerBootstraps, Step::workflow(op));
        }

        let mut final_steps = Vec::new();
        for (phase, steps) in phases {
            if !steps.is_empty() {
                final_steps.push(Step::phase(phase));
                final_steps.extend(steps);
            }
        }
        final_steps.push(Step::summary());

        Ok(final_steps)
    }

    fn push_step(phases: &mut [(ExecutionPhase, Vec<Step>)], phase: ExecutionPhase, step: Step) {
        phases
            .iter_mut()
            .find(|(p, _)| *p == phase)
            .expect("phase exists")
            .1
            .push(step);
    }

    fn plan_repository(
        repository: &crate::config::Repository,
        platform: &Platform,
        identity: crate::config::PlatformIdentity,
    ) -> Result<AptRepositoryOperation> {
        let resolved = repository.resolve_for_platform(0, platform, identity)?;
        let layout = if let Some(path) = &repository.path {
            AptRepositorySourceLayout::ExactPath(AptRepositoryPath::parse(path)?)
        } else {
            let suite_token = resolved.suite.as_ref().expect("validated suite/components repository");
            AptRepositorySourceLayout::SuiteComponents {
                suite: AptRepositoryToken::parse(suite_token.as_str())?,
                components: repository
                    .components
                    .as_ref()
                    .expect("validated suite/components repository")
                    .iter()
                    .map(|component| AptRepositoryToken::parse(component.as_str()))
                    .collect::<Result<Vec<_>>>()?,
            }
        };
        let stem = repository.filename_stem();
        AptRepositoryOperation::new(
            repository.name.clone(),
            stem,
            repository.key.clone(),
            resolved.source_url.clone(),
            platform.architecture,
            layout,
        )
    }

    fn plan_binary(
        binary: &crate::config::BinaryPackage,
        architecture: Architecture,
        mode: BinaryPackageMode,
    ) -> Result<BinaryPackageOperation> {
        let source = match binary.source.resolve_native(architecture)? {
            ResolvedNativeBinary::Github { repository, selector } => BinarySourceOperation::GithubLatest {
                repository: GithubRepository::parse(repository.to_owned())?,
                selector: BinaryPackageSelector::new(selector.to_owned())?,
                sha256: None,
            },
            ResolvedNativeBinary::Url { url, sha256 } => BinarySourceOperation::ChecksummedUrl {
                url: url.clone(),
                sha256: BinarySha256::parse(sha256.as_str())?,
            },
        };
        BinaryPackageOperation::new(
            binary.name.clone(),
            match binary.format {
                BinaryFormat::Deb => BinaryPackageFormat::Deb,
                BinaryFormat::Appimage => BinaryPackageFormat::AppImage,
            },
            binary.commands.clone(),
            architecture,
            source,
            mode,
        )
    }

    fn plan_system_states(
        config: &Config,
        platform: &Platform,
        phases: &mut [(ExecutionPhase, Vec<Step>)],
        needs_apt_refresh: &mut bool,
    ) {
        let Some(system) = &config.system else { return };
        if let Some(state) = system.apt.as_ref().and_then(|apt| apt.unattended_upgrades) {
            push_step(
                phases,
                ExecutionPhase::SystemPackageStates,
                Step::workflow(Operation::UnattendedUpgrades(UnattendedUpgradesOperation::new(
                    enabled(state),
                ))),
            );
            *needs_apt_refresh = true;
        }
        let Some(ubuntu) = &system.ubuntu else { return };
        let ubuntu_family = platform.upstream == "ubuntu";
        if let Some(state) = ubuntu.snap {
            if ubuntu_family {
                *needs_apt_refresh = true;
                push_step(
                    phases,
                    ExecutionPhase::SystemPackageStates,
                    Step::workflow(Operation::UbuntuSnap(UbuntuSnapOperation::new(enabled(state)))),
                );
            } else {
                push_step(
                    phases,
                    ExecutionPhase::SystemPackageStates,
                    Step::skip(
                        runner::SkippedAction::UbuntuSnap,
                        runner::SkipReason::RequiresUbuntuFamily,
                    ),
                );
            }
        }
        if let Some(state) = ubuntu.codecs {
            if ubuntu_family {
                *needs_apt_refresh = true;
                if state == InstalledState::Installed {
                    push_step(
                        phases,
                        ExecutionPhase::SystemPackageStates,
                        Step::workflow(Operation::AptPackages {
                            packages: vec!["ubuntu-restricted-extras".into()],
                        }),
                    );
                }
            } else {
                push_step(
                    phases,
                    ExecutionPhase::SystemPackageStates,
                    Step::skip(
                        runner::SkippedAction::UbuntuCodecs,
                        runner::SkipReason::RequiresUbuntuFamily,
                    ),
                );
            }
        }
    }

    fn plan_tools(
        config: &Config,
        platform: &Platform,
        phases: &mut [(ExecutionPhase, Vec<Step>)],
        prerequisites: &mut BTreeSet<&'static str>,
        managers: &mut BTreeSet<ManagerBootstrap>,
    ) -> Result<()> {
        let Some(tools) = &config.tools else { return Ok(()) };
        if let Some(selector) = tools.rust.as_deref() {
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            managers.insert(ManagerBootstrap::Rustup);
            push_step(
                phases,
                ExecutionPhase::LanguageToolchains,
                Step::workflow(Operation::RustToolchain(RustToolchainOperation::new(
                    rust_selector_main(selector),
                    platform.architecture,
                    ToolMutationMode::EnsurePresent,
                )?)),
            );
        }
        if let Some(selector) = tools.go.as_deref() {
            prerequisites.extend(["ca-certificates", "curl", "tar", "xz-utils"]);
            push_step(
                phases,
                ExecutionPhase::LanguageToolchains,
                Step::workflow(Operation::GoToolchain(GoToolchainOperation::new(
                    go_selector_main(selector),
                    platform.architecture,
                    ToolMutationMode::EnsurePresent,
                )?)),
            );
        }
        if let Some(selector) = tools.node.as_deref() {
            prerequisites.extend(["ca-certificates", "curl"]);
            managers.insert(ManagerBootstrap::Fnm);
            push_step(
                phases,
                ExecutionPhase::LanguageToolchains,
                Step::workflow(Operation::NodeToolchain(NodeToolchainOperation::new(
                    node_selector_main(selector),
                    platform.architecture,
                    ToolMutationMode::EnsurePresent,
                )?)),
            );
        }
        if let Some(selector) = &tools.python {
            prerequisites.extend(["ca-certificates", "curl"]);
            managers.insert(ManagerBootstrap::Uv);
            push_step(
                phases,
                ExecutionPhase::LanguageToolchains,
                Step::workflow(Operation::PythonToolchain(PythonToolchainOperation::new(
                    selector.clone(),
                    platform.architecture,
                )?)),
            );
        }
        Ok(())
    }

    fn plan_integrations(config: &Config, phases: &mut [(ExecutionPhase, Vec<Step>)]) -> Result<()> {
        let Some(integrations) = &config.integrations else {
            return Ok(());
        };
        if let Some(docker) = &integrations.docker {
            if docker.add_user_to_group == Some(true) {
                push_step(
                    phases,
                    ExecutionPhase::Integrations,
                    Step::workflow(Operation::DockerGroup),
                );
            }
            if let Some(logging) = &docker.logging {
                push_step(
                    phases,
                    ExecutionPhase::Integrations,
                    Step::workflow(Operation::DockerLocalLog(DockerLocalLogOperation::new(
                        logging.max_size.clone(),
                    )?)),
                );
            }
        }
        if integrations
            .virtualbox
            .as_ref()
            .is_some_and(|virtualbox| virtualbox.add_user_to_group == Some(true))
        {
            push_step(
                phases,
                ExecutionPhase::Integrations,
                Step::workflow(Operation::VirtualBoxGroup),
            );
        }
        if let Some(extensions) = integrations.vscode.as_ref().map(|vscode| vscode.extensions.clone()) {
            push_step(
                phases,
                ExecutionPhase::Integrations,
                Step::workflow(Operation::VsCodeExtensionSet(VsCodeExtensionOperation::new(
                    extensions,
                )?)),
            );
        }
        Ok(())
    }

    fn plan_desktop(
        config: &Config,
        platform: &Platform,
        phases: &mut [(ExecutionPhase, Vec<Step>)],
        prerequisites: &mut BTreeSet<&'static str>,
    ) -> Result<()> {
        let Some(desktop) = &config.desktop else {
            return Ok(());
        };
        let target = match platform.desktop.as_str() {
            "gnome" => DesktopEnvironment::Gnome,
            "cinnamon" => DesktopEnvironment::Cinnamon,
            _ => unreachable!("platform validation rejects unsupported desktop intent"),
        };
        prerequisites.extend(["dconf-cli", "libglib2.0-bin"]);
        if let Some(theme) = desktop.theme {
            push_step(
                phases,
                ExecutionPhase::Desktop,
                Step::workflow(Operation::DesktopSetting(DesktopSettingOperation::new(
                    target,
                    DesktopSetting::Theme(match theme {
                        Theme::Light => DesktopTheme::Light,
                        Theme::Dark => DesktopTheme::Dark,
                    }),
                )?)),
            );
        }
        if let Some(executable) = &desktop.terminal {
            push_step(
                phases,
                ExecutionPhase::Desktop,
                Step::workflow(Operation::DesktopSetting(DesktopSettingOperation::new(
                    target,
                    DesktopSetting::Terminal(executable.clone()),
                )?)),
            );
        }
        if let Some(idle) = &desktop.idle {
            if let Some(timeout) = &idle.timeout {
                push_step(
                    phases,
                    ExecutionPhase::Desktop,
                    Step::workflow(Operation::DesktopSetting(DesktopSettingOperation::new(
                        target,
                        DesktopSetting::IdleTimeoutSeconds(duration_seconds(timeout)?),
                    )?)),
                );
            }
            if let Some(enabled) = idle.dim {
                push_step(
                    phases,
                    ExecutionPhase::Desktop,
                    Step::workflow(Operation::DesktopSetting(DesktopSettingOperation::new(
                        target,
                        DesktopSetting::IdleDim(enabled),
                    )?)),
                );
            }
        }
        if target == DesktopEnvironment::Gnome {
            if let Some(gnome) = &desktop.gnome {
                if let Some(extensions) = &gnome.extensions {
                    prerequisites.insert("gnome-shell");
                    push_step(
                        phases,
                        ExecutionPhase::Desktop,
                        Step::workflow(Operation::GnomeExtensions(GnomeExtensionsOperation::new(
                            extensions.clone(),
                        )?)),
                    );
                }
                if gnome.dock == Some(true) {
                    prerequisites.insert("gnome-shell");
                    push_step(
                        phases,
                        ExecutionPhase::Desktop,
                        Step::workflow(Operation::GnomeDock(GnomeDockOperation::new())),
                    );
                }
                if gnome.rounded_corners == Some(true) {
                    prerequisites.insert("gnome-shell");
                    push_step(
                        phases,
                        ExecutionPhase::Desktop,
                        Step::workflow(Operation::GnomeRoundedCorners(GnomeRoundedCornersOperation::new())),
                    );
                }
            }
        }
        Ok(())
    }

    fn plan_updates(
        config: &Config,
        platform: &Platform,
        phases: &mut [(ExecutionPhase, Vec<Step>)],
        needs_apt_refresh: &mut bool,
    ) -> Result<()> {
        let Some(updates) = &config.updates else {
            return Ok(());
        };
        let packages = config.packages.as_ref();
        let tools = config.tools.as_ref();
        if let Some(policy) = updates.apt {
            *needs_apt_refresh = true;
            push_step(
                phases,
                ExecutionPhase::Updates,
                Step::workflow(Operation::AptUpgrade {
                    policy: match policy {
                        AptUpdate::Standard => AptUpgradePolicy::Standard,
                        AptUpdate::Full => AptUpgradePolicy::Full,
                    },
                }),
            );
        }
        if updates.flatpak == Some(true) {
            push_step(
                phases,
                ExecutionPhase::Updates,
                Step::workflow(Operation::FlatpakUpdateApps {
                    refs: packages
                        .and_then(|packages| packages.flatpak.clone())
                        .expect("validated update target"),
                }),
            );
        }
        if let Some(tool_updates) = &updates.tools {
            if tool_updates.rust == Some(true) {
                push_step(
                    phases,
                    ExecutionPhase::Updates,
                    Step::workflow(Operation::RustToolchain(RustToolchainOperation::new(
                        rust_selector_main(
                            tools
                                .and_then(|tools| tools.rust.as_deref())
                                .expect("validated update target"),
                        ),
                        platform.architecture,
                        ToolMutationMode::UpdateMoving,
                    )?)),
                );
            }
            if tool_updates.go == Some(true) {
                push_step(
                    phases,
                    ExecutionPhase::Updates,
                    Step::workflow(Operation::GoToolchain(GoToolchainOperation::new(
                        go_selector_main(
                            tools
                                .and_then(|tools| tools.go.as_deref())
                                .expect("validated update target"),
                        ),
                        platform.architecture,
                        ToolMutationMode::UpdateMoving,
                    )?)),
                );
            }
            if tool_updates.node == Some(true) {
                push_step(
                    phases,
                    ExecutionPhase::Updates,
                    Step::workflow(Operation::NodeToolchain(NodeToolchainOperation::new(
                        node_selector_main(
                            tools
                                .and_then(|tools| tools.node.as_deref())
                                .expect("validated update target"),
                        ),
                        platform.architecture,
                        ToolMutationMode::UpdateMoving,
                    )?)),
                );
            }
        }
        if let Some(package_updates) = &updates.packages {
            if package_updates.cargo == Some(true) {
                push_step(
                    phases,
                    ExecutionPhase::Updates,
                    Step::workflow(Operation::CargoPackageSet(CargoPackageOperation::new(
                        packages
                            .and_then(|packages| packages.cargo.clone())
                            .expect("validated update target"),
                        CargoPackageMode::UpdateCurrent,
                    )?)),
                );
            }
            if package_updates.npm == Some(true) {
                push_step(
                    phases,
                    ExecutionPhase::Updates,
                    Step::workflow(Operation::NpmPackageSet(NpmPackageOperation::new(
                        packages
                            .and_then(|packages| packages.npm.clone())
                            .expect("validated update target"),
                        NpmPackageMode::UpdateCurrent,
                    )?)),
                );
            }
            if package_updates.binaries == Some(true) {
                if let Some(binaries) = packages.and_then(|packages| packages.binaries.as_ref()) {
                    for binary in binaries {
                        let is_github = matches!(
                            binary.source.resolve_native(platform.architecture)?,
                            ResolvedNativeBinary::Github { .. }
                        );
                        if is_github {
                            let planned = plan_binary(binary, platform.architecture, BinaryPackageMode::Update)?;
                            push_step(
                                phases,
                                ExecutionPhase::Updates,
                                Step::workflow(Operation::BinaryPackage(planned)),
                            );
                        }
                    }
                }
            }
        }
        if updates.fonts == Some(true) {
            push_step(
                phases,
                ExecutionPhase::Updates,
                Step::workflow(Operation::NerdFonts(NerdFontsOperation::new(
                    config
                        .fonts
                        .as_ref()
                        .and_then(|fonts| fonts.nerd.clone())
                        .expect("validated update target"),
                    NerdFontsMode::Update,
                )?)),
            );
        }
        Ok(())
    }

    fn rust_selector_main(value: &str) -> RustToolchainSelector {
        match value {
            "stable" => RustToolchainSelector::Stable,
            "beta" => RustToolchainSelector::Beta,
            "nightly" => RustToolchainSelector::Nightly,
            value if value.starts_with("nightly-") => RustToolchainSelector::DatedNightly(value.to_owned()),
            value => RustToolchainSelector::Version(value.to_owned()),
        }
    }

    fn go_selector_main(value: &str) -> GoToolchainSelector {
        if value == "latest" {
            GoToolchainSelector::Latest
        } else {
            GoToolchainSelector::Version(value.to_owned())
        }
    }

    fn node_selector_main(value: &str) -> NodeToolchainSelector {
        match value {
            "lts" => NodeToolchainSelector::Lts,
            "latest" => NodeToolchainSelector::Latest,
            value => NodeToolchainSelector::Version(value.to_owned()),
        }
    }

    fn enabled(state: EnabledDisabled) -> bool {
        match state {
            EnabledDisabled::Enabled => true,
            EnabledDisabled::Disabled => false,
        }
    }

    fn duration_seconds(value: &str) -> Result<u32> {
        let (number, multiplier) = if let Some(number) = value.strip_suffix('s') {
            (number, 1_u64)
        } else if let Some(number) = value.strip_suffix('m') {
            (number, 60)
        } else {
            (value.strip_suffix('h').context("invalid desktop idle duration")?, 3600)
        };
        number
            .parse::<u64>()
            .context("invalid desktop idle duration")?
            .checked_mul(multiplier)
            .and_then(|seconds| u32::try_from(seconds).ok())
            .context("desktop idle duration exceeds the supported uint32 range")
    }
}
