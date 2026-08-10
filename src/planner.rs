use crate::{
    config::{
        AptUpdate, BinaryFormat, BinarySource, Config, EnabledDisabled, Theme, resolve_platform_identity,
        select_distro_map, selected_repository_codename,
    },
    operations::{
        AptRepositoryOperation, AptUpgradePolicy, BinaryPackageOperation, BinarySourceOperation, DesktopEnvironment,
        DesktopSetting, DesktopTheme, GoToolchainSelector, NerdFontsMode, Operation, ToolchainMode,
    },
    platform::{Architecture, Platform},
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

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
    BinaryManagerBootstrap,
    Fonts,
    Integrations,
    Dotfiles,
    Desktop,
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
    AppImageBinaryPackages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ManagerBootstrap {
    Flatpak,
    Rustup,
    Fnm,
    Uv,
    CargoBinstall,
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
    stages: Vec<(ExecutionStage, Vec<Operation>)>,
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
        stages: vec![
            (ExecutionStage::AdministrativeVerification, Vec::new()),
            (ExecutionStage::PlatformFoundation, Vec::new()),
            (ExecutionStage::SystemMetadataRefresh, Vec::new()),
            (ExecutionStage::SystemState, Vec::new()),
            (ExecutionStage::SystemPrerequisites, Vec::new()),
            (ExecutionStage::SystemPackages, Vec::new()),
            (ExecutionStage::ThirdPartyRepositories, Vec::new()),
            (ExecutionStage::RepositoryMetadataRefresh, Vec::new()),
            (ExecutionStage::RepositoryPackages, Vec::new()),
            (ExecutionStage::ApplicationManagerBootstraps, Vec::new()),
            (ExecutionStage::ApplicationPackages, Vec::new()),
            (ExecutionStage::RustManagerBootstrap, Vec::new()),
            (ExecutionStage::RustToolchain, Vec::new()),
            (ExecutionStage::GoToolchain, Vec::new()),
            (ExecutionStage::NodeManagerBootstrap, Vec::new()),
            (ExecutionStage::NodeToolchain, Vec::new()),
            (ExecutionStage::PythonManagerBootstrap, Vec::new()),
            (ExecutionStage::PythonToolchain, Vec::new()),
            (ExecutionStage::CargoManagerBootstrap, Vec::new()),
            (ExecutionStage::CargoPackages, Vec::new()),
            (ExecutionStage::NpmPackages, Vec::new()),
            (ExecutionStage::DebBinaryPackages, Vec::new()),
            (ExecutionStage::BinaryManagerBootstrap, Vec::new()),
            (ExecutionStage::AppImageBinaryPackages, Vec::new()),
            (ExecutionStage::Fonts, Vec::new()),
            (ExecutionStage::Dotfiles, Vec::new()),
            (ExecutionStage::Integrations, Vec::new()),
            (ExecutionStage::Desktop, Vec::new()),
        ],
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
    linux_system_workflow(workflow)?;
    linux_package_workflow(workflow)?;
    linux_shared_tools_workflow(workflow);
    linux_shared_package_workflow(workflow);
    linux_binary_workflow(workflow);
    linux_shared_font_workflow(workflow);
    linux_dotfiles_workflow(workflow)?;
    linux_integration_workflow(workflow);
    linux_desktop_workflow(workflow);
    Ok(())
}

fn linux_system_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
    linux_administrative_access_workflow(workflow);
    linux_platform_requirements_workflow(workflow)?;
    linux_debian_apt_components_workflow(workflow);
    linux_system_state_workflow(workflow);
    Ok(())
}

fn linux_administrative_access_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if workflow.config.os.linux.system.ensure_admin == Some(true) {
        push_operation(&mut workflow.stages, ExecutionStage::AdministrativeVerification, Operation::EnsureAdmin);
    }
}

fn linux_platform_requirements_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
    workflow.identity = Some(resolve_platform_identity(workflow.platform)?);
    Ok(())
}

fn linux_debian_apt_components_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if workflow.platform.distro == "debian" {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::PlatformFoundation,
            Operation::EnsureDebianAptComponents { release: workflow.platform.distro_codename.clone() },
        );
    }
}

fn linux_system_state_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    plan_system_states(
        workflow.config,
        workflow.platform,
        &mut workflow.stages,
        &mut workflow.needs_direct_apt_refresh,
    );
}

fn linux_package_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
    linux_apt_workflow(workflow)?;
    linux_flatpak_workflow(workflow);
    Ok(())
}

fn linux_apt_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
    linux_direct_apt_packages_workflow(workflow);
    linux_third_party_repository_workflows(workflow)?;
    Ok(())
}

fn linux_direct_apt_packages_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    let apt = workflow.config.os.linux.packages.apt.as_ref();

    if let Some(install) = apt.and_then(|apt| apt.install.as_ref()).filter(|values| !values.is_empty()) {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::SystemPackages,
            Operation::AptPackages { packages: install.clone() },
        );
        workflow.needs_direct_apt_refresh = true;
    }
}

fn linux_third_party_repository_workflows(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
    let apt = workflow.config.os.linux.packages.apt.as_ref();
    let identity =
        workflow.identity.ok_or_else(|| anyhow::anyhow!("Linux platform requirements workflow did not run"))?;
    if let Some(repositories) = apt.and_then(|apt| apt.repositories.as_ref()).filter(|values| !values.is_empty()) {
        for repository in repositories {
            let Some(operation) = plan_repository(repository, workflow.platform, identity)? else {
                continue;
            };
            workflow.prerequisites.extend(["ca-certificates", "curl", "gnupg"]);
            push_operation(
                &mut workflow.stages,
                ExecutionStage::ThirdPartyRepositories,
                Operation::AptRepository(Box::new(operation)),
            );
            linux_repository_packages_workflow(workflow, repository, identity);
            workflow.needs_repository_refresh = true;
        }
    }
    Ok(())
}

fn linux_repository_packages_workflow(
    workflow: &mut LinuxApplyWorkflow<'_>,
    repository: &crate::config::Repository,
    identity: crate::config::PlatformIdentity,
) {
    if !repository.packages.is_empty() {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::RepositoryPackages,
            Operation::AptRepositoryPackages {
                conflicts: selected_repository_conflicts(repository, identity).unwrap_or_default(),
                packages: repository.packages.clone(),
            },
        );
    }
}

fn linux_flatpak_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(applications) = workflow.config.os.linux.packages.flatpak.as_ref().filter(|values| !values.is_empty()) {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.insert(ManagerBootstrap::Flatpak);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::ApplicationPackages,
            Operation::FlatpakEnsureApps { refs: applications.clone() },
        );
    }
}

fn linux_shared_tools_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    linux_rust_workflow(workflow);
    linux_go_workflow(workflow);
    linux_node_workflow(workflow);
    linux_python_workflow(workflow);
}

fn linux_rust_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(selector) = workflow.config.shared.tools.rust.as_deref() {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::RustToolchain,
            Operation::RustToolchain { selector: Some(selector.to_owned()), mode: ToolchainMode::EnsurePresent },
        );
    }
}

fn linux_go_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(selector) = workflow.config.shared.tools.go.as_deref() {
        workflow.prerequisites.extend(["ca-certificates", "curl", "tar"]);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::GoToolchain,
            Operation::GoToolchain {
                selector: go_selector_main(selector),
                architecture: workflow.platform.architecture,
                mode: ToolchainMode::EnsurePresent,
            },
        );
    }
}

fn linux_node_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(selector) = workflow.config.shared.tools.node.as_deref() {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::NodeToolchain,
            Operation::NodeToolchain { selector: selector.to_owned(), mode: ToolchainMode::EnsurePresent },
        );
    }
}

