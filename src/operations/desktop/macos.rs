use crate::{
    config::{MacosDesktop, Theme},
    operations::host,
};
use anyhow::Result;

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
        if let Some(dock) = &desktop.dock {
            if let Some(value) = dock.autohide {
                write_bool("com.apple.dock", "autohide", value)?;
                restart_dock = true;
            }
            if let Some(value) = dock.show_recent_applications {
                write_bool("com.apple.dock", "show-recents", value)?;
                restart_dock = true;
            }
        }
        if let Some(finder) = &desktop.finder {
            if let Some(value) = finder.show_filename_extensions {
                write_bool("NSGlobalDomain", "AppleShowAllExtensions", value)?;
                restart_finder = true;
            }
            if let Some(value) = finder.show_hidden_files {
                write_bool("com.apple.finder", "AppleShowAllFiles", value)?;
                restart_finder = true;
            }
        }
        if let Some(keyboard) = &desktop.keyboard {
            if let Some(value) = keyboard.key_repeat {
                write_int("NSGlobalDomain", "KeyRepeat", value)?;
            }
            if let Some(value) = keyboard.initial_key_repeat {
                write_int("NSGlobalDomain", "InitialKeyRepeat", value)?;
            }
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
        // ignore restart errors when Finder isn't running
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
