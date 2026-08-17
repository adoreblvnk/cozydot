use super::{Host, TempPath};
use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacDefault {
    DarkMode(bool),
    DockAutohide(bool),
    DockRecentApplications(bool),
    ShowAllFilenameExtensions(bool),
    FinderHiddenFiles(bool),
    KeyRepeat(i32),
    InitialKeyRepeat(i32),
    TrackpadTapToClick(bool),
}

pub fn validate_sudo_access(host: &Host) -> Result<()> {
    host.run("macOS sudo access", "sudo", ["-v"])?;
    Ok(())
}

pub fn install_homebrew(host: &Host) -> Result<()> {
    if find_brew(host).is_ok() {
        return Ok(());
    }
    let script = TempPath::new(host, "homebrew-install")?;
    host.curl(
        "Homebrew installer download",
        "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh",
        [
            "--proto",
            "=https",
            "--output",
            script.path().to_str().ok_or_else(|| anyhow::anyhow!("Homebrew installer path is not UTF-8"))?,
        ],
    )?;
    host.run(
        "Homebrew install",
        "/bin/bash",
        [script.path().to_str().ok_or_else(|| anyhow::anyhow!("Homebrew installer path is not UTF-8"))?],
    )?;
    Ok(())
}

pub fn install_packages(host: &Host, formulae: &[String], casks: &[String]) -> Result<()> {
    let brew = find_brew(host)?;
    for formula in formulae {
        if !is_installed(host, &brew, "--formula", formula)? {
            host.run("Homebrew formula install", &brew, ["install", formula])?;
        }
    }
    for cask in casks {
        if !is_installed(host, &brew, "--cask", cask)? {
            host.run("Homebrew cask install", &brew, ["install", "--cask", cask])?;
        }
    }
    Ok(())
}

fn is_installed(host: &Host, brew: &str, kind: &str, name: &str) -> Result<bool> {
    Ok(host.output(brew, ["list", kind, name])?.status.success())
}

pub(crate) fn install_formula(host: &Host, formula: &str) -> Result<()> {
    let brew = find_brew(host)?;
    if !is_installed(host, &brew, "--formula", formula)? {
        host.run("Homebrew formula install", &brew, ["install", formula])?;
    }
    Ok(())
}

pub(crate) fn formula_executable(host: &Host, formula: &str, executable: &str) -> Result<String> {
    let brew = find_brew(host)?;
    let output = host.run("Homebrew formula prefix", &brew, ["--prefix", formula])?;
    let prefix = std::str::from_utf8(&output.stdout)?.trim();
    let program = std::path::Path::new(prefix).join("bin").join(executable);
    program.to_str().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("Homebrew executable path is not UTF-8"))
}

fn find_brew(host: &Host) -> Result<String> {
    for candidate in ["brew", "/opt/homebrew/bin/brew"] {
        if host.output(candidate, ["--version"]).is_ok_and(|output| output.status.success()) {
            return Ok(candidate.to_owned());
        }
    }
    bail!("Homebrew is unavailable after install; expected brew on PATH or /opt/homebrew/bin/brew")
}

pub fn install_command_line_tools_for_xcode(host: &Host) -> Result<()> {
    if host.output("xcode-select", ["-p"]).is_ok_and(|output| output.status.success()) {
        return Ok(());
    }
    // launch Apple's interactive installer; success only means it started
    host.run("Command Line Tools for Xcode install", "xcode-select", ["--install"])?;
    Ok(())
}

pub fn update_and_upgrade(host: &Host, formulae: bool, casks: bool) -> Result<()> {
    let brew = find_brew(host)?;
    host.run("Homebrew update", &brew, ["update"])?;
    if formulae {
        host.run("Homebrew formula upgrade", &brew, ["upgrade"])?;
    }
    if casks {
        host.run("Homebrew cask upgrade", &brew, ["upgrade", "--cask"])?;
    }
    Ok(())
}

pub fn write_defaults(host: &Host, settings: &[MacDefault]) -> Result<()> {
    for setting in settings {
        match setting {
            MacDefault::DarkMode(dark) => {
                if *dark {
                    host.run(
                        "macOS appearance",
                        "defaults",
                        ["write", "-g", "AppleInterfaceStyle", "-string", "Dark"],
                    )?;
                } else {
                    // ignore deletion errors because a missing preference already means light mode
                    host.output("defaults", ["delete", "-g", "AppleInterfaceStyle"]).ok();
                }
            }
            MacDefault::DockAutohide(value) => write_bool(host, "com.apple.dock", "autohide", *value)?,
            MacDefault::DockRecentApplications(value) => write_bool(host, "com.apple.dock", "show-recents", *value)?,
            MacDefault::ShowAllFilenameExtensions(value) => {
                write_bool(host, "NSGlobalDomain", "AppleShowAllExtensions", *value)?
            }
            MacDefault::FinderHiddenFiles(value) => write_bool(host, "com.apple.finder", "AppleShowAllFiles", *value)?,
            MacDefault::KeyRepeat(value) => write_int(host, "NSGlobalDomain", "KeyRepeat", *value)?,
            MacDefault::InitialKeyRepeat(value) => write_int(host, "NSGlobalDomain", "InitialKeyRepeat", *value)?,
            MacDefault::TrackpadTapToClick(value) => {
                write_bool(host, "com.apple.AppleMultitouchTrackpad", "Clicking", *value)?
            }
        }
    }
    // ignore restart errors when Dock or Finder isn't running
    host.run("Dock restart", "killall", ["Dock"]).ok();
    host.run("Finder restart", "killall", ["Finder"]).ok();
    Ok(())
}

fn write_bool(host: &Host, domain: &str, key: &str, value: bool) -> Result<()> {
    host.run("macOS defaults", "defaults", ["write", domain, key, "-bool", if value { "true" } else { "false" }])?;
    Ok(())
}

fn write_int(host: &Host, domain: &str, key: &str, value: i32) -> Result<()> {
    host.run("macOS defaults", "defaults", ["write", domain, key, "-int", &value.to_string()])?;
    Ok(())
}
