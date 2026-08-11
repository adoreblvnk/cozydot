use super::*;

pub(super) fn linux_desktop_workflow(workflow: &mut LinuxApplyWorkflow<'_>) {
    linux_desktop_settings_workflow(
        workflow.config,
        workflow.platform,
        &mut workflow.stages,
        &mut workflow.prerequisites,
    );
}

pub(super) fn macos_desktop_workflow(config: &Config, stages: &mut [(ExecutionStage, Vec<Operation>)]) {
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
