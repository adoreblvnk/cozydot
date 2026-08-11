use super::*;

pub(super) fn linux_system_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
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

pub(super) fn macos_system_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
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
