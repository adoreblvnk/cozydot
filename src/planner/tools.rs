use super::*;

pub(super) fn linux_shared_tools_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
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

pub(super) fn linux_shared_package_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
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

pub(super) fn linux_shared_tool_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
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

pub(super) fn linux_shared_package_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
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

pub(super) fn macos_shared_tools_workflow(
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

pub(super) fn macos_shared_package_workflow(
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

pub(super) fn macos_shared_tool_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
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

pub(super) fn macos_shared_package_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
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

fn go_selector_main(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}
