use super::*;

const LINUX_FONT_PREREQUISITES: [&str; 5] = ["ca-certificates", "curl", "tar", "xz-utils", "fontconfig"];

pub(super) fn linux_shared_font_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    if let Some(operation) = nerd_fonts_operation(workflow.config, NerdFontsMode::EnsurePresent, false) {
        workflow.prerequisites.extend(LINUX_FONT_PREREQUISITES);
        push_operation(&mut workflow.stages, ExecutionStage::Fonts, operation);
    }
}

pub(super) fn macos_shared_font_workflow(config: &Config, stages: &mut Stages) {
    if let Some(operation) = nerd_fonts_operation(config, NerdFontsMode::EnsurePresent, true) {
        push_operation(stages, ExecutionStage::Fonts, operation);
    }
}

pub(super) fn linux_shared_font_update_workflow(workflow: &mut LinuxUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.fonts == Some(true)
        && let Some(operation) = nerd_fonts_operation(workflow.config, NerdFontsMode::Update, false)
    {
        workflow.prerequisites.extend(LINUX_FONT_PREREQUISITES);
        push_operation(&mut workflow.stages, ExecutionStage::Fonts, operation);
    }
}

pub(super) fn macos_shared_font_update_workflow(workflow: &mut MacosUpdateWorkflow<'_>) {
    if workflow.config.shared.updates.fonts == Some(true)
        && let Some(operation) = nerd_fonts_operation(workflow.config, NerdFontsMode::Update, true)
    {
        push_operation(&mut workflow.stages, ExecutionStage::Fonts, operation);
    }
}

fn nerd_fonts_operation(config: &Config, mode: NerdFontsMode, user: bool) -> Option<Operation> {
    let families = config.shared.fonts.nerd.as_ref().filter(|families| !families.is_empty())?.clone();
    Some(if user { Operation::UserNerdFonts { families, mode } } else { Operation::NerdFonts { families, mode } })
}