fn linux_python_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(version) = &workflow.config.shared.tools.python {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.insert(ManagerBootstrap::Uv);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::PythonToolchain,
            Operation::PythonToolchain { version: version.clone(), mode: ToolchainMode::EnsurePresent },
        );
    }
}

fn linux_shared_package_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    linux_cargo_workflow(workflow);
    linux_npm_workflow(workflow);
}

fn linux_cargo_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(cargo) = workflow.config.shared.packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.extend([ManagerBootstrap::Rustup, ManagerBootstrap::CargoBinstall]);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::CargoPackages,
            Operation::CargoPackageSet { packages: cargo.clone() },
        );
    }
}

fn linux_npm_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(npm) = workflow.config.shared.packages.npm.as_ref().filter(|values| !values.is_empty()) {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::NpmPackages,
            Operation::NpmPackageSet { packages: npm.clone() },
        );
    }
}

fn linux_binary_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    linux_deb_package_workflows(workflow);
    linux_appimage_workflows(workflow);
}

fn linux_deb_package_workflows(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(binaries) = workflow.config.os.linux.packages.binaries.as_ref().filter(|values| !values.is_empty()) {
        for binary in binaries.iter().filter(|binary| binary.format == BinaryFormat::Deb) {
            let Some(planned) = plan_binary(binary, workflow.platform.architecture) else {
                continue;
            };
            workflow.prerequisites.extend(["ca-certificates", "curl"]);
            workflow.needs_direct_apt_refresh = true;
            push_operation(&mut workflow.stages, ExecutionStage::DebBinaryPackages, Operation::BinaryPackage(planned));
        }
    }
}

fn linux_appimage_workflows(workflow: &mut LinuxApplyWorkflow<'_>) {
    let mut packages = Vec::new();
    if let Some(binaries) = workflow.config.os.linux.packages.binaries.as_ref().filter(|values| !values.is_empty()) {
        for binary in binaries.iter().filter(|binary| binary.format == BinaryFormat::Appimage) {
            let Some(planned) = plan_binary(binary, workflow.platform.architecture) else {
                continue;
            };
            workflow.prerequisites.extend(["ca-certificates", "curl"]);
            packages.push(Operation::BinaryPackage(planned));
        }
    }
    if !packages.is_empty() {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::BinaryManagerBootstrap,
            Operation::Appimaged { architecture: workflow.platform.architecture },
        );
        for package in packages {
            push_operation(&mut workflow.stages, ExecutionStage::AppImageBinaryPackages, package);
        }
    }
}

fn linux_shared_font_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    linux_nerd_fonts_workflow(workflow);
}

fn linux_nerd_fonts_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(fonts) = workflow.config.shared.fonts.nerd.as_ref().filter(|values| !values.is_empty()) {
        workflow.prerequisites.extend(["ca-certificates", "curl", "tar", "xz-utils", "fontconfig"]);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::Fonts,
            Operation::NerdFonts { families: fonts.clone(), mode: NerdFontsMode::EnsurePresent },
        );
    }
}

fn linux_dotfiles_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
    let mut dotfiles = Vec::new();
    linux_shared_dotfiles_workflow(workflow, &mut dotfiles);
    linux_platform_dotfiles_workflow(workflow, &mut dotfiles);
    if !dotfiles.is_empty() {
        if workflow.dotfiles_root.as_os_str().is_empty() {
            anyhow::bail!("dotfiles root must not be empty");
        }
        workflow.prerequisites.insert("stow");
        push_operation(
            &mut workflow.stages,
            ExecutionStage::Dotfiles,
            Operation::Dotfiles { root: workflow.dotfiles_root.to_path_buf(), packages: dotfiles, replace: false },
        );
    }
    Ok(())
}

fn linux_shared_dotfiles_workflow(workflow: &LinuxApplyWorkflow<'_>, packages: &mut Vec<String>) {
    packages.extend(workflow.config.shared.dotfiles.packages.iter().cloned());
}

fn linux_platform_dotfiles_workflow(workflow: &LinuxApplyWorkflow<'_>, packages: &mut Vec<String>) {
    packages.extend(workflow.config.os.linux.dotfiles.packages.iter().cloned());
}

fn linux_integration_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    linux_docker_workflow(workflow.config, &mut workflow.stages);
    linux_virtualbox_workflow(workflow.config, &mut workflow.stages);
    linux_vscode_workflow(workflow.config, &mut workflow.stages);
}

fn linux_desktop_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    linux_desktop_settings_workflow(
        workflow.config,
        workflow.platform,
        &mut workflow.stages,
        &mut workflow.prerequisites,
    );
}

fn finish_linux_apply_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if workflow.needs_direct_apt_refresh {
        push_operation(&mut workflow.stages, ExecutionStage::SystemMetadataRefresh, Operation::AptMetadataRefresh);
    }

    linux_derived_system_prerequisites_workflow(workflow);
    push_apply_manager_bootstraps(&mut workflow.stages, &workflow.managers);

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
    let mut packages = Vec::new();
    standalone_shared_dotfiles_workflow(config, &mut packages);
    standalone_current_platform_dotfiles_workflow(config, platform, &mut packages);
    standalone_dotfiles_conflict_and_convergence_workflow(dotfiles_root, packages, replace)
}

fn standalone_shared_dotfiles_workflow(config: &Config, packages: &mut Vec<String>) {
    packages.extend(config.shared.dotfiles.packages.iter().cloned());
}

fn standalone_current_platform_dotfiles_workflow(config: &Config, platform: &Platform, packages: &mut Vec<String>) {
    let platform_packages =
        if platform.is_macos() { &config.os.macos.dotfiles.packages } else { &config.os.linux.dotfiles.packages };
    packages.extend(platform_packages.iter().cloned());
}

fn standalone_dotfiles_conflict_and_convergence_workflow(
    dotfiles_root: &Path,
    packages: Vec<String>,
    replace: bool,
) -> Result<Vec<Operation>> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    if dotfiles_root.as_os_str().is_empty() {
        anyhow::bail!("dotfiles root must not be empty");
    }
    Ok(vec![Operation::Dotfiles { root: dotfiles_root.to_path_buf(), packages, replace }])
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
    stages: Vec<(ExecutionStage, Vec<Operation>)>,
    prerequisites: BTreeSet<&'static str>,
    managers: BTreeSet<ManagerBootstrap>,
}

fn plan_linux_update(config: &Config, platform: &Platform) -> Result<Vec<Operation>> {
    let mut workflow = LinuxUpdateWorkflow {
        config,
        platform,
        stages: vec![
            (ExecutionStage::SystemMetadataRefresh, Vec::new()),
            (ExecutionStage::SystemUpdates, Vec::new()),
            (ExecutionStage::SystemPrerequisites, Vec::new()),
            (ExecutionStage::ApplicationManagerBootstraps, Vec::new()),
            (ExecutionStage::ApplicationPackages, Vec::new()),
            (ExecutionStage::RustManagerBootstrap, Vec::new()),
            (ExecutionStage::RustToolchain, Vec::new()),
            (ExecutionStage::GoToolchain, Vec::new()),
            (ExecutionStage::NodeManagerBootstrap, Vec::new()),
            (ExecutionStage::NodeToolchain, Vec::new()),
            (ExecutionStage::PythonManagerBootstrap, Vec::new()),
            (ExecutionStage::PythonToolchain, Vec::new()),
            (ExecutionStage::CargoPackages, Vec::new()),
            (ExecutionStage::NpmPackages, Vec::new()),
            (ExecutionStage::Fonts, Vec::new()),
        ],
        prerequisites: BTreeSet::new(),
        managers: BTreeSet::new(),
    };
    linux_update_workflow(&mut workflow);
    finish_linux_update_workflow(&mut workflow);
    Ok(flatten_stage_vec(workflow.stages))
}

