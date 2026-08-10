use super::{Host, TempPath};
use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacDefault {
    Appearance(bool),
    DockAutohide(bool),
    DockRecentApplications(bool),
    FinderExtensions(bool),
    FinderHiddenFiles(bool),
    KeyRepeat(i32),
    InitialKeyRepeat(i32),
    TrackpadTapToClick(bool),
}

pub fn ensure_admin(host: &Host) -> Result<()> {
    host.require("macOS administrator access", "sudo", ["-v"])?;
    Ok(())
}

pub fn bootstrap(host: &Host) -> Result<()> {
    if brew_program(host).is_ok() {
        return Ok(());
    }
    let script = TempPath::new(host, "homebrew-install")?;
    host.require(
        "Homebrew installer download",
        "curl",
        [
            "--proto",
            "=https",
            "--fail",
            "--location",
            "--output",
            script.path().to_str().ok_or_else(|| anyhow::anyhow!("Homebrew installer path is not UTF-8"))?,
            "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh",
        ],
    )?;
    host.require(
        "Homebrew bootstrap",
        "/bin/bash",
        [script.path().to_str().ok_or_else(|| anyhow::anyhow!("Homebrew installer path is not UTF-8"))?],
    )?;
    Ok(())
}

pub fn packages(host: &Host, formulae: &[String], casks: &[String]) -> Result<()> {
    let brew = brew_program(host)?;
    for formula in formulae {
        if !installed(host, &brew, "--formula", formula)? {
            host.require("Homebrew formula install", &brew, ["install", formula])?;
        }
    }
    for cask in casks {
        if !installed(host, &brew, "--cask", cask)? {
            host.require("Homebrew cask install", &brew, ["install", "--cask", cask])?;
        }
    }
    Ok(())
}

fn installed(host: &Host, brew: &str, kind: &str, name: &str) -> Result<bool> {
    Ok(host.run(brew, ["list", kind, name])?.status.success())
}

fn brew_program(host: &Host) -> Result<String> {
    for candidate in ["brew", "/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
        if host.run(candidate, ["--version"]).is_ok_and(|output| output.status.success()) {
            return Ok(candidate.to_owned());
        }
    }
    bail!(
        "Homebrew is unavailable after bootstrap; expected brew on PATH, /opt/homebrew/bin/brew, or /usr/local/bin/brew"
    )
}

pub fn xcode_command_line_tools(host: &Host) -> Result<()> {
    if host.run("xcode-select", ["-p"]).is_ok_and(|output| output.status.success()) {
        return Ok(());
    }
    host.require("Xcode command line tools", "xcode-select", ["--install"])?;
    Ok(())
}

pub fn rosetta(host: &Host) -> Result<()> {
    host.require("Rosetta", "softwareupdate", ["--install-rosetta", "--agree-to-license"])?;
    Ok(())
}

pub fn update(host: &Host, formulae: bool, casks: bool) -> Result<()> {
    let brew = brew_program(host)?;
    host.require("Homebrew metadata refresh", &brew, ["update"])?;
    if formulae {
        host.require("Homebrew formula updates", &brew, ["upgrade"])?;
    }
    if casks {
        host.require("Homebrew cask updates", &brew, ["upgrade", "--cask"])?;
    }
    Ok(())
}

pub fn defaults(host: &Host, settings: &[MacDefault]) -> Result<()> {
    for setting in settings {
        match setting {
            MacDefault::Appearance(dark) => {
                if *dark {
                    host.require(
                        "macOS appearance",
                        "defaults",
                        ["write", "-g", "AppleInterfaceStyle", "-string", "Dark"],
                    )?;
                } else {
                    host.run("defaults", ["delete", "-g", "AppleInterfaceStyle"]).ok();
                }
            }
            MacDefault::DockAutohide(value) => write_bool(host, "com.apple.dock", "autohide", *value)?,
            MacDefault::DockRecentApplications(value) => write_bool(host, "com.apple.dock", "show-recents", *value)?,
            MacDefault::FinderExtensions(value) => {
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
