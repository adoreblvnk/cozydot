use super::{
    AptRepositoryIntent, AptUpdatePolicy, BinaryPackageIntent, BinarySourceIntent, DesktopIntent,
    DesktopTarget, DotfilesPolicy, ExistingProductRequirement, FlatpakRemote, FlatpakUpdateScope,
    GithubReleasePolicy, GoSelector, IntegrationAction, IntegrationIntent, ManagerBootstrap,
    NodeSelector, Plan, PlanPhaseKind, PlannedAction, PreconditionTiming, ProviderConvergence,
    RepositoryLayout, RustSelector, SkipReason as PlannedSkipReason, SkippedIntent,
    SystemPrerequisite, ToolchainIntent, UpdateIntent,
};
use crate::{
    config::{BinaryFormat, EnabledDisabled, InstalledState, Theme},
    operations::{
        AptRepositoryOperation, AptRepositoryPath, AptRepositorySourceLayout, AptRepositoryToken,
        AptUpgradePolicy, BinaryPackageFormat, BinaryPackageMode, BinaryPackageOperation,
        BinaryPackageSelector, BinarySha256, BinarySourceOperation,
        CargoBinstallBootstrapOperation, CargoPackageMode, CargoPackageOperation,
        DesktopEnvironment, DesktopSetting, DesktopSettingOperation, DesktopTheme,
        DockerLocalLogOperation, DotfilesOperation, EnsureAdminOperation, GithubRepository,
        GnomeDockOperation, GnomeExtensionsOperation, GnomeRoundedCornersOperation,
        GoToolchainOperation, GoToolchainSelector, ManagedAptSourcesOperation, NerdFontsMode,
        NerdFontsOperation, NodeToolchainOperation, NodeToolchainSelector, NpmPackageMode,
        NpmPackageOperation, Operation, PythonToolchainOperation, RustToolchainOperation,
        RustToolchainSelector, ToolMutationMode, UbuntuSnapOperation, UnattendedUpgradesOperation,
        VsCodeExtensionOperation,
    },
    platform::Architecture,
    runner::{ExecutionPhase, SkipReason, SkippedAction, Step},
};
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;

pub fn lower(plan: &Plan) -> Result<Vec<Step>> {
    let managers = manager_bootstraps(plan)?;
    let cargo_binstall_architecture = plan.actions().find_map(|action| match action {
        PlannedAction::Toolchain(ToolchainIntent::Rust { architecture, .. }) => Some(*architecture),
        _ => None,
    });
    let mut steps = Vec::new();
    for phase in plan.phases() {
        steps.push(Step::phase(execution_phase(phase.kind())));
        for action in phase.actions() {
            lower_action(action, &managers, cargo_binstall_architecture, &mut steps)?;
        }
    }
    steps.push(Step::summary());
    Ok(steps)
}