fn linux_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    linux_apt_update_workflow(workflow);
    linux_flatpak_update_workflow(workflow);
    linux_shared_tool_update_workflow(workflow);
    linux_shared_package_update_workflow(workflow);
    linux_shared_font_update_workflow(workflow);
}

fn linux_apt_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    let Some(policy) = workflow.config.os.linux.updates.as_ref().and_then(|updates| updates.apt) else {
        return;
    };
    linux_apt_metadata_refresh_workflow(workflow);
    linux_apt_system_upgrade_workflow(workflow, policy);
}

fn linux_apt_metadata_refresh_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    push_operation(&mut workflow.stages, ExecutionStage::SystemMetadataRefresh, Operation::AptMetadataRefresh);
}

fn linux_apt_system_upgrade_workflow(workflow: &mut LinuxUpdateWorkflow<'_>, policy: AptUpdate) {
    push_operation(
        &mut workflow.stages,
        ExecutionStage::SystemUpdates,
        Operation::AptUpgrade {
            policy: match policy {
                AptUpdate::Standard => AptUpgradePolicy::Standard,
                AptUpdate::Full => AptUpgradePolicy::Full,
            },
        },
    );
}

fn linux_flatpak_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.os.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        workflow.prerequisites.insert("flatpak");
        push_operation(&mut workflow.stages, ExecutionStage::ApplicationPackages, Operation::FlatpakUpdateApps);
    }
}

fn linux_shared_tool_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    linux_rust_update_workflow(workflow);
    linux_go_update_workflow(workflow);
    linux_node_update_workflow(workflow);
    linux_python_update_workflow(workflow);
}

fn linux_rust_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.tools.rust == Some(true) {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::RustToolchain,
            Operation::RustToolchain {
                selector: workflow.config.shared.tools.rust.clone(),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

fn linux_go_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.tools.go == Some(true) {
        workflow.prerequisites.extend(["ca-certificates", "curl", "tar"]);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::GoToolchain,
            Operation::GoToolchain {
                selector: go_selector_main(workflow.config.shared.tools.go.as_deref().unwrap_or("latest")),
                architecture: workflow.platform.architecture,
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

fn linux_node_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.tools.node == Some(true) {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::NodeToolchain,
            Operation::NodeToolchain {
                selector: workflow.config.shared.tools.node.clone().unwrap_or_else(|| "latest".to_owned()),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

fn linux_python_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.tools.python == Some(true) {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
        workflow.managers.insert(ManagerBootstrap::Uv);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::PythonToolchain,
            Operation::PythonToolchain {
                version: workflow.config.shared.tools.python.clone().unwrap_or_else(|| "3".to_owned()),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

fn linux_shared_package_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    linux_cargo_update_workflow(workflow);
    linux_npm_update_workflow(workflow);
}

fn linux_cargo_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.packages.cargo == Some(true) {
        push_operation(&mut workflow.stages, ExecutionStage::CargoPackages, Operation::CargoPackageUpdate);
    }
}

fn linux_npm_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.packages.npm == Some(true) {
        push_operation(&mut workflow.stages, ExecutionStage::NpmPackages, Operation::NpmPackageUpdate);
    }
}

fn linux_shared_font_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    linux_nerd_fonts_update_workflow(workflow);
}

fn linux_nerd_fonts_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.fonts == Some(true)
        && let Some(families) = workflow.config.shared.fonts.nerd.as_ref().filter(|families| !families.is_empty())
    {
        workflow.prerequisites.extend(["ca-certificates", "curl", "tar", "xz-utils", "fontconfig"]);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::Fonts,
            Operation::NerdFonts { families: families.clone(), mode: NerdFontsMode::Update },
        );
    }
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
    push_update_manager_bootstraps(&mut workflow.stages, &workflow.managers);
}

fn plan_macos_apply(config: &Config, architecture: Architecture, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    let mut stages = vec![
        (ExecutionStage::AdministrativeVerification, Vec::new()),
        (ExecutionStage::PlatformFoundation, Vec::new()),
        (ExecutionStage::SystemManagerBootstrap, Vec::new()),
        (ExecutionStage::SystemPackages, Vec::new()),
        (ExecutionStage::RustManagerBootstrap, Vec::new()),
        (ExecutionStage::RustToolchain, Vec::new()),
        (ExecutionStage::GoToolchain, Vec::new()),
        (ExecutionStage::NodeManagerBootstrap, Vec::new()),
        (ExecutionStage::NodeToolchain, Vec::new()),
        (ExecutionStage::PythonManagerBootstrap, Vec::new()),
        (ExecutionStage::PythonToolchain, Vec::new()),
        (ExecutionStage::CargoManagerBootstrap, Vec::new()),
        (ExecutionStage::CargoPackages, Vec::new()),
        (ExecutionStage::NpmPackages, Vec::new()),
        (ExecutionStage::Fonts, Vec::new()),
        (ExecutionStage::Dotfiles, Vec::new()),
        (ExecutionStage::Integrations, Vec::new()),
        (ExecutionStage::Desktop, Vec::new()),
    ];
    let mut managers = BTreeSet::new();
    macos_apply_workflow(config, architecture, dotfiles_root, &mut stages, &mut managers);
    push_apply_manager_bootstraps(&mut stages, &managers);
    Ok(flatten_stage_vec(stages))
}

fn macos_apply_workflow(
    config: &Config,
    architecture: Architecture,
    dotfiles_root: &Path,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    macos_system_workflow(config, stages);
    macos_homebrew_workflow(config, stages);
    macos_shared_tools_workflow(config, architecture, stages, managers);
    macos_shared_package_workflow(config, stages, managers);
    macos_shared_font_workflow(config, stages);
    macos_dotfiles_workflow(config, dotfiles_root, stages);
    macos_integration_workflow(config, stages);
    macos_desktop_workflow(config, stages);
}

fn macos_system_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    macos_administrative_access_workflow(config, stages);
    macos_xcode_command_line_tools_workflow(config, stages);
    macos_rosetta_workflow(config, stages);
}

fn macos_administrative_access_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if config.macos().system.ensure_admin == Some(true) {
        push_operation(stages, ExecutionStage::AdministrativeVerification, Operation::MacEnsureAdmin);
    }
}

fn macos_xcode_command_line_tools_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if config.macos().system.xcode.command_line_tools == Some(true) {
        push_operation(stages, ExecutionStage::PlatformFoundation, Operation::XcodeCommandLineTools);
    }
}

fn macos_rosetta_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if config.macos().system.rosetta == Some(true) {
        push_operation(stages, ExecutionStage::PlatformFoundation, Operation::Rosetta);
    }
}

fn macos_homebrew_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    let mac = config.macos();
    let dotfiles = config.shared.dotfiles.packages.iter().chain(mac.dotfiles.packages.iter()).next().is_some();
    let has_packages = dotfiles || !mac.homebrew.formulae.is_empty() || !mac.homebrew.casks.is_empty();
    macos_homebrew_availability_workflow(has_packages, stages);
    let formulae = macos_homebrew_formulae_workflow(config, dotfiles);
    let casks = macos_homebrew_casks_workflow(config);
    macos_homebrew_packages_operation(formulae, casks, stages);
}

fn macos_homebrew_availability_workflow(has_packages: bool, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if has_packages {
        push_operation(stages, ExecutionStage::SystemManagerBootstrap, Operation::HomebrewBootstrap);
    }
}

