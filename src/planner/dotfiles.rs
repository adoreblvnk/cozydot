use super::*;

pub(super) fn operation(config: &Config, platform: &Platform, root: &Path, replace: bool) -> Result<Option<Operation>> {
    let platform_packages =
        if platform.is_macos() { &config.os.macos.dotfiles.packages } else { &config.os.linux.dotfiles.packages };
    operation_for_packages(config, platform_packages, root, replace)
}

pub(super) fn linux_dotfiles_workflow(workflow: &mut LinuxApplyWorkflow<'_>) -> Result<()> {
    if let Some(operation) = operation_for_packages(
        workflow.config,
        &workflow.config.os.linux.dotfiles.packages,
        workflow.dotfiles_root,
        false,
    )? {
        workflow.prerequisites.insert("stow");
        push_operation(&mut workflow.stages, ExecutionStage::Dotfiles, operation);
    }
    Ok(())
}

pub(super) fn macos_dotfiles_workflow(
    config: &Config,
    dotfiles_root: &Path,
    stages: &mut [(ExecutionStage, Vec<Operation>)],
) -> Result<()> {
    if let Some(operation) = operation_for_packages(config, &config.macos().dotfiles.packages, dotfiles_root, false)? {
        push_operation(stages, ExecutionStage::Dotfiles, operation);
    }
    Ok(())
}

fn operation_for_packages(
    config: &Config,
    platform_packages: &[String],
    root: &Path,
    replace: bool,
) -> Result<Option<Operation>> {
    let packages = config.shared.dotfiles.packages.iter().chain(platform_packages).cloned().collect::<Vec<_>>();
    if packages.is_empty() {
        return Ok(None);
    }
    if root.as_os_str().is_empty() {
        anyhow::bail!("dotfiles root must not be empty");
    }
    Ok(Some(Operation::Dotfiles { root: root.to_path_buf(), packages, replace }))
}