fn lower_action(
    action: &PlannedAction,
    managers: &BTreeSet<ManagerBootstrap>,
    cargo_binstall_architecture: Option<Architecture>,
    steps: &mut Vec<Step>,
) -> Result<()> {
    match action {
        PlannedAction::SystemPrerequisites(prerequisites) => {
            lower_prerequisites(prerequisites, managers, steps)
        }
        PlannedAction::ManagerBootstraps(managers) => {
            lower_managers(managers, cargo_binstall_architecture, steps)?
        }
        PlannedAction::EnsureAdmin => {
            push(steps, Operation::EnsureAdmin(EnsureAdminOperation::new()))
        }
        PlannedAction::ManagedOfficialAptSources(policy) => push(
            steps,
            Operation::ManagedAptSources(ManagedAptSourcesOperation::from_policy(policy.clone())?),
        ),
        PlannedAction::Repository(repository) => push(
            steps,
            Operation::AptRepository(lower_repository(repository)?),
        ),
        PlannedAction::AptMetadataRefresh => push(steps, Operation::AptMetadataRefresh),
        PlannedAction::UnattendedUpgrades(state) => push(
            steps,
            Operation::UnattendedUpgrades(UnattendedUpgradesOperation::new(enabled(*state))),
        ),
        PlannedAction::UbuntuSnap(state) => push(
            steps,
            Operation::UbuntuSnap(UbuntuSnapOperation::new(enabled(*state))),
        ),
        PlannedAction::UbuntuCodecs(InstalledState::Installed) => push(
            steps,
            Operation::AptPackages {
                packages: vec!["ubuntu-restricted-extras".into()],
            },
        ),
        PlannedAction::Skip(skip) => steps.push(Step::skip(
            match skip.intent {
                SkippedIntent::UbuntuSnap => SkippedAction::UbuntuSnap,
                SkippedIntent::UbuntuCodecs => SkippedAction::UbuntuCodecs,
            },
            match skip.reason {
                PlannedSkipReason::RequiresUbuntuFamily => SkipReason::RequiresUbuntuFamily,
            },
        )),
        PlannedAction::AptPurge(packages) => push(
            steps,
            Operation::AptPurge {
                packages: packages.clone(),
            },
        ),
        PlannedAction::RepositoryPackages(group) => steps.push(Step::labeled_workflow(
            Operation::AptPackages {
                packages: group.packages.clone(),
            },
            format!("repository {}", group.repository),
        )?),
        PlannedAction::AptPackages(packages) => push(
            steps,
            Operation::AptPackages {
                packages: packages.clone(),
            },
        ),
        PlannedAction::FlatpakApplications(intent) => {
            let FlatpakRemote::FlathubPerUser = intent.remote;
            push(
                steps,
                Operation::FlatpakEnsureApps {
                    refs: intent.applications.clone(),
                },
            );
        }
        PlannedAction::Toolchain(toolchain) => push(steps, lower_toolchain(toolchain)?),
        PlannedAction::CargoPackages(packages) => push(
            steps,
            Operation::CargoPackageSet(CargoPackageOperation::new(
                packages.clone(),
                CargoPackageMode::EnsurePresent,
            )?),
        ),
        PlannedAction::NpmPackages(packages) => push(
            steps,
            Operation::NpmPackageSet(NpmPackageOperation::new(
                packages.clone(),
                NpmPackageMode::EnsurePresent,
            )?),
        ),
        PlannedAction::Binary(binary) => push(
            steps,
            Operation::BinaryPackage(lower_binary(binary, BinaryPackageMode::EnsurePresent)?),
        ),
        PlannedAction::NerdFonts(families) => push(
            steps,
            Operation::NerdFonts(NerdFontsOperation::new(
                families.clone(),
                NerdFontsMode::EnsurePresent,
            )?),
        ),
        PlannedAction::Dotfiles(intent) => {
            let DotfilesPolicy::BackupBeforeStow = intent.policy;
            push(
                steps,
                Operation::Dotfiles(DotfilesOperation::new(
                    intent.root.clone(),
                    intent.packages.clone(),
                )?),
            );
        }
        PlannedAction::Integration(integration) => push(steps, lower_integration(integration)?),
        PlannedAction::Desktop(desktop) => push(steps, lower_desktop(desktop)?),
        PlannedAction::Update(update) => lower_update(update, steps)?,
    }
    Ok(())
}

fn lower_prerequisites(
    prerequisites: &BTreeSet<SystemPrerequisite>,
    managers: &BTreeSet<ManagerBootstrap>,
    steps: &mut Vec<Step>,
) {
    let mut packages = BTreeSet::new();
    for prerequisite in prerequisites {
        packages.extend(match prerequisite {
            SystemPrerequisite::HttpsCertificates => ["ca-certificates", "curl"].as_slice(),
            SystemPrerequisite::OpenPgp => ["gnupg"].as_slice(),
            SystemPrerequisite::ArchiveExtraction => ["tar", "xz-utils"].as_slice(),
            SystemPrerequisite::DebInspection => ["dpkg"].as_slice(),
            SystemPrerequisite::ElfInspection => [].as_slice(),
            SystemPrerequisite::FontCache => ["fontconfig"].as_slice(),
            SystemPrerequisite::DesktopSettings => ["dconf-cli", "libglib2.0-bin"].as_slice(),
            SystemPrerequisite::Stow => ["stow"].as_slice(),
            SystemPrerequisite::GnomeExtensionManagement => ["gnome-shell"].as_slice(),
        });
    }
    if managers.contains(&ManagerBootstrap::Flatpak) {
        packages.insert("flatpak");
    }
    if managers.contains(&ManagerBootstrap::Fnm) {
        packages.insert("unzip");
    }
    if managers.contains(&ManagerBootstrap::CargoBinstall) {
        packages.insert("tar");
    }
    if !packages.is_empty() {
        push(
            steps,
            Operation::AptBootstrapPackages {
                packages: packages.into_iter().map(str::to_owned).collect(),
            },
        );
    }
}

