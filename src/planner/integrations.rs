use super::*;

pub(super) fn linux_integration_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    linux_docker_workflow(workflow.config, &mut workflow.stages);
    linux_virtualbox_workflow(workflow.config, &mut workflow.stages);
    vscode_workflow(workflow.config, &mut workflow.stages);
}

pub(super) fn macos_integration_workflow(config: &Config, stages: &mut Stages) {
    vscode_workflow(config, stages);
}

fn vscode_workflow(config: &Config, stages: &mut Stages) {
    if !config.shared.integrations.vscode.extensions.is_empty() {
        push_operation(
            stages,
            ExecutionStage::Integrations,
            Operation::VsCodeExtensionSet { extensions: config.shared.integrations.vscode.extensions.clone() },
        );
    }
}

fn linux_docker_workflow(config: &Config, stages: &mut Stages) {
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

fn linux_virtualbox_workflow(config: &Config, stages: &mut Stages) {
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
