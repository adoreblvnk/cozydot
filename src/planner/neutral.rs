use crate::{
    config::{
        resolve_platform_identity, AptToken, AptUpdate, BinaryFormat, Config, EnabledDisabled,
        HttpsUrl, InstalledState, ResolvedNativeBinary, SourceMode, Theme,
    },
    platform::{Architecture, ManagedAptSources, Platform},
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    phases: Vec<PlanPhase>,
}

impl Plan {
    pub fn phases(&self) -> &[PlanPhase] {
        &self.phases
    }

    pub fn actions(&self) -> impl Iterator<Item = &PlannedAction> {
        self.phases.iter().flat_map(|phase| phase.actions.iter())
    }

    pub fn is_empty(&self) -> bool {
        self.actions().next().is_none()
    }

    pub fn phase(&self, kind: PlanPhaseKind) -> &PlanPhase {
        self.phases
            .iter()
            .find(|phase| phase.kind == kind)
            .expect("every plan contains all fixed phases")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanPhase {
    kind: PlanPhaseKind,
    actions: Vec<PlannedAction>,
}

impl PlanPhase {
    pub fn kind(&self) -> PlanPhaseKind {
        self.kind
    }

    pub fn actions(&self) -> &[PlannedAction] {
        &self.actions
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanPhaseKind {
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

impl PlanPhaseKind {
    pub const ORDERED: [Self; 20] = [
        Self::SystemPrerequisites,
        Self::ManagerBootstraps,
        Self::AdministrativeVerification,
        Self::OfficialAptSources,
        Self::ThirdPartyRepositories,
        Self::AptMetadataRefresh,
        Self::SystemPackageStates,
        Self::AptPurge,
        Self::RepositoryPackages,
        Self::AptPackages,
        Self::FlatpakApplications,
        Self::LanguageToolchains,
        Self::LanguagePackages,
        Self::BinaryPackages,
        Self::Fonts,
        Self::Dotfiles,
        Self::Integrations,
        Self::Desktop,
        Self::Updates,
        Self::FinalVerification,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedAction {
    SystemPrerequisites(BTreeSet<SystemPrerequisite>),
    ManagerBootstraps(BTreeSet<ManagerBootstrap>),
    EnsureAdmin,
    ManagedOfficialAptSources(ManagedAptSources),
    Repository(AptRepositoryIntent),
    AptMetadataRefresh,
    UnattendedUpgrades(EnabledDisabled),
    UbuntuSnap(EnabledDisabled),
    UbuntuCodecs(InstalledState),
    Skip(PlatformSkip),
    AptPurge(Vec<String>),
    RepositoryPackages(RepositoryPackageGroup),
    AptPackages(Vec<String>),
    FlatpakApplications(FlatpakIntent),
    Toolchain(ToolchainIntent),
    CargoPackages(Vec<String>),
    NpmPackages(Vec<String>),
    Binary(BinaryPackageIntent),
    NerdFonts(Vec<String>),
    Dotfiles(DotfilesIntent),
    Integration(IntegrationIntent),
    Desktop(DesktopIntent),
    Update(UpdateIntent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SystemPrerequisite {
    HttpsCertificates,
    OpenPgp,
    ArchiveExtraction,
    DebInspection,
    ElfInspection,
    FontCache,
    DesktopSettings,
    Stow,
    GnomeExtensionManagement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ManagerBootstrap {
    Flatpak,
    Rustup,
    CargoBinstall,
    Fnm,
    Uv,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AptRepositoryIntent {
    pub name: String,
    pub filename_stem: String,
    pub key_url: HttpsUrl,
    pub source_url: HttpsUrl,
    pub architecture: Architecture,
    pub keyring_path: PathBuf,
    pub source_list_path: PathBuf,
    pub layout: RepositoryLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryLayout {
    SuiteComponents {
        suite: RepositorySuite,
        components: Vec<AptToken>,
    },
    ExactPath(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositorySuite {
    ResolvedSystem(AptToken),
    Fixed(AptToken),
}

impl RepositorySuite {
    pub fn value(&self) -> &str {
        match self {
            Self::ResolvedSystem(value) | Self::Fixed(value) => value.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryPackageGroup {
    pub repository: String,
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatpakIntent {
    pub remote: FlatpakRemote,
    pub applications: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatpakRemote {
    FlathubPerUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainIntent {
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
        architecture: Architecture,
    },
    Python {
        selector: PinnedSelector,
        architecture: Architecture,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustSelector {
    Stable,
    Beta,
    Nightly,
    Pinned(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoSelector {
    Latest,
    Pinned(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeSelector {
    Lts,
    Latest,
    Pinned(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedSelector(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryPackageIntent {
    pub name: String,
    pub format: BinaryFormat,
    pub commands: Vec<String>,
    pub architecture: Architecture,
    pub source: BinarySourceIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinarySourceIntent {
    Github {
        repository: String,
        release: GithubReleasePolicy,
        selector: GithubAssetSelector,
    },
    FixedUrl {
        url: HttpsUrl,
        sha256: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubReleasePolicy {
    LatestNonDraftNonPrerelease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAssetSelector {
    pub include: String,
    pub exclude: Vec<String>,
    pub declared_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotfilesIntent {
    pub root: PathBuf,
    pub packages: Vec<String>,
    pub policy: DotfilesPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DotfilesPolicy {
    BackupBeforeStow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationIntent {
    pub required_product: ExistingProductRequirement,
    pub action: IntegrationAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingProductRequirement {
    Docker,
    VirtualBox,
    VsCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationAction {
    AddInvokingUserToGroup,
    DockerLocalLogging { max_size: Option<String> },
    VsCodeExtensions(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopIntent {
    Theme {
        target: DesktopTarget,
        theme: Theme,
    },
    Terminal {
        target: DesktopTarget,
        executable: ExecutablePrecondition,
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
    GnomeDock(ProviderConvergence),
    GnomeRoundedCorners(ProviderConvergence),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopTarget {
    Gnome,
    Cinnamon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutablePrecondition {
    pub exact_basename: String,
    pub timing: PreconditionTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreconditionTiming {
    AfterInstallPhases,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderConvergence {
    EnsureFixedProviderThenConfigureAndVerify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSkip {
    pub intent: SkippedIntent,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkippedIntent {
    UbuntuSnap,
    UbuntuCodecs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    RequiresUbuntuFamily,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateIntent {
    Apt(AptUpdatePolicy),
    Flatpak {
        applications: Vec<String>,
        scope: FlatpakUpdateScope,
    },
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
        architecture: Architecture,
    },
    Cargo(Vec<String>),
    Npm(Vec<String>),
    GithubBinaries(Vec<BinaryPackageIntent>),
    NerdFonts(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AptUpdatePolicy {
    Standard,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatpakUpdateScope {
    ConfiguredApplicationsWithRequiredRefsAndEolReplacements,
}

pub fn plan(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<Plan> {
    config.validate_for_platform(platform)?;
    let identity = resolve_platform_identity(platform)?;
    let mut phases = PlanPhaseKind::ORDERED
        .into_iter()
        .map(|kind| PlanPhase {
            kind,
            actions: Vec::new(),
        })
        .collect::<Vec<_>>();
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
        push(
            &mut phases,
            PlanPhaseKind::AdministrativeVerification,
            PlannedAction::EnsureAdmin,
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
            push(
                &mut phases,
                PlanPhaseKind::OfficialAptSources,
                PlannedAction::ManagedOfficialAptSources(managed),
            );
        }
    }

    if let Some(repositories) = apt.and_then(|apt| apt.repositories.as_ref()) {
        prerequisites.extend([
            SystemPrerequisite::HttpsCertificates,
            SystemPrerequisite::OpenPgp,
        ]);
        for (index, repository) in repositories.iter().enumerate() {
            let resolved = repository.resolve_for_platform(index, platform, identity)?;
            let layout = if let Some(path) = &repository.path {
                RepositoryLayout::ExactPath(path.clone())
            } else {
                let suite = if repository.suite.as_deref() == Some("system") {
                    RepositorySuite::ResolvedSystem(
                        resolved
                            .suite
                            .expect("validated suite/components repository"),
                    )
                } else {
                    RepositorySuite::Fixed(
                        resolved
                            .suite
                            .expect("validated suite/components repository"),
                    )
                };
                RepositoryLayout::SuiteComponents {
                    suite,
                    components: repository
                        .components
                        .clone()
                        .expect("validated suite/components repository"),
                }
            };
            let stem = repository.filename_stem();
            push(
                &mut phases,
                PlanPhaseKind::ThirdPartyRepositories,
                PlannedAction::Repository(AptRepositoryIntent {
                    name: repository.name.clone(),
                    filename_stem: stem.clone(),
                    key_url: repository.key.clone(),
                    source_url: resolved.source_url.clone(),
                    architecture: platform.architecture,
                    keyring_path: PathBuf::from(format!("/etc/apt/keyrings/cozydot-{stem}.gpg")),
                    source_list_path: PathBuf::from(format!(
                        "/etc/apt/sources.list.d/cozydot-{stem}.list"
                    )),
                    layout,
                }),
            );
            push(
                &mut phases,
                PlanPhaseKind::RepositoryPackages,
                PlannedAction::RepositoryPackages(RepositoryPackageGroup {
                    repository: repository.name.clone(),
                    packages: repository.packages.clone(),
                }),
            );
            needs_apt_refresh = true;
        }
    }

    plan_system_states(config, platform, &mut phases, &mut needs_apt_refresh);

    if let Some(remove) = apt.and_then(|apt| apt.remove.as_ref()) {
        push(
            &mut phases,
            PlanPhaseKind::AptPurge,
            PlannedAction::AptPurge(remove.clone()),
        );
        needs_apt_refresh = true;
    }
    if let Some(install) = apt.and_then(|apt| apt.install.as_ref()) {
        push(
            &mut phases,
            PlanPhaseKind::AptPackages,
            PlannedAction::AptPackages(install.clone()),
        );
        needs_apt_refresh = true;
    }

    if let Some(applications) = packages.and_then(|packages| packages.flatpak.as_ref()) {
        prerequisites.insert(SystemPrerequisite::HttpsCertificates);
        managers.insert(ManagerBootstrap::Flatpak);
        push(
            &mut phases,
            PlanPhaseKind::FlatpakApplications,
            PlannedAction::FlatpakApplications(FlatpakIntent {
                remote: FlatpakRemote::FlathubPerUser,
                applications: applications.clone(),
            }),
        );
    }

    plan_tools(
        config,
        platform,
        &mut phases,
        &mut prerequisites,
        &mut managers,
    );

    if let Some(cargo) = packages.and_then(|packages| packages.cargo.as_ref()) {
        prerequisites.insert(SystemPrerequisite::HttpsCertificates);
        managers.extend([ManagerBootstrap::Rustup, ManagerBootstrap::CargoBinstall]);
        push(
            &mut phases,
            PlanPhaseKind::LanguagePackages,
            PlannedAction::CargoPackages(cargo.clone()),
        );
    }
    if let Some(npm) = packages.and_then(|packages| packages.npm.as_ref()) {
        prerequisites.insert(SystemPrerequisite::HttpsCertificates);
        managers.insert(ManagerBootstrap::Fnm);
        push(
            &mut phases,
            PlanPhaseKind::LanguagePackages,
            PlannedAction::NpmPackages(npm.clone()),
        );
    }

    let mut binary_intents = Vec::new();
    if let Some(binaries) = packages.and_then(|packages| packages.binaries.as_ref()) {
        prerequisites.insert(SystemPrerequisite::HttpsCertificates);
        for (index, binary) in binaries.iter().enumerate() {
            let intent = binary_intent(binary, platform.architecture)
                .with_context(|| format!("packages.binaries[{index}].source"))?;
            match binary.format {
                BinaryFormat::Deb => {
                    prerequisites.insert(SystemPrerequisite::DebInspection);
                    needs_apt_refresh = true;
                }
                BinaryFormat::Appimage => {
                    prerequisites.insert(SystemPrerequisite::ElfInspection);
                }
            }
            push(
                &mut phases,
                PlanPhaseKind::BinaryPackages,
                PlannedAction::Binary(intent.clone()),
            );
            binary_intents.push(intent);
        }
    }

    if let Some(fonts) = config.fonts.as_ref().and_then(|fonts| fonts.nerd.as_ref()) {
        prerequisites.extend([
            SystemPrerequisite::HttpsCertificates,
            SystemPrerequisite::ArchiveExtraction,
            SystemPrerequisite::FontCache,
        ]);
        push(
            &mut phases,
            PlanPhaseKind::Fonts,
            PlannedAction::NerdFonts(fonts.clone()),
        );
    }

    if let Some(dotfiles) = &config.dotfiles {
        prerequisites.insert(SystemPrerequisite::Stow);
        push(
            &mut phases,
            PlanPhaseKind::Dotfiles,
            PlannedAction::Dotfiles(DotfilesIntent {
                root: dotfiles_root.to_path_buf(),
                packages: dotfiles.packages.clone(),
                policy: DotfilesPolicy::BackupBeforeStow,
            }),
        );
    }

    plan_integrations(config, &mut phases);
    plan_desktop(config, platform, &mut phases, &mut prerequisites);
    plan_updates(
        config,
        platform,
        &binary_intents,
        &mut phases,
        &mut needs_apt_refresh,
    );

    if needs_apt_refresh {
        push(
            &mut phases,
            PlanPhaseKind::AptMetadataRefresh,
            PlannedAction::AptMetadataRefresh,
        );
    }
    if !prerequisites.is_empty() {
        push(
            &mut phases,
            PlanPhaseKind::SystemPrerequisites,
            PlannedAction::SystemPrerequisites(prerequisites),
        );
    }
    if !managers.is_empty() {
        push(
            &mut phases,
            PlanPhaseKind::ManagerBootstraps,
            PlannedAction::ManagerBootstraps(managers),
        );
    }

    Ok(Plan { phases })
}

fn plan_system_states(
    config: &Config,
    platform: &Platform,
    phases: &mut [PlanPhase],
    needs_apt_refresh: &mut bool,
) {
    let Some(system) = &config.system else { return };
    if let Some(state) = system.apt.as_ref().and_then(|apt| apt.unattended_upgrades) {
        push(
            phases,
            PlanPhaseKind::SystemPackageStates,
            PlannedAction::UnattendedUpgrades(state),
        );
        *needs_apt_refresh = true;
    }
    let Some(ubuntu) = &system.ubuntu else { return };
    let ubuntu_family = platform.upstream == "ubuntu";
    if let Some(state) = ubuntu.snap {
        let action = if ubuntu_family {
            *needs_apt_refresh = true;
            PlannedAction::UbuntuSnap(state)
        } else {
            PlannedAction::Skip(PlatformSkip {
                intent: SkippedIntent::UbuntuSnap,
                reason: SkipReason::RequiresUbuntuFamily,
            })
        };
        push(phases, PlanPhaseKind::SystemPackageStates, action);
    }
    if let Some(state) = ubuntu.codecs {
        let action = if ubuntu_family {
            *needs_apt_refresh = true;
            PlannedAction::UbuntuCodecs(state)
        } else {
            PlannedAction::Skip(PlatformSkip {
                intent: SkippedIntent::UbuntuCodecs,
                reason: SkipReason::RequiresUbuntuFamily,
            })
        };
        push(phases, PlanPhaseKind::SystemPackageStates, action);
    }
}

fn plan_tools(
    config: &Config,
    platform: &Platform,
    phases: &mut [PlanPhase],
    prerequisites: &mut BTreeSet<SystemPrerequisite>,
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    let Some(tools) = &config.tools else { return };
    if let Some(selector) = tools.rust.as_deref() {
        prerequisites.insert(SystemPrerequisite::HttpsCertificates);
        managers.insert(ManagerBootstrap::Rustup);
        push(
            phases,
            PlanPhaseKind::LanguageToolchains,
            PlannedAction::Toolchain(ToolchainIntent::Rust {
                selector: rust_selector(selector),
                architecture: platform.architecture,
            }),
        );
    }
    if let Some(selector) = tools.go.as_deref() {
        prerequisites.extend([
            SystemPrerequisite::HttpsCertificates,
            SystemPrerequisite::ArchiveExtraction,
        ]);
        push(
            phases,
            PlanPhaseKind::LanguageToolchains,
            PlannedAction::Toolchain(ToolchainIntent::Go {
                selector: go_selector(selector),
                architecture: platform.architecture,
            }),
        );
    }
    if let Some(selector) = tools.node.as_deref() {
        prerequisites.insert(SystemPrerequisite::HttpsCertificates);
        managers.insert(ManagerBootstrap::Fnm);
        push(
            phases,
            PlanPhaseKind::LanguageToolchains,
            PlannedAction::Toolchain(ToolchainIntent::Node {
                selector: node_selector(selector),
                architecture: platform.architecture,
            }),
        );
    }
    if let Some(selector) = &tools.python {
        prerequisites.insert(SystemPrerequisite::HttpsCertificates);
        managers.insert(ManagerBootstrap::Uv);
        push(
            phases,
            PlanPhaseKind::LanguageToolchains,
            PlannedAction::Toolchain(ToolchainIntent::Python {
                selector: PinnedSelector(selector.clone()),
                architecture: platform.architecture,
            }),
        );
    }
}

fn binary_intent(
    binary: &crate::config::BinaryPackage,
    architecture: Architecture,
) -> Result<BinaryPackageIntent> {
    let source = match binary.source.resolve_native(architecture)? {
        ResolvedNativeBinary::Github {
            repository,
            selector,
        } => BinarySourceIntent::Github {
            repository: repository.to_owned(),
            release: GithubReleasePolicy::LatestNonDraftNonPrerelease,
            selector: GithubAssetSelector {
                include: selector.include.clone(),
                exclude: selector.exclude.clone().unwrap_or_default(),
                declared_sha256: selector
                    .sha256
                    .as_ref()
                    .map(|hash| hash.as_str().to_owned()),
            },
        },
        ResolvedNativeBinary::Url { url, sha256 } => BinarySourceIntent::FixedUrl {
            url: url.clone(),
            sha256: sha256.as_str().to_owned(),
        },
    };
    Ok(BinaryPackageIntent {
        name: binary.name.clone(),
        format: binary.format,
        commands: binary.commands.clone(),
        architecture,
        source,
    })
}

fn plan_integrations(config: &Config, phases: &mut [PlanPhase]) {
    let Some(integrations) = &config.integrations else {
        return;
    };
    if let Some(docker) = &integrations.docker {
        if docker.add_user_to_group == Some(true) {
            integration(
                phases,
                ExistingProductRequirement::Docker,
                IntegrationAction::AddInvokingUserToGroup,
            );
        }
        if let Some(logging) = &docker.logging {
            integration(
                phases,
                ExistingProductRequirement::Docker,
                IntegrationAction::DockerLocalLogging {
                    max_size: logging.max_size.clone(),
                },
            );
        }
    }
    if integrations
        .virtualbox
        .as_ref()
        .is_some_and(|virtualbox| virtualbox.add_user_to_group == Some(true))
    {
        integration(
            phases,
            ExistingProductRequirement::VirtualBox,
            IntegrationAction::AddInvokingUserToGroup,
        );
    }
    if let Some(extensions) = integrations
        .vscode
        .as_ref()
        .map(|vscode| vscode.extensions.clone())
    {
        integration(
            phases,
            ExistingProductRequirement::VsCode,
            IntegrationAction::VsCodeExtensions(extensions),
        );
    }
}

fn integration(
    phases: &mut [PlanPhase],
    required_product: ExistingProductRequirement,
    action: IntegrationAction,
) {
    push(
        phases,
        PlanPhaseKind::Integrations,
        PlannedAction::Integration(IntegrationIntent {
            required_product,
            action,
        }),
    );
}

fn plan_desktop(
    config: &Config,
    platform: &Platform,
    phases: &mut [PlanPhase],
    prerequisites: &mut BTreeSet<SystemPrerequisite>,
) {
    let Some(desktop) = &config.desktop else {
        return;
    };
    let target = match platform.desktop.as_str() {
        "gnome" => DesktopTarget::Gnome,
        "cinnamon" => DesktopTarget::Cinnamon,
        _ => unreachable!("platform validation rejects unsupported desktop intent"),
    };
    prerequisites.insert(SystemPrerequisite::DesktopSettings);
    if let Some(theme) = desktop.theme {
        desktop_action(phases, DesktopIntent::Theme { target, theme });
    }
    if let Some(executable) = &desktop.terminal {
        desktop_action(
            phases,
            DesktopIntent::Terminal {
                target,
                executable: ExecutablePrecondition {
                    exact_basename: executable.clone(),
                    timing: PreconditionTiming::AfterInstallPhases,
                },
            },
        );
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = &idle.timeout {
            desktop_action(
                phases,
                DesktopIntent::IdleTimeout {
                    target,
                    timeout: timeout.clone(),
                },
            );
        }
        if let Some(enabled) = idle.dim {
            desktop_action(phases, DesktopIntent::IdleDim { target, enabled });
        }
    }
    if let Some(gnome) = &desktop.gnome {
        if let Some(extensions) = &gnome.extensions {
            prerequisites.insert(SystemPrerequisite::GnomeExtensionManagement);
            desktop_action(phases, DesktopIntent::GnomeExtensions(extensions.clone()));
        }
        if gnome.dock == Some(true) {
            prerequisites.insert(SystemPrerequisite::GnomeExtensionManagement);
            desktop_action(
                phases,
                DesktopIntent::GnomeDock(
                    ProviderConvergence::EnsureFixedProviderThenConfigureAndVerify,
                ),
            );
        }
        if gnome.rounded_corners == Some(true) {
            prerequisites.insert(SystemPrerequisite::GnomeExtensionManagement);
            desktop_action(
                phases,
                DesktopIntent::GnomeRoundedCorners(
                    ProviderConvergence::EnsureFixedProviderThenConfigureAndVerify,
                ),
            );
        }
    }
}

fn desktop_action(phases: &mut [PlanPhase], action: DesktopIntent) {
    push(
        phases,
        PlanPhaseKind::Desktop,
        PlannedAction::Desktop(action),
    );
}

fn plan_updates(
    config: &Config,
    platform: &Platform,
    binaries: &[BinaryPackageIntent],
    phases: &mut [PlanPhase],
    needs_apt_refresh: &mut bool,
) {
    let Some(updates) = &config.updates else {
        return;
    };
    let packages = config.packages.as_ref();
    let tools = config.tools.as_ref();
    if let Some(policy) = updates.apt {
        *needs_apt_refresh = true;
        update(
            phases,
            UpdateIntent::Apt(match policy {
                AptUpdate::Standard => AptUpdatePolicy::Standard,
                AptUpdate::Full => AptUpdatePolicy::Full,
            }),
        );
    }
    if updates.flatpak == Some(true) {
        update(
            phases,
            UpdateIntent::Flatpak {
                applications: packages
                    .and_then(|packages| packages.flatpak.clone())
                    .expect("validated update target"),
                scope: FlatpakUpdateScope::ConfiguredApplicationsWithRequiredRefsAndEolReplacements,
            },
        );
    }
    if let Some(tool_updates) = &updates.tools {
        if tool_updates.rust == Some(true) {
            update(
                phases,
                UpdateIntent::Rust {
                    selector: rust_selector(
                        tools
                            .and_then(|tools| tools.rust.as_deref())
                            .expect("validated update target"),
                    ),
                    architecture: platform.architecture,
                },
            );
        }
        if tool_updates.go == Some(true) {
            update(
                phases,
                UpdateIntent::Go {
                    selector: go_selector(
                        tools
                            .and_then(|tools| tools.go.as_deref())
                            .expect("validated update target"),
                    ),
                    architecture: platform.architecture,
                },
            );
        }
        if tool_updates.node == Some(true) {
            update(
                phases,
                UpdateIntent::Node {
                    selector: node_selector(
                        tools
                            .and_then(|tools| tools.node.as_deref())
                            .expect("validated update target"),
                    ),
                    architecture: platform.architecture,
                },
            );
        }
    }
    if let Some(package_updates) = &updates.packages {
        if package_updates.cargo == Some(true) {
            update(
                phases,
                UpdateIntent::Cargo(
                    packages
                        .and_then(|packages| packages.cargo.clone())
                        .expect("validated update target"),
                ),
            );
        }
        if package_updates.npm == Some(true) {
            update(
                phases,
                UpdateIntent::Npm(
                    packages
                        .and_then(|packages| packages.npm.clone())
                        .expect("validated update target"),
                ),
            );
        }
        if package_updates.binaries == Some(true) {
            let github = binaries
                .iter()
                .filter(|binary| matches!(binary.source, BinarySourceIntent::Github { .. }))
                .cloned()
                .collect();
            update(phases, UpdateIntent::GithubBinaries(github));
        }
    }
    if updates.fonts == Some(true) {
        update(
            phases,
            UpdateIntent::NerdFonts(
                config
                    .fonts
                    .as_ref()
                    .and_then(|fonts| fonts.nerd.clone())
                    .expect("validated update target"),
            ),
        );
    }
}

fn update(phases: &mut [PlanPhase], intent: UpdateIntent) {
    push(
        phases,
        PlanPhaseKind::Updates,
        PlannedAction::Update(intent),
    );
}

fn rust_selector(value: &str) -> RustSelector {
    match value {
        "stable" => RustSelector::Stable,
        "beta" => RustSelector::Beta,
        "nightly" => RustSelector::Nightly,
        value => RustSelector::Pinned(value.to_owned()),
    }
}

fn go_selector(value: &str) -> GoSelector {
    if value == "latest" {
        GoSelector::Latest
    } else {
        GoSelector::Pinned(value.to_owned())
    }
}

fn node_selector(value: &str) -> NodeSelector {
    match value {
        "lts" => NodeSelector::Lts,
        "latest" => NodeSelector::Latest,
        value => NodeSelector::Pinned(value.to_owned()),
    }
}

fn push(phases: &mut [PlanPhase], kind: PlanPhaseKind, action: PlannedAction) {
    phases
        .iter_mut()
        .find(|phase| phase.kind == kind)
        .expect("fixed phase exists")
        .actions
        .push(action);
}