fn lower_managers(
    managers: &BTreeSet<ManagerBootstrap>,
    cargo_binstall_architecture: Option<Architecture>,
    steps: &mut Vec<Step>,
) -> Result<()> {
    for manager in managers {
        push(
            steps,
            match manager {
                ManagerBootstrap::Flatpak => Operation::FlatpakEnsureFlathub,
                ManagerBootstrap::Rustup => Operation::RustupBootstrap,
                ManagerBootstrap::CargoBinstall => Operation::CargoBinstallBootstrap(
                    CargoBinstallBootstrapOperation::new(cargo_binstall_architecture.context(
                        "cargo-binstall bootstrap requires a planned Rust architecture",
                    )?),
                ),
                ManagerBootstrap::Fnm => Operation::FnmBootstrap,
                ManagerBootstrap::Uv => Operation::UvBootstrap,
            },
        );
    }
    Ok(())
}

fn lower_repository(intent: &AptRepositoryIntent) -> Result<AptRepositoryOperation> {
    let layout = match &intent.layout {
        RepositoryLayout::SuiteComponents { suite, components } => {
            AptRepositorySourceLayout::SuiteComponents {
                suite: AptRepositoryToken::parse(suite.value())?,
                components: components
                    .iter()
                    .map(|component| AptRepositoryToken::parse(component.as_str()))
                    .collect::<Result<Vec<_>>>()?,
            }
        }
        RepositoryLayout::ExactPath(path) => {
            AptRepositorySourceLayout::ExactPath(AptRepositoryPath::parse(path)?)
        }
    };
    let operation = AptRepositoryOperation::new(
        intent.name.clone(),
        intent.filename_stem.clone(),
        intent.key_url.clone(),
        intent.source_url.clone(),
        intent.architecture,
        layout,
    )?;
    if operation.keyring_path() != intent.keyring_path
        || operation.source_list_path() != intent.source_list_path
    {
        bail!("neutral repository intent contains inconsistent derived destinations");
    }
    Ok(operation)
}

fn lower_toolchain(intent: &ToolchainIntent) -> Result<Operation> {
    Ok(match intent {
        ToolchainIntent::Rust {
            selector,
            architecture,
        } => Operation::RustToolchain(RustToolchainOperation::new(
            rust_selector(selector),
            *architecture,
            ToolMutationMode::EnsurePresent,
        )?),
        ToolchainIntent::Go {
            selector,
            architecture,
        } => Operation::GoToolchain(GoToolchainOperation::new(
            go_selector(selector),
            *architecture,
            ToolMutationMode::EnsurePresent,
        )?),
        ToolchainIntent::Node {
            selector,
            architecture,
        } => Operation::NodeToolchain(NodeToolchainOperation::new(
            node_selector(selector),
            *architecture,
            ToolMutationMode::EnsurePresent,
        )?),
        ToolchainIntent::Python {
            selector,
            architecture,
        } => Operation::PythonToolchain(PythonToolchainOperation::new(
            selector.0.clone(),
            *architecture,
        )?),
    })
}

fn lower_binary(
    intent: &BinaryPackageIntent,
    mode: BinaryPackageMode,
) -> Result<BinaryPackageOperation> {
    let source = match &intent.source {
        BinarySourceIntent::Github {
            repository,
            release,
            selector,
        } => {
            let GithubReleasePolicy::LatestNonDraftNonPrerelease = release;
            BinarySourceOperation::GithubLatest {
                repository: GithubRepository::parse(repository.clone())?,
                selector: BinaryPackageSelector::new(selector.pattern.clone())?,
                sha256: None,
            }
        }
        BinarySourceIntent::FixedUrl { url, sha256 } => BinarySourceOperation::ChecksummedUrl {
            url: url.clone(),
            sha256: BinarySha256::parse(sha256)?,
        },
    };
    BinaryPackageOperation::new(
        intent.name.clone(),
        match intent.format {
            BinaryFormat::Deb => BinaryPackageFormat::Deb,
            BinaryFormat::Appimage => BinaryPackageFormat::AppImage,
        },
        intent.commands.clone(),
        intent.architecture,
        source,
        mode,
    )
}

fn lower_integration(intent: &IntegrationIntent) -> Result<Operation> {
    Ok(match (&intent.required_product, &intent.action) {
        (ExistingProductRequirement::Docker, IntegrationAction::AddInvokingUserToGroup) => {
            Operation::DockerGroup
        }
        (ExistingProductRequirement::VirtualBox, IntegrationAction::AddInvokingUserToGroup) => {
            Operation::VirtualBoxGroup
        }
        (
            ExistingProductRequirement::Docker,
            IntegrationAction::DockerLocalLogging { max_size },
        ) => Operation::DockerLocalLog(DockerLocalLogOperation::new(max_size.clone())?),
        (ExistingProductRequirement::VsCode, IntegrationAction::VsCodeExtensions(extensions)) => {
            Operation::VsCodeExtensionSet(VsCodeExtensionOperation::new(extensions.clone())?)
        }
        _ => bail!("neutral integration intent has a mismatched product precondition"),
    })
}