fn macos_homebrew_formulae_workflow(config: &Config, dotfiles: bool) -> Vec<String> {
    let mut formulae = config.macos().homebrew.formulae.clone();
    if dotfiles && !formulae.iter().any(|formula| formula == "stow") {
        formulae.push("stow".into());
    }
    formulae
}

fn macos_homebrew_casks_workflow(config: &Config) -> Vec<String> {
    config.macos().homebrew.casks.clone()
}

fn macos_homebrew_packages_operation(
    formulae: Vec<String>,
    casks: Vec<String>,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
) {
    if !formulae.is_empty() || !casks.is_empty() {
        push_operation(stages, ExecutionStage::SystemPackages, Operation::HomebrewPackages { formulae, casks });
    }
}

fn macos_shared_tools_workflow(
    config: &Config,
    architecture: Architecture,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    macos_rust_workflow(config, stages, managers);
    macos_go_workflow(config, architecture, stages);
    macos_node_workflow(config, stages, managers);
    macos_python_workflow(config, stages, managers);
}

fn macos_rust_workflow(
    config: &Config,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    if let Some(selector) = &config.shared.tools.rust {
        managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            stages,
            ExecutionStage::RustToolchain,
            Operation::RustToolchain { selector: Some(selector.clone()), mode: ToolchainMode::EnsurePresent },
        );
    }
}

fn macos_go_workflow(config: &Config, architecture: Architecture, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if let Some(selector) = &config.shared.tools.go {
        push_operation(
            stages,
            ExecutionStage::GoToolchain,
            Operation::GoToolchain {
                selector: go_selector_main(selector),
                architecture,
                mode: ToolchainMode::EnsurePresent,
            },
        );
    }
}

fn macos_node_workflow(
    config: &Config,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    if let Some(selector) = &config.shared.tools.node {
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            stages,
            ExecutionStage::NodeToolchain,
            Operation::NodeToolchain { selector: selector.clone(), mode: ToolchainMode::EnsurePresent },
        );
    }
}

fn macos_python_workflow(
    config: &Config,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    if let Some(version) = &config.shared.tools.python {
        managers.insert(ManagerBootstrap::Uv);
        push_operation(
            stages,
            ExecutionStage::PythonToolchain,
            Operation::PythonToolchain { version: version.clone(), mode: ToolchainMode::EnsurePresent },
        );
    }
}

fn macos_shared_package_workflow(
    config: &Config,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    macos_cargo_workflow(config, stages, managers);
    macos_npm_workflow(config, stages);
}

fn macos_cargo_workflow(
    config: &Config,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    if let Some(packages) = config.shared.packages.cargo.as_ref().filter(|packages| !packages.is_empty()) {
        managers.extend([ManagerBootstrap::Rustup, ManagerBootstrap::CargoBinstall]);
        push_operation(
            stages,
            ExecutionStage::CargoPackages,
            Operation::CargoPackageSet { packages: packages.clone() },
        );
    }
}

fn macos_npm_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if let Some(packages) = config.shared.packages.npm.as_ref().filter(|packages| !packages.is_empty()) {
        push_operation(stages, ExecutionStage::NpmPackages, Operation::NpmPackageSet { packages: packages.clone() });
    }
}

fn macos_shared_font_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    macos_nerd_fonts_workflow(config, stages);
}

fn macos_nerd_fonts_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if let Some(families) = config.shared.fonts.nerd.as_ref().filter(|families| !families.is_empty()) {
        push_operation(
            stages,
            ExecutionStage::Fonts,
            Operation::UserNerdFonts { families: families.clone(), mode: NerdFontsMode::EnsurePresent },
        );
    }
}

fn macos_dotfiles_workflow(config: &Config, dotfiles_root: &Path, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    let mut packages = Vec::new();
    macos_shared_dotfiles_workflow(config, &mut packages);
    macos_platform_dotfiles_workflow(config, &mut packages);
    if !packages.is_empty() {
        push_operation(
            stages,
            ExecutionStage::Dotfiles,
            Operation::Dotfiles { root: dotfiles_root.to_path_buf(), packages, replace: false },
        );
    }
}

fn macos_shared_dotfiles_workflow(config: &Config, packages: &mut Vec<String>) {
    packages.extend(config.shared.dotfiles.packages.iter().cloned());
}

fn macos_platform_dotfiles_workflow(config: &Config, packages: &mut Vec<String>) {
    packages.extend(config.macos().dotfiles.packages.iter().cloned());
}

fn macos_integration_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    macos_vscode_workflow(config, stages);
}

fn macos_vscode_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if !config.shared.integrations.vscode.extensions.is_empty() {
        push_operation(
            stages,
            ExecutionStage::Integrations,
            Operation::VsCodeExtensionSet { extensions: config.shared.integrations.vscode.extensions.clone() },
        );
    }
}

fn macos_desktop_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    let mut settings = Vec::new();
    macos_appearance_workflow(config, &mut settings);
    macos_dock_workflow(config, &mut settings);
    macos_finder_workflow(config, &mut settings);
    macos_keyboard_workflow(config, &mut settings);
    macos_trackpad_workflow(config, &mut settings);
    if !settings.is_empty() {
        push_operation(stages, ExecutionStage::Desktop, Operation::MacDefaults { settings });
    }
}

fn macos_appearance_workflow(config: &Config, settings: &mut Vec<crate::operations::macos::MacDefault>) {
    if let Some(value) = config.macos().desktop.appearance {
        settings.push(crate::operations::macos::MacDefault::Appearance(value == Theme::Dark));
    }
}

fn macos_dock_workflow(config: &Config, settings: &mut Vec<crate::operations::macos::MacDefault>) {
    if let Some(dock) = &config.macos().desktop.dock {
        if let Some(value) = dock.autohide {
            settings.push(crate::operations::macos::MacDefault::DockAutohide(value));
        }
        if let Some(value) = dock.show_recent_applications {
            settings.push(crate::operations::macos::MacDefault::DockRecentApplications(value));
        }
    }
}

fn macos_finder_workflow(config: &Config, settings: &mut Vec<crate::operations::macos::MacDefault>) {
    if let Some(finder) = &config.macos().desktop.finder {
        if let Some(value) = finder.show_filename_extensions {
            settings.push(crate::operations::macos::MacDefault::FinderExtensions(value));
        }
        if let Some(value) = finder.show_hidden_files {
            settings.push(crate::operations::macos::MacDefault::FinderHiddenFiles(value));
        }
    }
}

fn macos_keyboard_workflow(config: &Config, settings: &mut Vec<crate::operations::macos::MacDefault>) {
    if let Some(keyboard) = &config.macos().desktop.keyboard {
        if let Some(value) = keyboard.key_repeat {
            settings.push(crate::operations::macos::MacDefault::KeyRepeat(value));
        }
        if let Some(value) = keyboard.initial_key_repeat {
            settings.push(crate::operations::macos::MacDefault::InitialKeyRepeat(value));
        }
    }
}

fn macos_trackpad_workflow(config: &Config, settings: &mut Vec<crate::operations::macos::MacDefault>) {
    if let Some(trackpad) = &config.macos().desktop.trackpad
        && let Some(value) = trackpad.tap_to_click
    {
        settings.push(crate::operations::macos::MacDefault::TrackpadTapToClick(value));
    }
}

