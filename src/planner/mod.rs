use crate::{
    config::{
        AptUpdate, BinaryFormat, BinarySource, Config, EnabledDisabled, Theme, resolve_platform_identity,
        select_distro_map, selected_repository_codename,
    },
    operations::{
        AptRepositoryOperation, AptUpgradePolicy, BinaryPackageOperation, BinarySourceOperation, DesktopEnvironment,
        DesktopSetting, DesktopTheme, GoToolchainSelector, NerdFontsMode, Operation,
    },
    platform::{Architecture, Platform},
};
use anyhow::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

mod desktop;
mod dotfiles;
mod fonts;
mod integrations;
mod packages;
mod system;
mod tools;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ExecutionStage {
    AdministrativeVerification,
    PlatformFoundation,
    SystemMetadataRefresh,
    SystemUpdates,
    SystemState,
    SystemPrerequisites,
    SystemManagerBootstrap,
    SystemPackages,
    ThirdPartyRepositories,
    RepositoryMetadataRefresh,
    RepositoryPackages,
    ApplicationManagerBootstraps,
    ApplicationPackages,
    RustManagerBootstrap,
    RustToolchain,
    GoToolchain,
    NodeManagerBootstrap,
    NodeToolchain,
    PythonManagerBootstrap,
    PythonToolchain,
    CargoManagerBootstrap,
    CargoPackages,
    NpmPackages,
    DebBinaryPackages,
    BinaryManagerBootstrap,
    AppImageBinaryPackages,
    Fonts,
    Dotfiles,
    Integrations,
    Desktop,
}

type Stages = BTreeMap<ExecutionStage, Vec<Operation>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ManagerBootstrap {
    Flatpak,
    Rustup,
    Fnm,
    Uv,
    CargoBinstall,
    CargoUpdate,
}

pub fn plan_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    if platform.is_macos() {
        return plan_macos_apply(config, platform.architecture, dotfiles_root);
    }
    plan_linux_apply(config, platform, dotfiles_root)
}

struct LinuxApplyWorkflow<'a> {
    config: &'a Config,
    platform: &'a Platform,
    identity: Option<crate::config::PlatformIdentity>,
    dotfiles_root: &'a Path,
    stages: Stages,
    prerequisites: BTreeSet<&'static str>,
    managers: BTreeSet<ManagerBootstrap>,
    needs_direct_apt_refresh: bool,
    needs_repository_refresh: bool,
}

fn plan_linux_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    let mut workflow = LinuxApplyWorkflow {
        config,
        platform,
        identity: None,
        dotfiles_root,
        stages: Stages::new(),
        prerequisites: BTreeSet::new(),
        managers: BTreeSet::new(),
        needs_direct_apt_refresh: false,
        needs_repository_refresh: false,
    };
    linux_apply_workflow(&mut workflow)?;
    finish_linux_apply_workflow(&mut workflow);
    Ok(flatten_stage_vec(workflow.stages))
}

fn linux_apply_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
    system::linux_system_workflow(workflow)?;
    packages::linux_package_workflow(workflow)?;
    tools::linux_shared_tools_workflow(workflow);
    tools::linux_shared_package_workflow(workflow);
    packages::linux_binary_workflow(workflow);
    fonts::linux_shared_font_workflow(workflow);
    dotfiles::linux_dotfiles_workflow(workflow)?;
    integrations::linux_integration_workflow(workflow);
    desktop::linux_desktop_workflow(workflow);
    Ok(())
}

fn finish_linux_apply_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if workflow.needs_direct_apt_refresh {
        push_operation(&mut workflow.stages, ExecutionStage::SystemMetadataRefresh, Operation::AptMetadataRefresh);
    }

    linux_derived_system_prerequisites_workflow(workflow);
    push_manager_bootstraps(&mut workflow.stages, &workflow.managers);

    if workflow.needs_repository_refresh {
        push_operation(&mut workflow.stages, ExecutionStage::RepositoryMetadataRefresh, Operation::AptMetadataRefresh);
    }
}

fn linux_derived_system_prerequisites_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if workflow.managers.contains(&ManagerBootstrap::Flatpak) {
        workflow.prerequisites.insert("flatpak");
    }
    if workflow.managers.contains(&ManagerBootstrap::Fnm) {
        workflow.prerequisites.insert("unzip");
    }
    if !workflow.prerequisites.is_empty() {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::SystemPrerequisites,
            Operation::AptBootstrapPackages {
                packages: workflow.prerequisites.iter().map(|value| (*value).to_owned()).collect(),
            },
        );
    }
}

pub fn plan_standalone_dotfiles(
    config: &Config,
    platform: &Platform,
    dotfiles_root: &Path,
    replace: bool,
) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    Ok(dotfiles::operation(config, platform, dotfiles_root, replace)?.into_iter().collect())
}

