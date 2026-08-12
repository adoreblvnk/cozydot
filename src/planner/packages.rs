use super::*;

pub(super) fn linux_package_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
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

pub(super) fn linux_binary_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
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

pub(super) fn linux_apt_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    let Some(policy) = workflow.config.os.linux.updates.as_ref().and_then(|updates| updates.apt) else {
        return;
    };
    push_operation(&mut workflow.stages, ExecutionStage::SystemMetadataRefresh, Operation::AptMetadataRefresh);
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

pub(super) fn linux_flatpak_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.os.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        workflow.prerequisites.insert("flatpak");
        push_operation(&mut workflow.stages, ExecutionStage::ApplicationPackages, Operation::FlatpakUpdateApps);
    }
}

pub(super) fn macos_homebrew_workflow(config: &Config, stages: &mut Stages) {
    let mac = config.macos();
    let dotfiles = config.shared.dotfiles.packages.iter().chain(mac.dotfiles.packages.iter()).next().is_some();
    let has_packages = dotfiles || !mac.homebrew.formulae.is_empty() || !mac.homebrew.casks.is_empty();
    macos_homebrew_availability_workflow(has_packages, stages);
    let formulae = macos_homebrew_formulae_workflow(config, dotfiles);
    let casks = macos_homebrew_casks_workflow(config);
    macos_homebrew_packages_operation(formulae, casks, stages);
}

fn macos_homebrew_availability_workflow(has_packages: bool, stages: &mut Stages) {
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

fn macos_homebrew_packages_operation(formulae: Vec<String>, casks: Vec<String>, stages: &mut Stages) {
    if !formulae.is_empty() || !casks.is_empty() {
        push_operation(stages, ExecutionStage::SystemPackages, Operation::HomebrewPackages { formulae, casks });
    }
}

pub(super) fn macos_homebrew_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    let formulae = workflow.config.macos().updates.homebrew.formulae == Some(true);
    let casks = workflow.config.macos().updates.homebrew.casks == Some(true);
    if formulae || casks {
        push_operation(
            &mut workflow.stages,
            ExecutionStage::SystemUpdates,
            Operation::HomebrewUpdate { formulae, casks },
        );
    }
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
