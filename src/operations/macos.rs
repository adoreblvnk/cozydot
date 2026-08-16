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
    host.require("macOS sudo access", "sudo", ["-v"])?;
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
    host.require(
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
            host.require("Homebrew formula install", &brew, ["install", formula])?;
        }
    }
    for cask in casks {
        if !is_installed(host, &brew, "--cask", cask)? {
            host.require("Homebrew cask install", &brew, ["install", "--cask", cask])?;
        }
    }
    Ok(())
}

fn is_installed(host: &Host, brew: &str, kind: &str, name: &str) -> Result<bool> {
    Ok(host.run(brew, ["list", kind, name])?.status.success())
}

pub(crate) fn install_formula(host: &Host, formula: &str) -> Result<()> {
    install_homebrew(host)?;
    let brew = find_brew(host)?;
    if !is_installed(host, &brew, "--formula", formula)? {
        host.require("Homebrew formula install", &brew, ["install", formula])?;
    }
    Ok(())
}

pub(crate) fn formula_executable(host: &Host, formula: &str, executable: &str) -> Result<String> {
    let brew = find_brew(host)?;
    let output = host.require("Homebrew formula prefix", &brew, ["--prefix", formula])?;
    let prefix = std::str::from_utf8(&output.stdout)?.trim();
    let program = std::path::Path::new(prefix).join("bin").join(executable);
    program.to_str().map(str::to_owned).ok_or_else(|| anyhow::anyhow!("Homebrew executable path is not UTF-8"))
}

fn find_brew(host: &Host) -> Result<String> {
    for candidate in ["brew", "/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
        if host.run(candidate, ["--version"]).is_ok_and(|output| output.status.success()) {
            return Ok(candidate.to_owned());
        }
    }
    bail!(
        "Homebrew is unavailable after install; expected brew on PATH, /opt/homebrew/bin/brew, or /usr/local/bin/brew"
    )
}

pub fn install_command_line_tools_for_xcode(host: &Host) -> Result<()> {
    if host.run("xcode-select", ["-p"]).is_ok_and(|output| output.status.success()) {
        return Ok(());
    }
    // launch Apple's interactive installer; success only means it started
    host.require("Xcode command line tools", "xcode-select", ["--install"])?;
    Ok(())
}

pub fn update(host: &Host, formulae: bool, casks: bool) -> Result<()> {
    let brew = find_brew(host)?;
    host.require("Homebrew update", &brew, ["update"])?;
    if formulae {
        host.require("Homebrew formula updates", &brew, ["upgrade"])?;
    }
    if casks {
        host.require("Homebrew cask updates", &brew, ["upgrade", "--cask"])?;
    }
    Ok(())
}

pub fn write_defaults(host: &Host, settings: &[MacDefault]) -> Result<()> {
    for setting in settings {
        match setting {
            MacDefault::DarkMode(dark) => {
                if *dark {
                    host.require(
                        "macOS appearance",
                        "defaults",
                        ["write", "-g", "AppleInterfaceStyle", "-string", "Dark"],
                    )?;
                } else {
                    // ignore deletion errors because a missing preference already means light mode
                    host.run("defaults", ["delete", "-g", "AppleInterfaceStyle"]).ok();
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
    host.require("Dock restart", "killall", ["Dock"]).ok();
    host.require("Finder restart", "killall", ["Finder"]).ok();
    Ok(())
}

fn write_bool(host: &Host, domain: &str, key: &str, value: bool) -> Result<()> {
    host.require("macOS defaults", "defaults", ["write", domain, key, "-bool", if value { "true" } else { "false" }])?;
    Ok(())
}

fn write_int(host: &Host, domain: &str, key: &str, value: i32) -> Result<()> {
    host.require("macOS defaults", "defaults", ["write", domain, key, "-int", &value.to_string()])?;
    Ok(())
}
