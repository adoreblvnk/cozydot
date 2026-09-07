use crate::{
    config::{MacosDesktop, Theme},
    operations::host,
    paths,
};
use anyhow::{Context, Result};
use std::{fs, path::PathBuf};

pub(crate) fn write_defaults(theme: Option<Theme>, desktop: Option<&MacosDesktop>) -> Result<()> {
    let mut restart_dock = false;
    let mut restart_finder = false;
    if let Some(theme) = theme {
        if theme == Theme::Dark {
            host::run("macOS appearance", "defaults", ["write", "-g", "AppleInterfaceStyle", "-string", "Dark"])?;
        } else {
            // ignore deletion errors because a missing preference already means light mode
            host::output("defaults", ["delete", "-g", "AppleInterfaceStyle"]).ok();
        }
    }
    if let Some(desktop) = desktop {
        if let Some(dialogs) = &desktop.dialogs {
            if let Some(value) = dialogs.expand_save_panel {
                write_bool("-g", "NSNavPanelExpandedStateForSaveMode", value)?;
            }
            if let Some(value) = dialogs.save_to_cloud {
                write_bool("-g", "NSDocumentSaveNewDocumentsToCloud", value)?;
            }
        }
        if let Some(dock) = &desktop.dock {
            if let Some(value) = dock.autohide {
                write_bool("com.apple.dock", "autohide", value)?;
                restart_dock = true;
            }
            if let Some(value) = dock.mru_spaces {
                write_bool("com.apple.dock", "mru-spaces", value)?;
                restart_dock = true;
            }
            if let Some(value) = dock.show_recent_applications {
                write_bool("com.apple.dock", "show-recents", value)?;
                restart_dock = true;
            }
        }
        if let Some(finder) = &desktop.finder {
            if let Some(enabled) = finder.ds_store_on_external {
                write_bool("com.apple.desktopservices", "DSDontWriteNetworkStores", !enabled)?;
                write_bool("com.apple.desktopservices", "DSDontWriteUSBStores", !enabled)?;
                restart_finder = true;
            }
            if let Some(scope) = finder.search_scope {
                write_string("com.apple.finder", "FXDefaultSearchScope", scope.as_str())?;
                restart_finder = true;
            }
            if let Some(value) = finder.show_filename_extensions {
                write_bool("-g", "AppleShowAllExtensions", value)?;
                restart_finder = true;
            }
            if let Some(value) = finder.show_hidden_files {
                write_bool("com.apple.finder", "AppleShowAllFiles", value)?;
                restart_finder = true;
            }
            if let Some(value) = finder.show_path_bar {
                write_bool("com.apple.finder", "ShowPathbar", value)?;
                restart_finder = true;
            }
            if let Some(value) = finder.show_status_bar {
                write_bool("com.apple.finder", "ShowStatusBar", value)?;
                restart_finder = true;
            }
            if let Some(value) = finder.sort_folders_first {
                write_bool("com.apple.finder", "_FXSortFoldersFirst", value)?;
                restart_finder = true;
            }
            if let Some(value) = finder.warn_on_extension_change {
                write_bool("com.apple.finder", "FXEnableExtensionChangeWarning", value)?;
                restart_finder = true;
            }
        }
        if let Some(keyboard) = &desktop.keyboard {
            if let Some(value) = keyboard.auto_substitutions {
                write_bool("-g", "NSAutomaticCapitalizationEnabled", value)?;
                write_bool("-g", "NSAutomaticDashSubstitutionEnabled", value)?;
                write_bool("-g", "NSAutomaticPeriodSubstitutionEnabled", value)?;
                write_bool("-g", "NSAutomaticQuoteSubstitutionEnabled", value)?;
                write_bool("-g", "NSAutomaticSpellingCorrectionEnabled", value)?;
            }
            if let Some(value) = keyboard.keyboard_navigation {
                write_int("-g", "AppleKeyboardUIMode", if value { 2 } else { 0 })?;
            }
            if let Some(value) = keyboard.initial_key_repeat {
                write_int("-g", "InitialKeyRepeat", value)?;
            }
            if let Some(value) = keyboard.key_repeat {
                write_int("-g", "KeyRepeat", value)?;
            }
            if let Some(value) = keyboard.press_and_hold {
                write_bool("-g", "ApplePressAndHoldEnabled", value)?;
            }
        }
        if let Some(location) = desktop.screenshots.as_ref().and_then(|s| s.location.as_deref()) {
            let path = if let Some(stripped) = location.strip_prefix("~/") {
                paths::home()?.join(stripped)
            } else {
                PathBuf::from(location)
            };
            fs::create_dir_all(&path)?;
            let path_str = path.to_str().context("screenshot location is not valid UTF-8")?;
            write_string("com.apple.screencapture", "location", path_str)?;
        }
        if let Some(value) = desktop.trackpad.as_ref().and_then(|trackpad| trackpad.tap_to_click) {
            write_bool("com.apple.AppleMultitouchTrackpad", "Clicking", value)?;
        }
    }
    if restart_dock {
        // ignore restart errors when Dock isn't running
        host::run("Dock restart", "killall", ["Dock"]).ok();
    }
    if restart_finder {
        host::run("Finder restart", "killall", ["Finder"]).ok();
    }
    Ok(())
}

fn write_bool(domain: &str, key: &str, value: bool) -> Result<()> {
    host::run("macOS defaults", "defaults", ["write", domain, key, "-bool", if value { "true" } else { "false" }])?;
    Ok(())
}

fn write_int(domain: &str, key: &str, value: i32) -> Result<()> {
    host::run("macOS defaults", "defaults", ["write", domain, key, "-int", &value.to_string()])?;
    Ok(())
}

fn write_string(domain: &str, key: &str, value: &str) -> Result<()> {
    host::run("macOS defaults", "defaults", ["write", domain, key, "-string", value])?;
    Ok(())
}
