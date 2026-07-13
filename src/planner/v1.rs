use crate::{
    config::v1::{
        AptComponent, AptSources, AptUpdate, AssetSelector, ConfigV1, DirectFormat, DirectPackage,
        Theme,
    },
    platform::{Architecture, Platform},
};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanV1 {
    pub actions: Vec<PlannedAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedAction {
    Prepare(Preparation),
    System(SystemAction),
    RemovePackages(Vec<String>),
    Repository(AptRepository),
    AptPackages(Vec<String>),
    Flatpak(FlatpakInstall),
    Tool(ToolInstall),
    CargoPackages(Vec<String>),
    NpmPackages(Vec<String>),
    DirectPackage(DirectPackageIntent),
    NerdFonts(Vec<String>),
    Dotfiles(DotfilesIntent),
    Integration(IntegrationAction),
    Desktop(DesktopAction),
    Update(UpdateAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preparation {
    pub prerequisites: BTreeSet<Prerequisite>,
    pub apt_metadata: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prerequisite {
    NetworkDownload,
    AptRepositorySupport,
    FlatpakFlathub,
    RustupCargoBinstall,
    GoArchives,
    FnmNpm,
    Uv,
    Stow,
    DirectDeb,
    DirectAppImage,
    NerdFonts,
    DockerIntegration,
    VirtualBoxIntegration,
    VsCodeIntegration,
    GnomeTools,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemAction {
    AptSources(AptSourcesIntent),
    EnsureAdmin { enabled: bool },
    UnattendedUpgrades { enabled: bool },
    UbuntuSnap { enabled: bool },
    UbuntuCodecs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AptSourcesIntent {
    Preserve,
    Managed {
        distro: String,
        upstream: String,
        codename: String,
        components: Option<Vec<AptComponent>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AptRepository {
    pub name: String,
    pub key_url: String,
    pub source_url: String,
    pub suite: String,
    pub components: Vec<String>,
    pub architecture: String,
    pub keyring_path: PathBuf,
    pub source_list_path: PathBuf,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatpakInstall {
    pub remote: FlatpakRemote,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatpakRemote {
    Flathub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolInstall {
    Rust {
        selector: RustSelector,
        target: String,
    },
    Go {
        selector: GoSelector,
        archive_architecture: String,
    },
    Node {
        selector: NodeSelector,
    },
    Python {
        version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustSelector {
    Stable,
    Beta,
    Nightly,
    DatedNightly(String),
    Version(String),
}

impl RustSelector {
    pub fn is_moving(&self) -> bool {
        matches!(self, Self::Stable | Self::Beta | Self::Nightly)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoSelector {
    Latest,
    Version(String),
}

impl GoSelector {
    pub fn is_moving(&self) -> bool {
        matches!(self, Self::Latest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeSelector {
    Lts,
    Latest,
    Version(String),
}

impl NodeSelector {
    pub fn is_moving(&self) -> bool {
        matches!(self, Self::Lts | Self::Latest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectPackageIntent {
    pub name: String,
    pub format: DirectFormat,
    pub provides: Vec<String>,
    pub source: GithubReleaseIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubReleaseIntent {
    pub repository: String,
    pub release: GithubRelease,
    pub architecture: Architecture,
    pub selector: AssetSelector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubRelease {
    Latest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotfilesIntent {
    pub root: PathBuf,
    pub packages: Vec<String>,
    pub conflict_policy: DotfilesConflictPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotfilesConflictPolicy {
    BackupBeforeStow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationAction {
    DockerGroup,
    DockerLocalLog { max_size: Option<String> },
    VirtualBoxGroup,
    VsCodeExtensions(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopAction {
    Theme(Theme),
    Terminal(String),
    IdleTimeout(String),
    IdleDim { enabled: bool },
    GnomeExtensions(Vec<String>),
    GnomeDock,
    GnomeRoundedCorners,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateAction {
    Apt {
        policy: AptUpdatePolicy,
        target: AptUpdateTarget,
    },
    Flatpak {
        refs: Vec<String>,
        scope: FlatpakUpdateScope,
    },
    Tool(ToolUpdate),
    Cargo {
        packages: Vec<String>,
    },
    Npm {
        packages: Vec<String>,
    },
    Direct {
        packages: Vec<DirectPackageIntent>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AptUpdatePolicy {
    Standard,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AptUpdateTarget {
    SystemPackages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatpakUpdateScope {
    ConfiguredRefsAndRequiredRuntimes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolUpdate {
    Rust {
        selector: RustSelector,
        target: String,
    },
    Go {
        selector: GoSelector,
        archive_architecture: String,
    },
    Node {
        selector: NodeSelector,
    },
}

/// Builds schema-v1 intents only. Desktop controls unsupported by the resolved desktop are omitted.
pub fn plan(config: &ConfigV1, platform: &Platform, dotfiles_root: &Path) -> Result<PlanV1> {
    config.validate_for_platform(platform)?;

    let packages = config.packages.as_ref();
    let tools = config.tools.as_ref();
    let npm = packages
        .and_then(|packages| packages.npm.as_deref())
        .unwrap_or_default();
    if !npm.is_empty() && tools.and_then(|tools| tools.node.as_ref()).is_none() {
        bail!("packages.npm: requires tools.node");
    }

    let mut actions = Vec::new();
    let mut prerequisites = BTreeSet::new();
    let mut apt_metadata = false;

    plan_system(config, platform, &mut actions, &mut apt_metadata)?;

    if let Some(remove) = non_empty(packages.and_then(|packages| packages.remove.as_ref())) {
        apt_metadata = true;
        actions.push(PlannedAction::RemovePackages(remove.clone()));
    }

    if let Some(repositories) = packages.and_then(|packages| packages.repositories.as_ref()) {
        for (index, repository) in repositories.iter().enumerate() {
            prerequisites.extend([
                Prerequisite::NetworkDownload,
                Prerequisite::AptRepositorySupport,
            ]);
            apt_metadata = true;
            let source_url = repository
                .source
                .urls
                .select(&platform.distro)
                .with_context(|| format!("packages.repositories[{index}].source.urls"))?;
            let suite = if repository.source.suite == "system" {
                if platform.codename.trim().is_empty() {
                    bail!("packages.repositories[{index}].source.suite: system requires a non-empty platform codename");
                }
                platform.codename.clone()
            } else {
                repository.source.suite.clone()
            };
            let stem = repository.sanitized_name();
            actions.push(PlannedAction::Repository(AptRepository {
                name: repository.name.clone(),
                key_url: repository.key.as_str().to_owned(),
                source_url: source_url.to_owned(),
                suite,
                components: repository.source.components.clone(),
                architecture: platform.architecture.debian().into(),
                keyring_path: PathBuf::from(format!("/etc/apt/keyrings/cozydot-{stem}.gpg")),
                source_list_path: PathBuf::from(format!(
                    "/etc/apt/sources.list.d/cozydot-{stem}.list"
                )),
                packages: repository.packages.clone(),
            }));
        }
    }
    if let Some(apt) = non_empty(packages.and_then(|packages| packages.apt.as_ref())) {
        prerequisites.insert(Prerequisite::NetworkDownload);
        apt_metadata = true;
        actions.push(PlannedAction::AptPackages(apt.clone()));
    }

    if let Some(refs) = non_empty(packages.and_then(|packages| packages.flatpak.as_ref())) {
        prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::FlatpakFlathub]);
        actions.push(PlannedAction::Flatpak(FlatpakInstall {
            remote: FlatpakRemote::Flathub,
            refs: refs.clone(),
        }));
    }

    plan_tools(tools, platform, &mut actions, &mut prerequisites);

    if let Some(cargo) = non_empty(packages.and_then(|packages| packages.cargo.as_ref())) {
        prerequisites.extend([
            Prerequisite::NetworkDownload,
            Prerequisite::RustupCargoBinstall,
        ]);
        actions.push(PlannedAction::CargoPackages(cargo.clone()));
    }
    if !npm.is_empty() {
        prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::FnmNpm]);
        actions.push(PlannedAction::NpmPackages(npm.to_vec()));
    }

    let direct = packages
        .and_then(|packages| packages.direct.as_deref())
        .unwrap_or_default();
    let direct_intents = direct
        .iter()
        .map(|package| direct_intent(package, platform.architecture))
        .collect::<Result<Vec<_>>>()?;
    for (package, intent) in direct.iter().zip(direct_intents.iter()) {
        prerequisites.insert(Prerequisite::NetworkDownload);
        prerequisites.insert(match package.format {
            DirectFormat::Deb => Prerequisite::DirectDeb,
            DirectFormat::Appimage => Prerequisite::DirectAppImage,
        });
        actions.push(PlannedAction::DirectPackage(intent.clone()));
    }

    if let Some(fonts) = non_empty(config.fonts.as_ref().and_then(|fonts| fonts.nerd.as_ref())) {
        prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::NerdFonts]);
        actions.push(PlannedAction::NerdFonts(fonts.clone()));
    }

    if let Some(dotfiles) = &config.dotfiles {
        prerequisites.insert(Prerequisite::Stow);
        actions.push(PlannedAction::Dotfiles(DotfilesIntent {
            root: dotfiles_root.to_path_buf(),
            packages: dotfiles.packages.clone(),
            conflict_policy: DotfilesConflictPolicy::BackupBeforeStow,
        }));
    }

    plan_integrations(config, &mut actions, &mut prerequisites);
    plan_desktop(config, platform, &mut actions, &mut prerequisites);
    plan_updates(
        config,
        platform,
        &direct_intents,
        &mut actions,
        &mut prerequisites,
        &mut apt_metadata,
    );

    if !prerequisites.is_empty() {
        apt_metadata = true;
    }
    if apt_metadata || !prerequisites.is_empty() {
        actions.insert(
            0,
            PlannedAction::Prepare(Preparation {
                prerequisites,
                apt_metadata,
            }),
        );
    }
    Ok(PlanV1 { actions })
}

fn plan_system(
    config: &ConfigV1,
    platform: &Platform,
    actions: &mut Vec<PlannedAction>,
    apt_metadata: &mut bool,
) -> Result<()> {
    let Some(system) = &config.system else {
        return Ok(());
    };
    if let Some(apt) = &system.apt {
        if let Some(sources) = &apt.sources {
            let intent = match sources {
                AptSources::Preserve => AptSourcesIntent::Preserve,
                AptSources::Managed => {
                    if platform.codename.trim().is_empty() {
                        bail!("system.apt.sources: managed requires a non-empty platform codename");
                    }
                    *apt_metadata = true;
                    AptSourcesIntent::Managed {
                        distro: platform.distro.clone(),
                        upstream: platform.upstream.clone(),
                        codename: platform.codename.clone(),
                        components: apt.components.clone(),
                    }
                }
            };
            actions.push(PlannedAction::System(SystemAction::AptSources(intent)));
        }
    }
    if let Some(enabled) = system.ensure_admin {
        actions.push(PlannedAction::System(SystemAction::EnsureAdmin { enabled }));
    }
    if let Some(enabled) = system.apt.as_ref().and_then(|apt| apt.unattended_upgrades) {
        *apt_metadata = true;
        actions.push(PlannedAction::System(SystemAction::UnattendedUpgrades {
            enabled,
        }));
    }
    if platform.upstream == "ubuntu" {
        if let Some(ubuntu) = &system.ubuntu {
            if let Some(enabled) = ubuntu.snap {
                *apt_metadata = true;
                actions.push(PlannedAction::System(SystemAction::UbuntuSnap { enabled }));
            }
            if ubuntu.codecs == Some(true) {
                *apt_metadata = true;
                actions.push(PlannedAction::System(SystemAction::UbuntuCodecs));
            }
        }
    }
    Ok(())
}

fn plan_tools(
    tools: Option<&crate::config::v1::Tools>,
    platform: &Platform,
    actions: &mut Vec<PlannedAction>,
    prerequisites: &mut BTreeSet<Prerequisite>,
) {
    let Some(tools) = tools else { return };
    if let Some(selector) = tools.rust.as_deref() {
        prerequisites.extend([
            Prerequisite::NetworkDownload,
            Prerequisite::RustupCargoBinstall,
        ]);
        actions.push(PlannedAction::Tool(ToolInstall::Rust {
            selector: rust_selector(selector),
            target: platform.architecture.rust_target().into(),
        }));
    }
    if let Some(selector) = tools.go.as_deref() {
        prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::GoArchives]);
        actions.push(PlannedAction::Tool(ToolInstall::Go {
            selector: go_selector(selector),
            archive_architecture: platform.architecture.go_archive().into(),
        }));
    }
    if let Some(selector) = tools.node.as_deref() {
        prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::FnmNpm]);
        actions.push(PlannedAction::Tool(ToolInstall::Node {
            selector: node_selector(selector),
        }));
    }
    if let Some(version) = tools.python.as_ref() {
        prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::Uv]);
        actions.push(PlannedAction::Tool(ToolInstall::Python {
            version: version.clone(),
        }));
    }
}

fn direct_intent(
    package: &DirectPackage,
    architecture: Architecture,
) -> Result<DirectPackageIntent> {
    let selector = package.source.assets.get(architecture).with_context(|| {
        format!(
            "packages.direct source assets: missing {} selector for {:?}",
            architecture.canonical(),
            package.name
        )
    })?;
    Ok(DirectPackageIntent {
        name: package.name.clone(),
        format: package.format.clone(),
        provides: package.provides.clone(),
        source: GithubReleaseIntent {
            repository: package.source.repository.clone(),
            release: GithubRelease::Latest,
            architecture,
            selector: selector.clone(),
        },
    })
}

fn plan_integrations(
    config: &ConfigV1,
    actions: &mut Vec<PlannedAction>,
    prerequisites: &mut BTreeSet<Prerequisite>,
) {
    let Some(integrations) = &config.integrations else {
        return;
    };
    if let Some(docker) = &integrations.docker {
        if docker.add_user_to_group == Some(true) {
            prerequisites.insert(Prerequisite::DockerIntegration);
            actions.push(PlannedAction::Integration(IntegrationAction::DockerGroup));
        }
        if docker.local_log_driver == Some(true) {
            prerequisites.insert(Prerequisite::DockerIntegration);
            actions.push(PlannedAction::Integration(
                IntegrationAction::DockerLocalLog {
                    max_size: docker.max_log_size.clone(),
                },
            ));
        }
    }
    if integrations
        .virtualbox
        .as_ref()
        .is_some_and(|virtualbox| virtualbox.add_user_to_group == Some(true))
    {
        prerequisites.insert(Prerequisite::VirtualBoxIntegration);
        actions.push(PlannedAction::Integration(
            IntegrationAction::VirtualBoxGroup,
        ));
    }
    if let Some(extensions) = non_empty(
        integrations
            .vscode
            .as_ref()
            .and_then(|vscode| vscode.extensions.as_ref()),
    ) {
        prerequisites.insert(Prerequisite::VsCodeIntegration);
        actions.push(PlannedAction::Integration(
            IntegrationAction::VsCodeExtensions(extensions.clone()),
        ));
    }
}

fn plan_desktop(
    config: &ConfigV1,
    platform: &Platform,
    actions: &mut Vec<PlannedAction>,
    prerequisites: &mut BTreeSet<Prerequisite>,
) {
    let Some(desktop) = &config.desktop else {
        return;
    };
    if !matches!(platform.desktop.as_str(), "gnome" | "cinnamon") {
        return;
    }
    let start = actions.len();
    if let Some(theme) = &desktop.theme {
        actions.push(PlannedAction::Desktop(DesktopAction::Theme(theme.clone())));
    }
    if let Some(terminal) = &desktop.terminal {
        actions.push(PlannedAction::Desktop(DesktopAction::Terminal(
            terminal.clone(),
        )));
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = &idle.timeout {
            actions.push(PlannedAction::Desktop(DesktopAction::IdleTimeout(
                timeout.clone(),
            )));
        }
        if let Some(enabled) = idle.dim {
            actions.push(PlannedAction::Desktop(DesktopAction::IdleDim { enabled }));
        }
    }
    if platform.desktop == "gnome" {
        if let Some(gnome) = &desktop.gnome {
            if let Some(extensions) = non_empty(gnome.extensions.as_ref()) {
                actions.push(PlannedAction::Desktop(DesktopAction::GnomeExtensions(
                    extensions.clone(),
                )));
            }
            if gnome.dock == Some(true) {
                actions.push(PlannedAction::Desktop(DesktopAction::GnomeDock));
            }
            if gnome.rounded_corners == Some(true) {
                actions.push(PlannedAction::Desktop(DesktopAction::GnomeRoundedCorners));
            }
        }
    }
    if platform.desktop == "gnome" && actions.len() != start {
        prerequisites.insert(Prerequisite::GnomeTools);
    }
}

fn plan_updates(
    config: &ConfigV1,
    platform: &Platform,
    direct: &[DirectPackageIntent],
    actions: &mut Vec<PlannedAction>,
    prerequisites: &mut BTreeSet<Prerequisite>,
    apt_metadata: &mut bool,
) {
    let Some(updates) = &config.updates else {
        return;
    };
    match updates.apt {
        Some(AptUpdate::Standard) => {
            *apt_metadata = true;
            prerequisites.insert(Prerequisite::NetworkDownload);
            actions.push(PlannedAction::Update(UpdateAction::Apt {
                policy: AptUpdatePolicy::Standard,
                target: AptUpdateTarget::SystemPackages,
            }));
        }
        Some(AptUpdate::Full) => {
            *apt_metadata = true;
            prerequisites.insert(Prerequisite::NetworkDownload);
            actions.push(PlannedAction::Update(UpdateAction::Apt {
                policy: AptUpdatePolicy::Full,
                target: AptUpdateTarget::SystemPackages,
            }));
        }
        Some(AptUpdate::Off) | None => {}
    }

    let packages = config.packages.as_ref();
    let tools = config.tools.as_ref();
    if updates.flatpak == Some(true) {
        if let Some(refs) = non_empty(packages.and_then(|packages| packages.flatpak.as_ref())) {
            prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::FlatpakFlathub]);
            actions.push(PlannedAction::Update(UpdateAction::Flatpak {
                refs: refs.clone(),
                scope: FlatpakUpdateScope::ConfiguredRefsAndRequiredRuntimes,
            }));
        }
    }
    if let Some(tool_updates) = &updates.tools {
        if tool_updates.rust == Some(true) {
            if let Some(selector) = tools.and_then(|tools| tools.rust.as_deref()) {
                prerequisites.extend([
                    Prerequisite::NetworkDownload,
                    Prerequisite::RustupCargoBinstall,
                ]);
                actions.push(PlannedAction::Update(UpdateAction::Tool(
                    ToolUpdate::Rust {
                        selector: rust_selector(selector),
                        target: platform.architecture.rust_target().into(),
                    },
                )));
            }
        }
        if tool_updates.go == Some(true) {
            if let Some(selector) = tools.and_then(|tools| tools.go.as_deref()) {
                prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::GoArchives]);
                actions.push(PlannedAction::Update(UpdateAction::Tool(ToolUpdate::Go {
                    selector: go_selector(selector),
                    archive_architecture: platform.architecture.go_archive().into(),
                })));
            }
        }
        if tool_updates.node == Some(true) {
            if let Some(selector) = tools.and_then(|tools| tools.node.as_deref()) {
                prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::FnmNpm]);
                actions.push(PlannedAction::Update(UpdateAction::Tool(
                    ToolUpdate::Node {
                        selector: node_selector(selector),
                    },
                )));
            }
        }
    }
    if let Some(package_updates) = &updates.packages {
        if package_updates.cargo == Some(true) {
            if let Some(packages) = non_empty(packages.and_then(|packages| packages.cargo.as_ref()))
            {
                prerequisites.extend([
                    Prerequisite::NetworkDownload,
                    Prerequisite::RustupCargoBinstall,
                ]);
                actions.push(PlannedAction::Update(UpdateAction::Cargo {
                    packages: packages.clone(),
                }));
            }
        }
        if package_updates.npm == Some(true) {
            if let Some(packages) = non_empty(packages.and_then(|packages| packages.npm.as_ref())) {
                prerequisites.extend([Prerequisite::NetworkDownload, Prerequisite::FnmNpm]);
                actions.push(PlannedAction::Update(UpdateAction::Npm {
                    packages: packages.clone(),
                }));
            }
        }
        if package_updates.direct == Some(true) && !direct.is_empty() {
            prerequisites.insert(Prerequisite::NetworkDownload);
            actions.push(PlannedAction::Update(UpdateAction::Direct {
                packages: direct.to_vec(),
            }));
        }
    }
}

fn rust_selector(value: &str) -> RustSelector {
    match value {
        "stable" => RustSelector::Stable,
        "beta" => RustSelector::Beta,
        "nightly" => RustSelector::Nightly,
        value if value.starts_with("nightly-") => RustSelector::DatedNightly(value.into()),
        value => RustSelector::Version(value.into()),
    }
}

fn go_selector(value: &str) -> GoSelector {
    if value == "latest" {
        GoSelector::Latest
    } else {
        GoSelector::Version(value.into())
    }
}

fn node_selector(value: &str) -> NodeSelector {
    match value {
        "lts" => NodeSelector::Lts,
        "latest" => NodeSelector::Latest,
        value => NodeSelector::Version(value.into()),
    }
}

fn non_empty<T>(values: Option<&Vec<T>>) -> Option<&Vec<T>> {
    values.filter(|values| !values.is_empty())
}