fn lower_desktop(intent: &DesktopIntent) -> Result<Operation> {
    Ok(match intent {
        DesktopIntent::Theme { target, theme } => {
            Operation::DesktopSetting(DesktopSettingOperation::new(
                desktop_target(*target),
                DesktopSetting::Theme(match theme {
                    Theme::Light => DesktopTheme::Light,
                    Theme::Dark => DesktopTheme::Dark,
                }),
            )?)
        }
        DesktopIntent::Terminal { target, executable } => {
            let PreconditionTiming::AfterInstallPhases = executable.timing;
            Operation::DesktopSetting(DesktopSettingOperation::new(
                desktop_target(*target),
                DesktopSetting::Terminal(executable.exact_basename.clone()),
            )?)
        }
        DesktopIntent::IdleTimeout { target, timeout } => {
            Operation::DesktopSetting(DesktopSettingOperation::new(
                desktop_target(*target),
                DesktopSetting::IdleTimeoutSeconds(duration_seconds(timeout)?),
            )?)
        }
        DesktopIntent::IdleDim { target, enabled } => {
            Operation::DesktopSetting(DesktopSettingOperation::new(
                desktop_target(*target),
                DesktopSetting::IdleDim(*enabled),
            )?)
        }
        DesktopIntent::GnomeExtensions(extensions) => {
            Operation::GnomeExtensions(GnomeExtensionsOperation::new(extensions.clone())?)
        }
        DesktopIntent::GnomeDock(provider) => {
            let ProviderConvergence::EnsureFixedProviderThenConfigureAndVerify = provider;
            Operation::GnomeDock(GnomeDockOperation::new())
        }
        DesktopIntent::GnomeRoundedCorners(provider) => {
            let ProviderConvergence::EnsureFixedProviderThenConfigureAndVerify = provider;
            Operation::GnomeRoundedCorners(GnomeRoundedCornersOperation::new())
        }
    })
}

fn lower_update(update: &UpdateIntent, steps: &mut Vec<Step>) -> Result<()> {
    match update {
        UpdateIntent::Apt(policy) => push(
            steps,
            Operation::AptUpgrade {
                policy: match policy {
                    AptUpdatePolicy::Standard => AptUpgradePolicy::Standard,
                    AptUpdatePolicy::Full => AptUpgradePolicy::Full,
                },
            },
        ),
        UpdateIntent::Flatpak {
            applications,
            scope,
        } => {
            let FlatpakUpdateScope::ConfiguredApplicationsWithRequiredRefsAndEolReplacements =
                scope;
            push(
                steps,
                Operation::FlatpakUpdateApps {
                    refs: applications.clone(),
                },
            );
        }
        UpdateIntent::Rust {
            selector,
            architecture,
        } => push(
            steps,
            Operation::RustToolchain(RustToolchainOperation::new(
                rust_selector(selector),
                *architecture,
                ToolMutationMode::UpdateMoving,
            )?),
        ),
        UpdateIntent::Go {
            selector,
            architecture,
        } => push(
            steps,
            Operation::GoToolchain(GoToolchainOperation::new(
                go_selector(selector),
                *architecture,
                ToolMutationMode::UpdateMoving,
            )?),
        ),
        UpdateIntent::Node {
            selector,
            architecture,
        } => push(
            steps,
            Operation::NodeToolchain(NodeToolchainOperation::new(
                node_selector(selector),
                *architecture,
                ToolMutationMode::UpdateMoving,
            )?),
        ),
        UpdateIntent::Cargo(packages) => push(
            steps,
            Operation::CargoPackageSet(CargoPackageOperation::new(
                packages.clone(),
                CargoPackageMode::UpdateCurrent,
            )?),
        ),
        UpdateIntent::Npm(packages) => push(
            steps,
            Operation::NpmPackageSet(NpmPackageOperation::new(
                packages.clone(),
                NpmPackageMode::UpdateCurrent,
            )?),
        ),
        UpdateIntent::GithubBinaries(binaries) => {
            for binary in binaries {
                push(
                    steps,
                    Operation::BinaryPackage(lower_binary(binary, BinaryPackageMode::Update)?),
                );
            }
        }
        UpdateIntent::NerdFonts(families) => push(
            steps,
            Operation::NerdFonts(NerdFontsOperation::new(
                families.clone(),
                NerdFontsMode::Update,
            )?),
        ),
    }
    Ok(())
}

