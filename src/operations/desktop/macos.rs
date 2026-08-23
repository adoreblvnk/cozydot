use crate::{
    config::{MacDesktop, Theme},
    operations::host,
};
use anyhow::Result;

pub(crate) fn write_defaults(theme: Option<Theme>, desktop: Option<&MacDesktop>) -> Result<()> {
    write_defaults_with(theme, desktop, |label, program, args| {
        host::run(label, program, args.iter().copied()).map(|_| ())
    })
}

fn write_defaults_with(
    theme: Option<Theme>,
    desktop: Option<&MacDesktop>,
    mut run: impl FnMut(&str, &str, &[&str]) -> Result<()>,
) -> Result<()> {
    let mut restart_dock = false;
    let mut restart_finder = false;
    if let Some(theme) = theme {
        if theme == Theme::Dark {
            run("macOS appearance", "defaults", &["write", "-g", "AppleInterfaceStyle", "-string", "Dark"])?;
        } else {
            // ignore deletion errors because a missing preference already means light mode
            run("macOS appearance", "defaults", &["delete", "-g", "AppleInterfaceStyle"]).ok();
        }
    }
    if let Some(desktop) = desktop {
        if let Some(dock) = &desktop.dock {
            if let Some(value) = dock.autohide {
                write_bool(&mut run, "com.apple.dock", "autohide", value)?;
                restart_dock = true;
            }
            if let Some(value) = dock.show_recent_applications {
                write_bool(&mut run, "com.apple.dock", "show-recents", value)?;
                restart_dock = true;
            }
        }
        if let Some(finder) = &desktop.finder {
            if let Some(value) = finder.show_filename_extensions {
                write_bool(&mut run, "NSGlobalDomain", "AppleShowAllExtensions", value)?;
                restart_finder = true;
            }
            if let Some(value) = finder.show_hidden_files {
                write_bool(&mut run, "com.apple.finder", "AppleShowAllFiles", value)?;
                restart_finder = true;
            }
        }
        if let Some(keyboard) = &desktop.keyboard {
            if let Some(value) = keyboard.key_repeat {
                write_int(&mut run, "NSGlobalDomain", "KeyRepeat", value)?;
            }
            if let Some(value) = keyboard.initial_key_repeat {
                write_int(&mut run, "NSGlobalDomain", "InitialKeyRepeat", value)?;
            }
        }
        if let Some(trackpad) = &desktop.trackpad
            && let Some(value) = trackpad.tap_to_click
        {
            write_bool(&mut run, "com.apple.AppleMultitouchTrackpad", "Clicking", value)?;
        }
    }
    if restart_dock {
        // ignore restart errors when Dock isn't running
        run("Dock restart", "killall", &["Dock"]).ok();
    }
    if restart_finder {
        // ignore restart errors when Finder isn't running
        run("Finder restart", "killall", &["Finder"]).ok();
    }
    Ok(())
}

fn write_bool(
    run: &mut impl FnMut(&str, &str, &[&str]) -> Result<()>,
    domain: &str,
    key: &str,
    value: bool,
) -> Result<()> {
    run("macOS defaults", "defaults", &["write", domain, key, "-bool", if value { "true" } else { "false" }])
}

fn write_int(
    run: &mut impl FnMut(&str, &str, &[&str]) -> Result<()>,
    domain: &str,
    key: &str,
    value: i32,
) -> Result<()> {
    run("macOS defaults", "defaults", &["write", domain, key, "-int", &value.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MacDock, MacFinder, MacKeyboard, MacTrackpad};

    fn restarts(theme: Option<Theme>, desktop: Option<&MacDesktop>) -> Vec<String> {
        let mut restarts = Vec::new();
        write_defaults_with(theme, desktop, |_, program, args| {
            if program == "killall" {
                restarts.push(args[0].to_owned());
            }
            Ok(())
        })
        .unwrap();
        restarts
    }

    fn desktop() -> MacDesktop {
        MacDesktop { dock: None, finder: None, keyboard: None, trackpad: None }
    }

    #[test]
    fn dock_preferences_restart_only_dock() {
        let desktop =
            MacDesktop { dock: Some(MacDock { autohide: Some(true), show_recent_applications: None }), ..desktop() };
        assert_eq!(restarts(None, Some(&desktop)), ["Dock"]);
    }

    #[test]
    fn finder_preferences_restart_only_finder() {
        let desktop = MacDesktop {
            finder: Some(MacFinder { show_filename_extensions: Some(true), show_hidden_files: None }),
            ..desktop()
        };
        assert_eq!(restarts(None, Some(&desktop)), ["Finder"]);
    }

    #[test]
    fn unrelated_preferences_restart_neither_process() {
        let desktop = MacDesktop {
            keyboard: Some(MacKeyboard { key_repeat: Some(2), initial_key_repeat: None }),
            trackpad: Some(MacTrackpad { tap_to_click: Some(true) }),
            ..desktop()
        };
        assert!(restarts(Some(Theme::Dark), Some(&desktop)).is_empty());
    }
}