fn plan_macos_update(config: &Config, architecture: Architecture) -> Result<Vec<Operation>> {
    let mut workflow = MacosUpdateWorkflow {
        config,
        architecture,
        stages: vec![
            (ExecutionStage::SystemUpdates, Vec::new()),
            (ExecutionStage::RustManagerBootstrap, Vec::new()),
            (ExecutionStage::RustToolchain, Vec::new()),
            (ExecutionStage::GoToolchain, Vec::new()),
            (ExecutionStage::NodeManagerBootstrap, Vec::new()),
            (ExecutionStage::NodeToolchain, Vec::new()),
            (ExecutionStage::PythonManagerBootstrap, Vec::new()),
            (ExecutionStage::PythonToolchain, Vec::new()),
            (ExecutionStage::CargoPackages, Vec::new()),
            (ExecutionStage::NpmPackages, Vec::new()),
            (ExecutionStage::Fonts, Vec::new()),
        ],
        managers: BTreeSet::new(),
    };
    macos_update_workflow(&mut workflow);
    push_update_manager_bootstraps(&mut workflow.stages, &workflow.managers);
    Ok(flatten_stage_vec(workflow.stages))
}

struct MacosUpdateWorkflow<'a> {
    config: &'a Config,
    architecture: Architecture,
    stages: Vec<(ExecutionStage, Vec<Operation>)>,
    managers: BTreeSet<ManagerBootstrap>,
}

fn macos_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    macos_homebrew_update_workflow(workflow);
    macos_shared_tool_update_workflow(workflow);
    macos_shared_package_update_workflow(workflow);
    macos_shared_font_update_workflow(workflow);
}

fn macos_homebrew_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    let formulae = macos_homebrew_formulae_update_workflow(workflow);
    let casks = macos_homebrew_casks_update_workflow(workflow);
    if formulae || casks {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::SystemUpdates,
            Operation::HomebrewUpdate { formulae, casks },
        );
    }
}

fn macos_homebrew_formulae_update_workflow(workflow: &MacosUpdateWorkflow<'_>) -> bool {
    workflow.config.macos().updates.homebrew.formulae == Some(true)
}

fn macos_homebrew_casks_update_workflow(workflow: &MacosUpdateWorkflow<'_>) -> bool {
    workflow.config.macos().updates.homebrew.casks == Some(true)
}

fn macos_shared_tool_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    macos_rust_update_workflow(workflow);
    macos_go_update_workflow(workflow);
    macos_node_update_workflow(workflow);
    macos_python_update_workflow(workflow);
}

fn macos_rust_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.tools.rust == Some(true) {
        workflow.managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::RustToolchain,
            Operation::RustToolchain {
                selector: workflow.config.shared.tools.rust.clone(),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

fn macos_go_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.tools.go == Some(true) {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::GoToolchain,
            Operation::GoToolchain {
                selector: go_selector_main(workflow.config.shared.tools.go.as_deref().unwrap_or("latest")),
                architecture: workflow.architecture,
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

fn macos_node_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.tools.node == Some(true) {
        workflow.managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::NodeToolchain,
            Operation::NodeToolchain {
                selector: workflow.config.shared.tools.node.clone().unwrap_or_else(|| "latest".into()),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

fn macos_python_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.tools.python == Some(true) {
        workflow.managers.insert(ManagerBootstrap::Uv);
        push_operation(
            &mut workflow.stages,
            ExecutionStage::PythonToolchain,
            Operation::PythonToolchain {
                version: workflow.config.shared.tools.python.clone().unwrap_or_else(|| "latest".into()),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

fn macos_shared_package_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    macos_cargo_update_workflow(workflow);
    macos_npm_update_workflow(workflow);
}

fn macos_cargo_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.packages.cargo == Some(true) {
        push_operation(&mut workflow.stages, ExecutionStage::CargoPackages, Operation::CargoPackageUpdate);
    }
}

fn macos_npm_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.packages.npm == Some(true) {
        workflow.managers.insert(ManagerBootstrap::Fnm);
        push_operation(&mut workflow.stages, ExecutionStage::NpmPackages, Operation::NpmPackageUpdate);
    }
}

fn macos_shared_font_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    macos_nerd_fonts_update_workflow(workflow);
}

fn macos_nerd_fonts_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.fonts == Some(true)
        && let Some(families) = workflow.config.shared.fonts.nerd.as_ref().filter(|families| !families.is_empty())
    {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::Fonts,
            Operation::UserNerdFonts { families: families.clone(), mode: NerdFontsMode::Update },
        );
    }
}

fn push_operation(stages: &mut [(ExecutionStage, Vec<Operation>)], stage: ExecutionStage, op: Operation) {
    stages.iter_mut().find(|(p, _)| *p == stage).expect("stage exists").1.push(op);
}

fn push_update_manager_bootstraps(
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &BTreeSet<ManagerBootstrap>,
) {
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
        };
        push_operation(stages, stage, operation);
    }
}

fn push_apply_manager_bootstraps(
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    managers: &BTreeSet<ManagerBootstrap>,
) {
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
        };
        push_operation(stages, stage, operation);
    }
}

fn flatten_stage_vec(stages: Vec<(ExecutionStage, Vec<Operation>)>) -> Vec<Operation> {
    stages.into_iter().flat_map(|(_, operations)| operations).collect()
}

fn plan_system_states(
    config: &Config,
    platform: &Platform,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    needs_apt_refresh: &mut bool,
) {
    let system = &config.os.linux.system;
    if let Some(state) = system.apt.as_ref().and_then(|apt| apt.unattended_upgrades) {
        push_operation(stages, ExecutionStage::SystemState, Operation::UnattendedUpgrades { enabled: enabled(state) });
        *needs_apt_refresh = true;
    }
    let Some(ubuntu) = &system.ubuntu else { return };
    let ubuntu_family = platform.upstream == "ubuntu";
    if let Some(state) = ubuntu.snap
        && ubuntu_family
    {
        *needs_apt_refresh = true;
        push_operation(stages, ExecutionStage::SystemState, Operation::UbuntuSnap { enabled: enabled(state) });
    }
    if ubuntu.codecs && ubuntu_family {
        *needs_apt_refresh = true;
        push_operation(
            stages,
            ExecutionStage::SystemState,
            Operation::AptPackages { packages: vec!["ubuntu-restricted-extras".into()] },
        );
    }
}

fn enabled(state: EnabledDisabled) -> bool {
    state == EnabledDisabled::Enabled
}

fn go_selector_main(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}

fn plan_repository(
    repository: &crate::config::Repository,
    platform: &Platform,
    identity: crate::config::PlatformIdentity,
) -> Result<Option<AptRepositoryOperation>> {
    let Some((key, source_url)) = select_distro_map(&repository.urls, identity.distro, identity.upstream) else {
        return Ok(None);
    };
    let suite = repository.suite.as_ref().map(|suite| {
        if suite == "system" {
            selected_repository_codename(key, platform, identity.distro).to_owned()
        } else {
            suite.clone()
        }
    });
    AptRepositoryOperation::new(
        repository.name.clone(),
        repository.key.clone(),
        source_url.clone(),
        platform.architecture,
        suite,
        repository.components.clone().unwrap_or_default(),
        repository.path.clone(),
        PathBuf::from(&repository.key_path),
    )
    .map(Some)
}

fn selected_repository_conflicts(
    repository: &crate::config::Repository,
    identity: crate::config::PlatformIdentity,
) -> Option<Vec<String>> {
    repository
        .conflicts
        .as_ref()
        .and_then(|conflicts| select_distro_map(conflicts, identity.distro, identity.upstream))
        .map(|(_, packages)| packages.clone())
}

fn plan_binary(binary: &crate::config::BinaryPackage, architecture: Architecture) -> Option<BinaryPackageOperation> {
    let source = match &binary.source {
        BinarySource::Github { repository, assets } => {
            let selector = assets.get(architecture)?;
            BinarySourceOperation::GithubLatest { repository: repository.clone(), selector: selector.to_owned() }
        }
        BinarySource::Url { urls } => BinarySourceOperation::Url { url: urls.get(architecture)?.to_owned() },
    };
    Some(BinaryPackageOperation::new(binary.name.clone(), binary.format, architecture, source))
}

