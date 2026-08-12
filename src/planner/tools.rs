use super::*;

pub(super) fn linux_shared_tools_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    let tools = &workflow.config.shared.tools;
    if tools.rust.is_some() || tools.node.is_some() || tools.python.is_some() {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
    }
    if tools.go.is_some() {
        workflow.prerequisites.extend(["ca-certificates", "curl", "tar"]);
    }
    shared_tools_workflow(
        workflow.config,
        workflow.platform.architecture,
        &mut workflow.stages,
        &mut workflow.managers,
    );
}

pub(super) fn macos_shared_tools_workflow(
    config: &Config,
    architecture: Architecture,
    stages: &mut Stages,
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    shared_tools_workflow(config, architecture, stages, managers);
}

fn shared_tools_workflow(
    config: &Config,
    architecture: Architecture,
    stages: &mut Stages,
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    if let Some(selector) = config.shared.tools.rust.as_deref() {
        managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            stages,
            ExecutionStage::RustToolchain,
            Operation::RustToolchain { selector: Some(selector.to_owned()), mode: ToolchainMode::EnsurePresent },
        );
    }
    if let Some(selector) = config.shared.tools.go.as_deref() {
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
    if let Some(selector) = config.shared.tools.node.as_deref() {
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            stages,
            ExecutionStage::NodeToolchain,
            Operation::NodeToolchain { selector: selector.to_owned(), mode: ToolchainMode::EnsurePresent },
        );
    }
    if let Some(version) = &config.shared.tools.python {
        managers.insert(ManagerBootstrap::Uv);
        push_operation(
            stages,
            ExecutionStage::PythonToolchain,
            Operation::PythonToolchain { version: version.clone(), mode: ToolchainMode::EnsurePresent },
        );
    }
}

pub(super) fn linux_shared_package_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    let packages = &workflow.config.shared.packages;
    let cargo = packages.cargo.as_ref().is_some_and(|values| !values.is_empty());
    let npm = packages.npm.as_ref().is_some_and(|values| !values.is_empty());
    if cargo || npm {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
    }
    if cargo {
        workflow.managers.extend([ManagerBootstrap::Rustup, ManagerBootstrap::CargoBinstall]);
    }
    if npm {
        workflow.managers.insert(ManagerBootstrap::Fnm);
    }
    shared_package_workflow(workflow.config, &mut workflow.stages);
}

pub(super) fn macos_shared_package_workflow(
    config: &Config,
    stages: &mut Stages,
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    if config.shared.packages.cargo.as_ref().is_some_and(|packages| !packages.is_empty()) {
        managers.extend([ManagerBootstrap::Rustup, ManagerBootstrap::CargoBinstall]);
    }
    shared_package_workflow(config, stages);
}

fn shared_package_workflow(config: &Config, stages: &mut Stages) {
    if let Some(packages) = config.shared.packages.cargo.as_ref().filter(|packages| !packages.is_empty()) {
        push_operation(
            stages,
            ExecutionStage::CargoPackages,
            Operation::CargoPackageSet { packages: packages.clone() },
        );
    }
    if let Some(packages) = config.shared.packages.npm.as_ref().filter(|packages| !packages.is_empty()) {
        push_operation(stages, ExecutionStage::NpmPackages, Operation::NpmPackageSet { packages: packages.clone() });
    }
}

pub(super) fn linux_shared_tool_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    let updates = &workflow.config.shared.updates.tools;
    if updates.rust == Some(true) || updates.node == Some(true) || updates.python == Some(true) {
        workflow.prerequisites.extend(["ca-certificates", "curl"]);
    }
    if updates.go == Some(true) {
        workflow.prerequisites.extend(["ca-certificates", "curl", "tar"]);
    }
    shared_tool_update_workflow(
        workflow.config,
        workflow.platform.architecture,
        "3",
        &mut workflow.stages,
        &mut workflow.managers,
    );
}

pub(super) fn macos_shared_tool_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    shared_tool_update_workflow(
        workflow.config,
        workflow.architecture,
        "latest",
        &mut workflow.stages,
        &mut workflow.managers,
    );
}

fn shared_tool_update_workflow(
    config: &Config,
    architecture: Architecture,
    python_default: &str,
    stages: &mut Stages,
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    let updates = &config.shared.updates.tools;
    if updates.rust == Some(true) {
        managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            stages,
            ExecutionStage::RustToolchain,
            Operation::RustToolchain {
                selector: config.shared.tools.rust.clone(),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
    if updates.go == Some(true) {
        push_operation(
            stages,
            ExecutionStage::GoToolchain,
            Operation::GoToolchain {
                selector: go_selector_main(config.shared.tools.go.as_deref().unwrap_or("latest")),
                architecture,
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
    if updates.node == Some(true) {
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            stages,
            ExecutionStage::NodeToolchain,
            Operation::NodeToolchain {
                selector: config.shared.tools.node.clone().unwrap_or_else(|| "latest".to_owned()),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
    if updates.python == Some(true) {
        managers.insert(ManagerBootstrap::Uv);
        push_operation(
            stages,
            ExecutionStage::PythonToolchain,
            Operation::PythonToolchain {
                version: config.shared.tools.python.clone().unwrap_or_else(|| python_default.to_owned()),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
}

pub(super) fn linux_shared_package_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    shared_package_update_workflow(workflow.config, &mut workflow.stages);
}

pub(super) fn macos_shared_package_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.packages.npm == Some(true) {
        workflow.managers.insert(ManagerBootstrap::Fnm);
    }
    shared_package_update_workflow(workflow.config, &mut workflow.stages);
}

fn shared_package_update_workflow(config: &Config, stages: &mut Stages) {
    if config.shared.updates.packages.cargo == Some(true) {
        push_operation(stages, ExecutionStage::CargoPackages, Operation::CargoPackageUpdate);
    }
    if config.shared.updates.packages.npm == Some(true) {
        push_operation(stages, ExecutionStage::NpmPackages, Operation::NpmPackageUpdate);
    }
}

fn go_selector_main(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}
