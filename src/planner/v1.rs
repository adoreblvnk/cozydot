use crate::{
    config::v1::{
        AptComponent, AptSourceToken, AptSources, AptUpdate, AssetSelector, ConfigV1,
        ConfiguredRepositorySuite, DirectFormat, DirectPackage, HttpsUrl, Theme,
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
    Bootstrap(Bootstrap),
    System(SystemAction),
    AptMetadataRefresh,
    RemovePackages(Vec<String>),
    Repository(AptRepository),
    RepositoryPackages(AptRepositoryPackages),
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
pub struct Bootstrap {
    pub prerequisites: BTreeSet<Prerequisite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prerequisite {
    HttpsDownloader,
    OpenPgpRepositorySupport,
    FlatpakFlathub,
    Rustup,
    CargoBinstall,
    GoArchives,
    FnmNpm,
    Uv,
    Stow,
    DirectDeb,
    DirectAppImage,
    NerdFonts,
    GnomeTools,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemAction {
    AptSources(AptSourcesIntent),
    EnsureAdmin,
    UnattendedUpgrades { enabled: bool },
    UbuntuSnap { enabled: bool },
    UbuntuCodecs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AptSourcesIntent {
    Managed {
        distro: String,
        upstream: String,
        codename: AptSourceToken,
        components: Option<Vec<AptComponent>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AptRepository {
    pub name: String,
    pub key_url: HttpsUrl,
    pub source_url: HttpsUrl,
    pub suite: RepositorySuite,
    pub components: Vec<AptSourceToken>,
    pub architecture: Architecture,
    pub keyring_path: PathBuf,
    pub source_list_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositorySuite {
    ResolvedSystem(AptSourceToken),
    Fixed(AptSourceToken),
}

impl RepositorySuite {
    pub fn value(&self) -> &str {
        match self {
            Self::ResolvedSystem(value) | Self::Fixed(value) => value.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AptRepositoryPackages {
    pub repository: String,
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
        architecture: Architecture,
    },
    Go {
        selector: GoSelector,
        architecture: Architecture,
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

impl IntegrationAction {
    pub fn required_product(&self) -> ExistingProduct {
        match self {
            Self::DockerGroup | Self::DockerLocalLog { .. } => ExistingProduct::Docker,
            Self::VirtualBoxGroup => ExistingProduct::VirtualBox,
            Self::VsCodeExtensions(_) => ExistingProduct::VsCode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingProduct {
    Docker,
    VirtualBox,
    VsCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopAction {
    Theme {
        target: DesktopTarget,
        theme: Theme,
    },
    Terminal {
        target: DesktopTarget,
        executable: String,
    },
    IdleTimeout {
        target: DesktopTarget,
        timeout: String,
    },
    IdleDim {
        target: DesktopTarget,
        enabled: bool,
    },
    GnomeExtensions(Vec<String>),
    GnomeDock,
    GnomeRoundedCorners,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopTarget {
    Gnome,
    Cinnamon,
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
        architecture: Architecture,
    },
    Go {
        selector: GoSelector,
        architecture: Architecture,
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

    let mut prerequisites = BTreeSet::new();
    let mut preparation = Vec::new();
    let mut sources = Vec::new();
    let mut apt_consumers = Vec::new();
    let mut repository_consumers = Vec::new();
    let mut remaining = Vec::new();
    let mut updates = Vec::new();
    let mut needs_apt_metadata = false;

    plan_system(
        config,
        platform,
        &mut preparation,
        &mut sources,
        &mut apt_consumers,
        &mut needs_apt_metadata,
    )?;

    if let Some(repositories) = packages.and_then(|packages| packages.repositories.as_ref()) {
        for (index, repository) in repositories.iter().enumerate() {
            prerequisites.extend([
                Prerequisite::HttpsDownloader,
                Prerequisite::OpenPgpRepositorySupport,
            ]);
            let source_url = repository
                .source
                .urls
                .select_url(&platform.distro)
                .with_context(|| format!("packages.repositories[{index}].source.urls"))?;
            let suite = match &repository.source.suite {
                ConfiguredRepositorySuite::System => RepositorySuite::ResolvedSystem(
                    AptSourceToken::parse(&platform.codename).with_context(|| {
                        format!("packages.repositories[{index}].source.suite: invalid system platform codename")
                    })?,
                ),
                ConfiguredRepositorySuite::Fixed(suite) => RepositorySuite::Fixed(suite.clone()),
            };
            let stem = repository.sanitized_name();
            sources.push(PlannedAction::Repository(AptRepository {
                name: repository.name.clone(),
                key_url: repository.key.clone(),
                source_url: source_url.clone(),
                suite,
                components: repository.source.components.clone(),
                architecture: platform.architecture,
                keyring_path: PathBuf::from(format!("/etc/apt/keyrings/cozydot-{stem}.gpg")),
                source_list_path: PathBuf::from(format!(
                    "/etc/apt/sources.list.d/cozydot-{stem}.list"
                )),
            }));
            repository_consumers.push(PlannedAction::RepositoryPackages(AptRepositoryPackages {
                repository: repository.name.clone(),
                packages: repository.packages.clone(),
            }));
            needs_apt_metadata = true;
        }
    }
    if let Some(remove) = non_empty(packages.and_then(|packages| packages.remove.as_ref())) {
        apt_consumers.push(PlannedAction::RemovePackages(remove.clone()));
        needs_apt_metadata = true;
    }
    apt_consumers.append(&mut repository_consumers);
    if let Some(apt) = non_empty(packages.and_then(|packages| packages.apt.as_ref())) {
        apt_consumers.push(PlannedAction::AptPackages(apt.clone()));
        needs_apt_metadata = true;
    }

    if let Some(refs) = non_empty(packages.and_then(|packages| packages.flatpak.as_ref())) {
        prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::FlatpakFlathub]);
        remaining.push(PlannedAction::Flatpak(FlatpakInstall {
            remote: FlatpakRemote::Flathub,
            refs: refs.clone(),
        }));
    }

    plan_tools(tools, platform, &mut remaining, &mut prerequisites);

    if let Some(cargo) = non_empty(packages.and_then(|packages| packages.cargo.as_ref())) {
        prerequisites.extend([
            Prerequisite::HttpsDownloader,
            Prerequisite::Rustup,
            Prerequisite::CargoBinstall,
        ]);
        remaining.push(PlannedAction::CargoPackages(cargo.clone()));
    }
    if !npm.is_empty() {
        prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::FnmNpm]);
        remaining.push(PlannedAction::NpmPackages(npm.to_vec()));
    }

    let direct = packages
        .and_then(|packages| packages.direct.as_deref())
        .unwrap_or_default();
    let direct_intents = direct
        .iter()
        .map(|package| direct_intent(package, platform.architecture))
        .collect::<Result<Vec<_>>>()?;
    for (package, intent) in direct.iter().zip(direct_intents.iter()) {
        prerequisites.insert(Prerequisite::HttpsDownloader);
        prerequisites.insert(match package.format {
            DirectFormat::Deb => Prerequisite::DirectDeb,
            DirectFormat::Appimage => Prerequisite::DirectAppImage,
        });
        remaining.push(PlannedAction::DirectPackage(intent.clone()));
    }

    if let Some(fonts) = non_empty(config.fonts.as_ref().and_then(|fonts| fonts.nerd.as_ref())) {
        prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::NerdFonts]);
        remaining.push(PlannedAction::NerdFonts(fonts.clone()));
    }

    if let Some(dotfiles) = &config.dotfiles {
        prerequisites.insert(Prerequisite::Stow);
        remaining.push(PlannedAction::Dotfiles(DotfilesIntent {
            root: dotfiles_root.to_path_buf(),
            packages: dotfiles.packages.clone(),
            conflict_policy: DotfilesConflictPolicy::BackupBeforeStow,
        }));
    }

    plan_integrations(config, &mut remaining);
    plan_desktop(config, platform, &mut remaining, &mut prerequisites);
    plan_updates(
        config,
        platform,
        &direct_intents,
        &mut updates,
        &mut prerequisites,
        &mut needs_apt_metadata,
    );

    let mut actions = Vec::new();
    if !prerequisites.is_empty() {
        actions.push(PlannedAction::Bootstrap(Bootstrap { prerequisites }));
    }
    actions.append(&mut preparation);
    actions.append(&mut sources);
    if needs_apt_metadata {
        actions.push(PlannedAction::AptMetadataRefresh);
    }
    actions.append(&mut apt_consumers);
    actions.append(&mut remaining);
    actions.append(&mut updates);
    Ok(PlanV1 { actions })
}

fn plan_system(
    config: &ConfigV1,
    platform: &Platform,
    preparation: &mut Vec<PlannedAction>,
    sources: &mut Vec<PlannedAction>,
    apt_consumers: &mut Vec<PlannedAction>,
    needs_apt_metadata: &mut bool,
) -> Result<()> {
    let Some(system) = &config.system else {
        return Ok(());
    };
    if system.ensure_admin == Some(true) {
        preparation.push(PlannedAction::System(SystemAction::EnsureAdmin));
    }
    if let Some(apt) = &system.apt {
        if matches!(apt.sources, Some(AptSources::Managed)) {
            let codename = AptSourceToken::parse(&platform.codename)
                .context("system.apt.sources: managed requires a valid platform codename")?;
            sources.push(PlannedAction::System(SystemAction::AptSources(
                AptSourcesIntent::Managed {
                    distro: platform.distro.clone(),
                    upstream: platform.upstream.clone(),
                    codename,
                    components: apt.components.clone(),
                },
            )));
        }
        if let Some(enabled) = apt.unattended_upgrades {
            *needs_apt_metadata = true;
            apt_consumers.push(PlannedAction::System(SystemAction::UnattendedUpgrades {
                enabled,
            }));
        }
    }
    if platform.upstream == "ubuntu" {
        if let Some(ubuntu) = &system.ubuntu {
            if let Some(enabled) = ubuntu.snap {
                *needs_apt_metadata = true;
                apt_consumers.push(PlannedAction::System(SystemAction::UbuntuSnap { enabled }));
            }
            if ubuntu.codecs == Some(true) {
                *needs_apt_metadata = true;
                apt_consumers.push(PlannedAction::System(SystemAction::UbuntuCodecs));
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
        prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::Rustup]);
        actions.push(PlannedAction::Tool(ToolInstall::Rust {
            selector: rust_selector(selector),
            architecture: platform.architecture,
        }));
    }
    if let Some(selector) = tools.go.as_deref() {
        prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::GoArchives]);
        actions.push(PlannedAction::Tool(ToolInstall::Go {
            selector: go_selector(selector),
            architecture: platform.architecture,
        }));
    }
    if let Some(selector) = tools.node.as_deref() {
        prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::FnmNpm]);
        actions.push(PlannedAction::Tool(ToolInstall::Node {
            selector: node_selector(selector),
        }));
    }
    if let Some(version) = tools.python.as_ref() {
        prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::Uv]);
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

fn plan_integrations(config: &ConfigV1, actions: &mut Vec<PlannedAction>) {
    let Some(integrations) = &config.integrations else {
        return;
    };
    if let Some(docker) = &integrations.docker {
        if docker.add_user_to_group == Some(true) {
            actions.push(PlannedAction::Integration(IntegrationAction::DockerGroup));
        }
        if docker.local_log_driver == Some(true) {
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
    let target = match platform.desktop.as_str() {
        "gnome" => DesktopTarget::Gnome,
        "cinnamon" => DesktopTarget::Cinnamon,
        _ => return,
    };
    let start = actions.len();
    if let Some(theme) = &desktop.theme {
        actions.push(PlannedAction::Desktop(DesktopAction::Theme {
            target,
            theme: theme.clone(),
        }));
    }
    if let Some(terminal) = &desktop.terminal {
        actions.push(PlannedAction::Desktop(DesktopAction::Terminal {
            target,
            executable: terminal.clone(),
        }));
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = &idle.timeout {
            actions.push(PlannedAction::Desktop(DesktopAction::IdleTimeout {
                target,
                timeout: timeout.clone(),
            }));
        }
        if let Some(enabled) = idle.dim {
            actions.push(PlannedAction::Desktop(DesktopAction::IdleDim {
                target,
                enabled,
            }));
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
            actions.push(PlannedAction::Update(UpdateAction::Apt {
                policy: AptUpdatePolicy::Standard,
                target: AptUpdateTarget::SystemPackages,
            }));
        }
        Some(AptUpdate::Full) => {
            *apt_metadata = true;
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
            prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::FlatpakFlathub]);
            actions.push(PlannedAction::Update(UpdateAction::Flatpak {
                refs: refs.clone(),
                scope: FlatpakUpdateScope::ConfiguredRefsAndRequiredRuntimes,
            }));
        }
    }
    if let Some(tool_updates) = &updates.tools {
        if tool_updates.rust == Some(true) {
            if let Some(selector) = tools.and_then(|tools| tools.rust.as_deref()) {
                prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::Rustup]);
                actions.push(PlannedAction::Update(UpdateAction::Tool(
                    ToolUpdate::Rust {
                        selector: rust_selector(selector),
                        architecture: platform.architecture,
                    },
                )));
            }
        }
        if tool_updates.go == Some(true) {
            if let Some(selector) = tools.and_then(|tools| tools.go.as_deref()) {
                prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::GoArchives]);
                actions.push(PlannedAction::Update(UpdateAction::Tool(ToolUpdate::Go {
                    selector: go_selector(selector),
                    architecture: platform.architecture,
                })));
            }
        }
        if tool_updates.node == Some(true) {
            if let Some(selector) = tools.and_then(|tools| tools.node.as_deref()) {
                prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::FnmNpm]);
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
                    Prerequisite::HttpsDownloader,
                    Prerequisite::Rustup,
                    Prerequisite::CargoBinstall,
                ]);
                actions.push(PlannedAction::Update(UpdateAction::Cargo {
                    packages: packages.clone(),
                }));
            }
        }
        if package_updates.npm == Some(true) {
            if let Some(packages) = non_empty(packages.and_then(|packages| packages.npm.as_ref())) {
                prerequisites.extend([Prerequisite::HttpsDownloader, Prerequisite::FnmNpm]);
                actions.push(PlannedAction::Update(UpdateAction::Npm {
                    packages: packages.clone(),
                }));
            }
        }
        if package_updates.direct == Some(true) && !direct.is_empty() {
            prerequisites.insert(Prerequisite::HttpsDownloader);
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