fn linux_docker_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if let Some(docker) = &config.os.linux.integrations.docker {
        if docker.add_user_to_group == Some(true) {
            push_operation(stages, ExecutionStage::Integrations, Operation::DockerGroup);
        }
        if let Some(logging) = &docker.logging {
            push_operation(
                stages,
                ExecutionStage::Integrations,
                Operation::DockerLocalLog { max_size: logging.max_size.clone() },
            );
        }
    }
}

fn linux_virtualbox_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if config
        .os
        .linux
        .integrations
        .virtualbox
        .as_ref()
        .is_some_and(|virtualbox| virtualbox.add_user_to_group == Some(true))
    {
        push_operation(stages, ExecutionStage::Integrations, Operation::VirtualBoxGroup);
    }
}

fn linux_vscode_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
    if !config.shared.integrations.vscode.extensions.is_empty() {
        let extensions = config.shared.integrations.vscode.extensions.clone();
        push_operation(stages, ExecutionStage::Integrations, Operation::VsCodeExtensionSet { extensions });
    }
}

fn linux_desktop_settings_workflow(
    config: &Config,
    platform: &Platform,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
) {
    let Some(desktop) = config.os.linux.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return };
    let target = match platform.desktop.as_str() {
        "gnome" => DesktopEnvironment::Gnome,
        "cinnamon" => DesktopEnvironment::Cinnamon,
        _ => unreachable!("platform validation rejects unsupported desktop intent"),
    };
    prerequisites.extend(["dconf-cli", "libglib2.0-bin"]);
    linux_theme_workflow(desktop, target, stages);
    linux_terminal_workflow(desktop, target, stages);
    linux_idle_workflow(desktop, target, stages);
    linux_gnome_workflow(desktop, target, stages, prerequisites);
}

fn linux_theme_workflow(
    desktop: &crate::config::Desktop,
    target: DesktopEnvironment,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
) {
    if let Some(theme) = desktop.theme {
        push_operation(
            stages,
            ExecutionStage::Desktop,
            Operation::DesktopSetting {
                target,
                setting: DesktopSetting::Theme(match theme {
                    Theme::Light => DesktopTheme::Light,
                    Theme::Dark => DesktopTheme::Dark,
                }),
            },
        );
    }
}

fn linux_terminal_workflow(
    desktop: &crate::config::Desktop,
    target: DesktopEnvironment,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
) {
    if let Some(executable) = &desktop.terminal {
        push_operation(
            stages,
            ExecutionStage::Desktop,
            Operation::DesktopSetting { target, setting: DesktopSetting::Terminal(executable.clone()) },
        );
    }
}

fn linux_idle_workflow(
    desktop: &crate::config::Desktop,
    target: DesktopEnvironment,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
) {
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = &idle.timeout {
            push_operation(
                stages,
                ExecutionStage::Desktop,
                Operation::DesktopSetting { target, setting: DesktopSetting::IdleTimeoutSeconds(timeout.seconds()) },
            );
        }
        if let Some(enabled) = idle.dim {
            push_operation(
                stages,
                ExecutionStage::Desktop,
                Operation::DesktopSetting { target, setting: DesktopSetting::IdleDim(enabled) },
            );
        }
    }
}

