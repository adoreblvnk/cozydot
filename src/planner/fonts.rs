use super::*;

pub(super) fn linux_shared_font_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
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

pub(super) fn linux_shared_font_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
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

pub(super) fn macos_shared_font_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
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

pub(super) fn macos_shared_font_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
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