fn manager_bootstraps(plan: &Plan) -> Result<BTreeSet<ManagerBootstrap>> {
    let mut found = None;
    for action in plan.actions() {
        if let PlannedAction::ManagerBootstraps(managers) = action {
            if found.replace(managers.clone()).is_some() {
                bail!("neutral plan contains duplicate manager bootstrap actions");
            }
        }
    }
    Ok(found.unwrap_or_default())
}

fn execution_phase(phase: PlanPhaseKind) -> ExecutionPhase {
    match phase {
        PlanPhaseKind::SystemPrerequisites => ExecutionPhase::SystemPrerequisites,
        PlanPhaseKind::ManagerBootstraps => ExecutionPhase::ManagerBootstraps,
        PlanPhaseKind::AdministrativeVerification => ExecutionPhase::AdministrativeVerification,
        PlanPhaseKind::OfficialAptSources => ExecutionPhase::OfficialAptSources,
        PlanPhaseKind::ThirdPartyRepositories => ExecutionPhase::ThirdPartyRepositories,
        PlanPhaseKind::AptMetadataRefresh => ExecutionPhase::AptMetadataRefresh,
        PlanPhaseKind::SystemPackageStates => ExecutionPhase::SystemPackageStates,
        PlanPhaseKind::AptPurge => ExecutionPhase::AptPurge,
        PlanPhaseKind::RepositoryPackages => ExecutionPhase::RepositoryPackages,
        PlanPhaseKind::AptPackages => ExecutionPhase::AptPackages,
        PlanPhaseKind::FlatpakApplications => ExecutionPhase::FlatpakApplications,
        PlanPhaseKind::LanguageToolchains => ExecutionPhase::LanguageToolchains,
        PlanPhaseKind::LanguagePackages => ExecutionPhase::LanguagePackages,
        PlanPhaseKind::BinaryPackages => ExecutionPhase::BinaryPackages,
        PlanPhaseKind::Fonts => ExecutionPhase::Fonts,
        PlanPhaseKind::Dotfiles => ExecutionPhase::Dotfiles,
        PlanPhaseKind::Integrations => ExecutionPhase::Integrations,
        PlanPhaseKind::Desktop => ExecutionPhase::Desktop,
        PlanPhaseKind::Updates => ExecutionPhase::Updates,
        PlanPhaseKind::FinalVerification => ExecutionPhase::FinalVerification,
    }
}

fn rust_selector(selector: &RustSelector) -> RustToolchainSelector {
    match selector {
        RustSelector::Stable => RustToolchainSelector::Stable,
        RustSelector::Beta => RustToolchainSelector::Beta,
        RustSelector::Nightly => RustToolchainSelector::Nightly,
        RustSelector::Pinned(value) if value.starts_with("nightly-") => {
            RustToolchainSelector::DatedNightly(value.clone())
        }
        RustSelector::Pinned(value) => RustToolchainSelector::Version(value.clone()),
    }
}

fn go_selector(selector: &GoSelector) -> GoToolchainSelector {
    match selector {
        GoSelector::Latest => GoToolchainSelector::Latest,
        GoSelector::Pinned(value) => GoToolchainSelector::Version(value.clone()),
    }
}

fn node_selector(selector: &NodeSelector) -> NodeToolchainSelector {
    match selector {
        NodeSelector::Lts => NodeToolchainSelector::Lts,
        NodeSelector::Latest => NodeToolchainSelector::Latest,
        NodeSelector::Pinned(value) => NodeToolchainSelector::Version(value.clone()),
    }
}

fn desktop_target(target: DesktopTarget) -> DesktopEnvironment {
    match target {
        DesktopTarget::Gnome => DesktopEnvironment::Gnome,
        DesktopTarget::Cinnamon => DesktopEnvironment::Cinnamon,
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
        (
            value
                .strip_suffix('h')
                .context("invalid desktop idle duration")?,
            3600,
        )
    };
    number
        .parse::<u64>()
        .context("invalid desktop idle duration")?
        .checked_mul(multiplier)
        .and_then(|seconds| u32::try_from(seconds).ok())
        .context("desktop idle duration exceeds the supported uint32 range")
}

fn push(steps: &mut Vec<Step>, operation: Operation) {
    steps.push(Step::workflow(operation));
}