fn linux_gnome_workflow(
    desktop: &crate::config::Desktop,
    target: DesktopEnvironment,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
) {
    if target == DesktopEnvironment::Gnome
        && let Some(gnome) = &desktop.gnome
    {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            prerequisites.insert("gnome-shell");
            push_operation(
                stages,
                ExecutionStage::Desktop,
                Operation::GnomeExtensions { extensions: extensions.clone() },
            );
        }
        if gnome.dock == Some(true) {
            prerequisites.insert("gnome-shell");
            push_operation(stages, ExecutionStage::Desktop, Operation::GnomeDock);
        }
        if gnome.rounded_corners == Some(true) {
            prerequisites.insert("gnome-shell");
            push_operation(stages, ExecutionStage::Desktop, Operation::GnomeRoundedCorners);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn macos_platform() -> Platform {
        Platform::from_release_parts(
            "macos".into(),
            "macos".into(),
            String::new(),
            String::new(),
            "none".into(),
            "aarch64",
        )
        .unwrap()
    }

    fn debian_platform() -> Platform {
        Platform::from_release_parts(
            "debian".into(),
            "debian".into(),
            "bookworm".into(),
            "bookworm".into(),
            "gnome".into(),
            "amd64",
        )
        .unwrap()
    }

    fn headless_ubuntu_platform() -> Platform {
        Platform::from_release_parts(
            "ubuntu".into(),
            "ubuntu".into(),
            "noble".into(),
            "noble".into(),
            "none".into(),
            "amd64",
        )
        .unwrap()
    }

    fn assert_no_empty_collection_operations(operations: &[Operation]) {
        for operation in operations {
            let populated = match operation {
                Operation::AptPackages { packages }
                | Operation::AptBootstrapPackages { packages }
                | Operation::CargoPackageSet { packages }
                | Operation::NpmPackageSet { packages } => !packages.is_empty(),
                Operation::AptRepositoryPackages { packages, .. } => !packages.is_empty(),
                Operation::FlatpakEnsureApps { refs } => !refs.is_empty(),
                Operation::NerdFonts { families, .. } | Operation::UserNerdFonts { families, .. } => {
                    !families.is_empty()
                }
                Operation::Dotfiles { packages, .. } => !packages.is_empty(),
                Operation::VsCodeExtensionSet { extensions } | Operation::GnomeExtensions { extensions } => {
                    !extensions.is_empty()
                }
                Operation::HomebrewPackages { formulae, casks } => !formulae.is_empty() || !casks.is_empty(),
                Operation::MacDefaults { settings } => !settings.is_empty(),
                _ => true,
            };
            assert!(populated, "empty synthetic operation: {operation:?}");
        }
    }

    #[test]
    fn full_example_parses_macos_configuration() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        assert_eq!(config.macos().homebrew.formulae[0], "cmake");
        assert_eq!(config.macos().desktop.appearance, Some(Theme::Dark));
    }

    #[test]
    fn macos_planner_emits_native_operations() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        config.os.macos.system.rosetta = Some(true);
        let operations = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();

        assert!(operations.contains(&Operation::HomebrewBootstrap));
        assert!(operations.contains(&Operation::MacEnsureAdmin));
        assert!(operations.contains(&Operation::XcodeCommandLineTools));
        assert!(operations.contains(&Operation::Rosetta));
        assert!(operations.iter().any(
            |operation| matches!(operation, Operation::HomebrewPackages { formulae, .. } if formulae.iter().any(|formula| formula == "stow"))
        ));
        assert_eq!(operations.iter().filter(|operation| **operation == Operation::FnmBootstrap).count(), 1);
        assert!(operations.iter().any(|operation| matches!(operation, Operation::Dotfiles { replace: false, .. })));
        assert!(
            operations
                .iter()
                .any(|operation| matches!(operation, Operation::MacDefaults { settings } if settings.len() == 8))
        );

        let dotfiles = plan_standalone_dotfiles(&config, &macos_platform(), Path::new("/tmp/dotfiles"), true).unwrap();
        assert!(matches!(dotfiles.as_slice(), [Operation::Dotfiles { replace: true, .. }]));
    }

    #[test]
    fn macos_apply_workflow_preserves_capability_order() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        config.os.macos.system.rosetta = Some(true);
        let operations = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();
        let position = |predicate: fn(&Operation) -> bool| operations.iter().position(predicate).unwrap();

        let admin = position(|operation| matches!(operation, Operation::MacEnsureAdmin));
        let xcode = position(|operation| matches!(operation, Operation::XcodeCommandLineTools));
        let rosetta = position(|operation| matches!(operation, Operation::Rosetta));
        let homebrew_bootstrap = position(|operation| matches!(operation, Operation::HomebrewBootstrap));
        let homebrew_packages = position(|operation| matches!(operation, Operation::HomebrewPackages { .. }));
        let rustup = position(|operation| matches!(operation, Operation::RustupBootstrap));
        let rust = position(|operation| matches!(operation, Operation::RustToolchain { .. }));
        let fnm = position(|operation| matches!(operation, Operation::FnmBootstrap));
        let node = position(|operation| matches!(operation, Operation::NodeToolchain { .. }));
        let uv = position(|operation| matches!(operation, Operation::UvBootstrap));
        let python = position(|operation| matches!(operation, Operation::PythonToolchain { .. }));
        let cargo_binstall = position(|operation| matches!(operation, Operation::CargoBinstallBootstrap));
        let cargo = position(|operation| matches!(operation, Operation::CargoPackageSet { .. }));
        let dotfiles = position(|operation| matches!(operation, Operation::Dotfiles { .. }));
        let vscode = position(|operation| matches!(operation, Operation::VsCodeExtensionSet { .. }));
        let desktop = position(|operation| matches!(operation, Operation::MacDefaults { .. }));

        assert!(admin < xcode);
        assert!(xcode < rosetta);
        assert!(rosetta < homebrew_bootstrap);
        assert!(homebrew_bootstrap < homebrew_packages);
        assert!(homebrew_packages < rustup);
        assert!(rustup < rust);
        let go = position(|operation| matches!(operation, Operation::GoToolchain { .. }));
        assert!(rust < go);
        assert!(go < fnm);
        assert!(fnm < node);
        assert!(node < uv);
        assert!(uv < python);
        assert!(python < cargo_binstall);
        assert!(cargo_binstall < cargo);
        assert!(cargo < dotfiles);
        assert!(dotfiles < vscode);
        assert!(dotfiles < desktop);
    }

    #[test]
    fn linux_apply_workflow_preserves_capability_order() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let operations = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();
        let position = |predicate: fn(&Operation) -> bool| operations.iter().position(predicate).unwrap();
        let last_position = |predicate: fn(&Operation) -> bool| operations.iter().rposition(predicate).unwrap();

        let admin = position(|operation| matches!(operation, Operation::EnsureAdmin));
        let platform = position(|operation| matches!(operation, Operation::EnsureDebianAptComponents { .. }));
        let refresh = position(|operation| matches!(operation, Operation::AptMetadataRefresh));
        let state = position(|operation| matches!(operation, Operation::UnattendedUpgrades { .. }));
        let prerequisites = position(|operation| matches!(operation, Operation::AptBootstrapPackages { .. }));
        let direct_packages =
            position(|operation| matches!(operation, Operation::AptPackages { packages } if packages.len() > 1));
        let repository = position(|operation| matches!(operation, Operation::AptRepository(_)));
        let repository_refresh = last_position(|operation| matches!(operation, Operation::AptMetadataRefresh));
        let repository_packages = position(|operation| matches!(operation, Operation::AptRepositoryPackages { .. }));
        let flatpak_bootstrap = position(|operation| matches!(operation, Operation::FlatpakEnsureFlathub));
        let flatpak_packages = position(|operation| matches!(operation, Operation::FlatpakEnsureApps { .. }));
        let rustup = position(|operation| matches!(operation, Operation::RustupBootstrap));
        let rust = position(|operation| matches!(operation, Operation::RustToolchain { .. }));
        let go = position(|operation| matches!(operation, Operation::GoToolchain { .. }));
        let fnm = position(|operation| matches!(operation, Operation::FnmBootstrap));
        let node = position(|operation| matches!(operation, Operation::NodeToolchain { .. }));
        let uv = position(|operation| matches!(operation, Operation::UvBootstrap));
        let python = position(|operation| matches!(operation, Operation::PythonToolchain { .. }));
        let cargo_binstall = position(|operation| matches!(operation, Operation::CargoBinstallBootstrap));
        let cargo = position(|operation| matches!(operation, Operation::CargoPackageSet { .. }));
        let npm = position(|operation| matches!(operation, Operation::NpmPackageSet { .. }));
        let first_binary = position(|operation| matches!(operation, Operation::BinaryPackage(_)));
        let appimaged = position(|operation| matches!(operation, Operation::Appimaged { .. }));
        let last_binary = last_position(|operation| matches!(operation, Operation::BinaryPackage(_)));
        let fonts = position(|operation| matches!(operation, Operation::NerdFonts { .. }));
        let dotfiles = position(|operation| matches!(operation, Operation::Dotfiles { .. }));
        let docker = position(|operation| matches!(operation, Operation::DockerGroup));
        let virtualbox = position(|operation| matches!(operation, Operation::VirtualBoxGroup));
        let vscode = position(|operation| matches!(operation, Operation::VsCodeExtensionSet { .. }));
        let desktop = position(|operation| matches!(operation, Operation::DesktopSetting { .. }));

        assert!(admin < platform);
        assert!(platform < refresh);
        assert!(refresh < state);
        assert!(state < prerequisites);
        assert!(prerequisites < direct_packages);
        assert!(direct_packages < repository);
        assert!(repository < repository_refresh);
        assert!(repository_refresh < repository_packages);
        assert!(repository_packages < flatpak_bootstrap);
        assert!(flatpak_bootstrap < flatpak_packages);
        assert!(flatpak_packages < rustup);
        assert!(rustup < rust);
        assert!(rust < go);
        assert!(go < fnm);
        assert!(fnm < node);
        assert!(node < uv);
        assert!(uv < python);
        assert!(python < cargo_binstall);
        assert!(cargo_binstall < cargo);
        assert!(cargo < npm);
        assert!(npm < first_binary);
        assert!(first_binary < appimaged);
        assert!(appimaged < last_binary);
        assert!(last_binary < fonts);
        assert!(fonts < dotfiles);
        assert!(dotfiles < docker);
        assert!(docker < virtualbox);
        assert!(virtualbox < vscode);
        assert!(vscode < desktop);
    }

    #[test]
    fn apply_derives_prerequisites_and_deduplicates_bootstraps() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let linux = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();
        let macos = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();
        let prerequisites = linux
            .iter()
            .find_map(|operation| match operation {
                Operation::AptBootstrapPackages { packages } => Some(packages),
                _ => None,
            })
            .unwrap();

        for package in ["ca-certificates", "curl", "flatpak", "fontconfig", "gnupg", "stow", "tar", "unzip", "xz-utils"]
        {
            assert!(prerequisites.iter().any(|candidate| candidate == package));
        }
        for operations in [&linux, &macos] {
            assert_eq!(
                operations.iter().filter(|operation| matches!(operation, Operation::RustupBootstrap)).count(),
                1
            );
            assert_eq!(operations.iter().filter(|operation| matches!(operation, Operation::FnmBootstrap)).count(), 1);
            assert_eq!(
                operations.iter().filter(|operation| matches!(operation, Operation::CargoBinstallBootstrap)).count(),
                1
            );
            assert_no_empty_collection_operations(operations);
        }
    }

    #[test]
    fn yaml_mapping_order_does_not_change_apply_order() {
        let source = include_str!("../configs/full.yaml");
        let shared = source.find("\nshared:").unwrap();
        let os = source.find("\nos:\n").unwrap();
        let reordered = format!("{}{}{}", &source[..shared], &source[os..], &source[shared..os]);
        let original = Config::parse(source).unwrap();
        let reordered = Config::parse(&reordered).unwrap();

        assert_eq!(
            plan_apply(&original, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap(),
            plan_apply(&reordered, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap()
        );
        assert_eq!(
            plan_apply(&original, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap(),
            plan_apply(&reordered, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap()
        );
    }

    #[test]
    fn update_workflows_deduplicate_non_apt_manager_bootstraps() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        for operations in
            [plan_update(&config, &debian_platform()).unwrap(), plan_update(&config, &macos_platform()).unwrap()]
        {
            assert_eq!(operations.iter().filter(|operation| **operation == Operation::RustupBootstrap).count(), 1);
            assert_eq!(operations.iter().filter(|operation| **operation == Operation::FnmBootstrap).count(), 1);
            assert_eq!(operations.iter().filter(|operation| **operation == Operation::UvBootstrap).count(), 1);
        }
    }

    #[test]
    fn debian_apply_always_ensures_required_apt_components() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let operations = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();
        assert!(operations.contains(&Operation::EnsureDebianAptComponents { release: "bookworm".into() }));
    }

    #[test]
    fn cli_preset_plans_on_a_headless_host() {
        let config = Config::parse(include_str!("../configs/cli.yaml")).unwrap();
        let operations = plan_apply(&config, &headless_ubuntu_platform(), Path::new("/tmp/dotfiles")).unwrap();
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::VsCodeExtensionSet { .. })));
    }

    #[test]
    fn macos_planner_skips_empty_portable_package_and_font_sets() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        config.shared.packages.cargo = Some(Vec::new());
        config.shared.packages.npm = Some(Vec::new());
        config.shared.fonts.nerd = Some(Vec::new());
        config.os.macos.system.rosetta = Some(false);

        let apply = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();
        assert!(!apply.iter().any(|operation| matches!(operation, Operation::CargoPackageSet { .. })));
        assert!(!apply.iter().any(|operation| matches!(operation, Operation::NpmPackageSet { .. })));
        assert!(!apply.iter().any(|operation| matches!(operation, Operation::Rosetta)));
        assert_no_empty_collection_operations(&apply);

        let update = plan_update(&config, &macos_platform()).unwrap();
        assert!(!update.iter().any(|operation| matches!(operation, Operation::UserNerdFonts { .. })));
    }

    #[test]
    fn linux_planner_skips_empty_package_binary_and_font_sets() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let packages = &mut config.os.linux.packages;
        packages.apt.as_mut().unwrap().install = Some(Vec::new());
        packages.apt.as_mut().unwrap().repositories = Some(Vec::new());
        packages.flatpak = Some(Vec::new());
        packages.binaries = Some(Vec::new());
        config.shared.packages.cargo = Some(Vec::new());
        config.shared.packages.npm = Some(Vec::new());
        config.shared.fonts.nerd = Some(Vec::new());
        config.os.linux.system.ensure_admin = Some(false);
        config.os.linux.integrations.docker.as_mut().unwrap().add_user_to_group = Some(false);
        config.os.linux.integrations.virtualbox.as_mut().unwrap().add_user_to_group = Some(false);

        let operations = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::AptRepository(_))));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::AptRepositoryPackages { .. })));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::FlatpakEnsureApps { .. })));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::BinaryPackage(_))));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::CargoPackageSet { .. })));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::NpmPackageSet { .. })));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::NerdFonts { .. })));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::EnsureAdmin)));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::DockerGroup)));
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::VirtualBoxGroup)));
        assert_no_empty_collection_operations(&operations);
    }

    #[test]
    fn linux_update_workflow_preserves_exact_capability_order() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let operations = plan_update(&config, &debian_platform()).unwrap();

        assert_eq!(
            operations,
            vec![
                Operation::AptMetadataRefresh,
                Operation::AptUpgrade { policy: AptUpgradePolicy::Full },
                Operation::AptBootstrapPackages {
                    packages: vec![
                        "ca-certificates".into(),
                        "curl".into(),
                        "flatpak".into(),
                        "fontconfig".into(),
                        "tar".into(),
                        "unzip".into(),
                        "xz-utils".into(),
                    ],
                },
                Operation::FlatpakUpdateApps,
                Operation::RustupBootstrap,
                Operation::RustToolchain { selector: Some("stable".into()), mode: ToolchainMode::ConvergeLatest },
                Operation::GoToolchain {
                    selector: GoToolchainSelector::Latest,
                    architecture: Architecture::Amd64,
                    mode: ToolchainMode::ConvergeLatest,
                },
                Operation::FnmBootstrap,
                Operation::NodeToolchain { selector: "lts".into(), mode: ToolchainMode::ConvergeLatest },
                Operation::UvBootstrap,
                Operation::PythonToolchain { version: "3.13".into(), mode: ToolchainMode::ConvergeLatest },
                Operation::CargoPackageUpdate,
                Operation::NpmPackageUpdate,
                Operation::NerdFonts { families: vec!["GeistMono".into()], mode: NerdFontsMode::Update },
            ]
        );
    }

    #[test]
    fn macos_update_workflow_preserves_exact_capability_order() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let operations = plan_update(&config, &macos_platform()).unwrap();

        assert_eq!(
            operations,
            vec![
                Operation::HomebrewUpdate { formulae: true, casks: true },
                Operation::RustupBootstrap,
                Operation::RustToolchain { selector: Some("stable".into()), mode: ToolchainMode::ConvergeLatest },
                Operation::GoToolchain {
                    selector: GoToolchainSelector::Latest,
                    architecture: Architecture::DarwinArm64,
                    mode: ToolchainMode::ConvergeLatest,
                },
                Operation::FnmBootstrap,
                Operation::NodeToolchain { selector: "lts".into(), mode: ToolchainMode::ConvergeLatest },
                Operation::UvBootstrap,
                Operation::PythonToolchain { version: "3.13".into(), mode: ToolchainMode::ConvergeLatest },
                Operation::CargoPackageUpdate,
                Operation::NpmPackageUpdate,
                Operation::UserNerdFonts { families: vec!["GeistMono".into()], mode: NerdFontsMode::Update },
            ]
        );
    }

    #[test]
    fn apt_update_plan_contains_only_refresh_and_selected_upgrade() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        config.os.linux.updates.as_mut().unwrap().apt = Some(AptUpdate::Full);
        config.os.linux.updates.as_mut().unwrap().flatpak = Some(false);
        config.shared.updates.tools.rust = Some(false);
        config.shared.updates.tools.go = Some(false);
        config.shared.updates.tools.node = Some(false);
        config.shared.updates.tools.python = Some(false);
        config.shared.updates.packages.cargo = Some(false);
        config.shared.updates.packages.npm = Some(false);
        config.shared.updates.fonts = Some(false);

        assert_eq!(
            plan_update(&config, &debian_platform()).unwrap(),
            [Operation::AptMetadataRefresh, Operation::AptUpgrade { policy: AptUpgradePolicy::Full },]
        );
    }

    #[test]
    fn absent_and_false_update_controls_are_no_ops() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        config.os.linux.updates = None;
        config.os.macos.updates.homebrew.formulae = Some(false);
        config.os.macos.updates.homebrew.casks = Some(false);
        config.shared.updates.tools.rust = Some(false);
        config.shared.updates.tools.go = Some(false);
        config.shared.updates.tools.node = Some(false);
        config.shared.updates.tools.python = Some(false);
        config.shared.updates.packages.cargo = Some(false);
        config.shared.updates.packages.npm = Some(false);
        config.shared.updates.fonts = Some(false);

        assert!(plan_update(&config, &debian_platform()).unwrap().is_empty());
        assert!(plan_update(&config, &macos_platform()).unwrap().is_empty());
    }
}