pub fn plan_update(config: &Config, platform: &Platform) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    if platform.is_macos() {
        return plan_macos_update(config, platform.architecture);
    }
    plan_linux_update(config, platform)
}

struct LinuxUpdateWorkflow<'a> {
    config: &'a Config,
    platform: &'a Platform,
    stages: Stages,
    prerequisites: BTreeSet<&'static str>,
    managers: BTreeSet<ManagerBootstrap>,
}

fn plan_linux_update(config: &Config, platform: &Platform) -> Result<Vec<Operation>> {
    let mut workflow = LinuxUpdateWorkflow {
        config,
        platform,
        stages: Stages::new(),
        prerequisites: BTreeSet::new(),
        managers: BTreeSet::new(),
    };
    linux_update_workflow(&mut workflow);
    finish_linux_update_workflow(&mut workflow);
    Ok(flatten_stage_vec(workflow.stages))
}

fn linux_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    packages::linux_apt_update_workflow(workflow);
    packages::linux_flatpak_update_workflow(workflow);
    tools::linux_shared_tool_update_workflow(workflow);
    tools::linux_shared_package_update_workflow(workflow);
    fonts::linux_shared_font_update_workflow(workflow);
}

fn finish_linux_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.managers.contains(&ManagerBootstrap::Fnm) {
        workflow.prerequisites.insert("unzip");
    }
    if !workflow.prerequisites.is_empty() {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::SystemPrerequisites,
            Operation::AptBootstrapPackages {
                packages: workflow.prerequisites.iter().map(|value| (*value).to_owned()).collect(),
            },
        );
    }
    push_manager_bootstraps(&mut workflow.stages, &workflow.managers);
}

fn plan_macos_apply(config: &Config, architecture: Architecture, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    let mut stages = Stages::new();
    let mut managers = BTreeSet::new();
    macos_apply_workflow(config, architecture, dotfiles_root, &mut stages, &mut managers)?;
    push_manager_bootstraps(&mut stages, &managers);
    Ok(flatten_stage_vec(stages))
}

fn macos_apply_workflow(
    config: &Config,
    architecture: Architecture,
    dotfiles_root: &Path,
    stages: &mut Stages,
    managers: &mut BTreeSet<ManagerBootstrap>,
) -> Result<()> {
    system::macos_system_workflow(config, stages);
    packages::macos_homebrew_workflow(config, stages);
    tools::macos_shared_tools_workflow(config, architecture, stages, managers);
    tools::macos_shared_package_workflow(config, stages, managers);
    fonts::macos_shared_font_workflow(config, stages);
    dotfiles::macos_dotfiles_workflow(config, dotfiles_root, stages)?;
    integrations::macos_integration_workflow(config, stages);
    desktop::macos_desktop_workflow(config, stages);
    Ok(())
}

fn plan_macos_update(config: &Config, architecture: Architecture) -> Result<Vec<Operation>> {
    let mut workflow = MacosUpdateWorkflow { config, architecture, stages: Stages::new(), managers: BTreeSet::new() };
    macos_update_workflow(&mut workflow);
    push_manager_bootstraps(&mut workflow.stages, &workflow.managers);
    Ok(flatten_stage_vec(workflow.stages))
}

struct MacosUpdateWorkflow<'a> {
    config: &'a Config,
    architecture: Architecture,
    stages: Stages,
    managers: BTreeSet<ManagerBootstrap>,
}

fn macos_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    packages::macos_homebrew_update_workflow(workflow);
    tools::macos_shared_tool_update_workflow(workflow);
    tools::macos_shared_package_update_workflow(workflow);
    fonts::macos_shared_font_update_workflow(workflow);
}

fn push_operation(stages: &mut Stages, stage: ExecutionStage, op: Operation) {
    stages.entry(stage).or_default().push(op);
}

fn push_manager_bootstraps(stages: &mut Stages, managers: &BTreeSet<ManagerBootstrap>) {
    for manager in managers {
        let (stage, operation) = match manager {
            ManagerBootstrap::Flatpak => {
                (ExecutionStage::ApplicationManagerBootstraps, Operation::FlatpakEnsureFlathub)
            }
            ManagerBootstrap::Rustup => (ExecutionStage::RustManagerBootstrap, Operation::RustupBootstrap),
            ManagerBootstrap::Fnm => (ExecutionStage::NodeManagerBootstrap, Operation::FnmBootstrap),
            ManagerBootstrap::Uv => (ExecutionStage::PythonManagerBootstrap, Operation::UvBootstrap),
            ManagerBootstrap::CargoBinstall => {
                (ExecutionStage::CargoManagerBootstrap, Operation::CargoBinstallBootstrap)
            }
            ManagerBootstrap::CargoUpdate => (ExecutionStage::CargoManagerBootstrap, Operation::CargoUpdateBootstrap),
        };
        push_operation(stages, stage, operation);
    }
}

fn flatten_stage_vec(stages: Stages) -> Vec<Operation> {
    stages.into_values().flatten().collect()
}
