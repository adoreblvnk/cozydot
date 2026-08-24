//! Derive prerequisites & run each platform's operations in dependency order.

use crate::{
    config::{
        AptArch, BinaryFormat, Config, Dotfiles, Enablement, Fonts, Gnome, LinuxDesktop, LinuxIntegrations,
        MacosDesktop, Theme, ToolUpdates, Tools, select_distro_entry, select_repo_codename,
    },
    operations::{
        desktop::{self, fonts, gnome, macos as macos_defaults},
        dotfiles,
        host::{self, macos as macos_host, users},
        integrations::{docker, vscode},
        packages::{
            apt::{
                self,
                repo::{self, AptRepo},
            },
            binary::{self, appimaged},
            cargo, flatpak, homebrew, npm, snapd,
        },
        toolchains::{fnm, go, rustup, uv},
    },
    platform::{Arch, Distro, Platform, PlatformIdentity},
    style::STATUS,
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const APT_PREREQS: [&str; 8] =
    ["ca-certificates", "curl", "fontconfig", "gnupg", "stow", "unzip", "xdg-terminal-exec", "xz-utils"];

pub fn apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    host::home()?;
    match platform.identity {
        PlatformIdentity::Macos => macos_apply(config, platform.arch, dotfiles_root),
        PlatformIdentity::Linux { .. } => linux_apply(config, platform, dotfiles_root),
    }
}

pub fn dotfiles(config: &Config, platform: &Platform, root: &Path, replace: bool) -> Result<()> {
    let platform_packages = match platform.identity {
        PlatformIdentity::Macos => &config.dotfiles.packages.macos,
        PlatformIdentity::Linux { .. } => &config.dotfiles.packages.linux,
    };
    if let Some(packages) = dotfile_packages(&config.dotfiles, platform_packages) {
        host::home()?;
        run("Applying", "dotfiles", || dotfiles::apply(root, &packages, replace || config.dotfiles.replace))?;
    }
    Ok(())
}

pub fn update(config: &Config, platform: &Platform) -> Result<()> {
    host::home()?;
    match platform.identity {
        PlatformIdentity::Macos => macos_update(config, platform.arch),
        PlatformIdentity::Linux { .. } => linux_update(config, platform.arch),
    }
}

fn linux_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    let PlatformIdentity::Linux { distro, .. } = platform.identity else { unreachable!() };
    let theme = config.desktop.as_ref().and_then(|desktop| desktop.theme);
    let desktop_config = config.desktop.as_ref().and_then(|desktop| desktop.linux.as_ref());
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    let mut repos = Vec::new();
    let mut repo_conflicts = Vec::new();
    let mut repo_packages = Vec::new();
    let apt_arch = match platform.arch {
        Arch::X86_64 => AptArch::Amd64,
        Arch::Aarch64 => AptArch::Arm64,
    };
    if let Some(apt) = &config.packages.linux.apt {
        for repo in &apt.repos {
            if repo.arch.as_ref().is_some_and(|values| !values.contains(&apt_arch)) {
                continue;
            }
            let Some((key, source_uri)) = select_distro_entry(&repo.uris, platform.identity) else { continue };
            let suite = if repo.suite == "codename" {
                select_repo_codename(key, platform).to_owned()
            } else {
                repo.suite.clone()
            };
            let apt_repo = AptRepo::new(
                repo.name.clone(),
                repo.key_url.clone(),
                source_uri.to_owned(),
                platform.arch,
                suite,
                repo.components.clone(),
                PathBuf::from(&repo.key_path),
            );
            repos.push((repo.name.as_str(), apt_repo));
            repo_conflicts.extend(repo.conflicts.iter().cloned());
            repo_packages.extend(repo.packages.iter().cloned());
        }
    }
    let flatpak_refs = (!config.packages.linux.flatpak.is_empty()).then_some(config.packages.linux.flatpak.as_slice());
    if flatpak_refs.is_some() {
        apt_prereqs.insert("flatpak");
    }
    let mut deb_binaries = Vec::new();
    let mut appimages = Vec::new();
    for package in &config.packages.linux.binaries {
        let Some(source) = binary::select_source(package, platform.arch) else { continue };
        match package.format {
            BinaryFormat::Deb => deb_binaries.push((package, source)),
            BinaryFormat::AppImage => appimages.push((package, source)),
        }
    }
    if theme.is_some() || desktop_config.is_some_and(LinuxDesktop::has_intent) {
        apt_prereqs.extend(["dconf-cli", "libglib2.0-bin"]);
        if desktop_config.and_then(|desktop| desktop.gnome.as_ref()).is_some_and(Gnome::has_intent) {
            apt_prereqs.insert("gnome-shell");
        }
    }

    // establish distro services and package prerequisites before third-party repositories
    if distro == Distro::Debian {
        if config.system.debian.as_ref().is_some_and(|debian| debian.sudo_group) {
            run("Configuring", "sudo group membership", users::ensure_in_sudo_group)?;
        }
        run("Enabling", "Debian APT components", repo::debian_components::add)?;
    }
    if distro == Distro::Ubuntu
        && let Some(ubuntu) = &config.system.ubuntu
    {
        if let Some(state) = ubuntu.unattended_upgrades {
            run("Configuring", "unattended-upgrades", || apt::set_unattended_upgrades(state == Enablement::Enabled))?;
        }
        if let Some(state) = ubuntu.snapd {
            run("Configuring", "snapd", || snapd::set_enabled(state == Enablement::Enabled))?;
        }
        if ubuntu.restricted_extras {
            run("Installing", "ubuntu-restricted-extras", || apt::install(&["ubuntu-restricted-extras".into()]))?;
        }
    }
    run("Updating", "APT package metadata", apt::update)?;
    run("Installing", "APT prerequisites", || {
        apt::install(&apt_prereqs.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>())
    })?;
    if let Some(apt) = &config.packages.linux.apt
        && !apt.install.is_empty()
    {
        run("Installing", "configured APT packages", || apt::install(&apt.install))?;
    }
    // add repositories before changing packages supplied by them
    if !repos.is_empty() {
        for (name, apt_repo) in repos {
            let subject = format!("{name} APT repository");
            run("Adding", &subject, || repo::add(&apt_repo))?;
        }
        run("Updating", "APT package metadata", apt::update)?;
    }
    if !repo_conflicts.is_empty() {
        run("Removing", "conflicting APT packages", || apt::purge(&repo_conflicts))?;
    }
    if !repo_packages.is_empty() {
        run("Installing", "APT repository packages", || apt::install(&repo_packages))?;
    }
    if let Some(refs) = flatpak_refs {
        run("Adding", "Flathub remote", flatpak::add_flathub_remote)?;
        run("Installing", "Flatpak apps", || flatpak::install(refs))?;
    }
    for (package, source) in deb_binaries {
        let subject = format!("{} binary package", package.name);
        run("Installing", &subject, || binary::install(package, platform.arch, source))?;
    }
    if !appimages.is_empty() {
        run("Installing", "appimaged", || appimaged::install(platform.arch))?;
        for (package, source) in appimages {
            let subject = format!("{} binary package", package.name);
            run("Installing", &subject, || binary::install(package, platform.arch, source))?;
        }
    }
    apply_tools(&config.tools, platform.arch)?;
    if let Some(families) = nerd_fonts(&config.fonts) {
        run("Installing", "Nerd Fonts", || fonts::apply(families, false))?;
    }
    if let Some(packages) = dotfile_packages(&config.dotfiles, &config.dotfiles.packages.linux) {
        run("Applying", "dotfiles", || dotfiles::apply(dotfiles_root, &packages, config.dotfiles.replace))?;
    }
    linux_integrations(&config.integrations.linux)?;
    apply_vscode_extensions(&config.integrations.vscode.extensions)?;
    linux_desktop(theme, desktop_config)?;
    Ok(())
}

fn macos_apply(config: &Config, arch: Arch, dotfiles_root: &Path) -> Result<()> {
    let theme = config.desktop.as_ref().and_then(|desktop| desktop.theme);
    let desktop_config = config.desktop.as_ref().and_then(|desktop| desktop.macos.as_ref());
    let dotfiles = dotfile_packages(&config.dotfiles, &config.dotfiles.packages.macos);
    let homebrew = &config.packages.macos.homebrew;
    let mut formulae = homebrew.formulae.clone();
    if !formulae.iter().any(|formula| formula == "stow") {
        formulae.push("stow".into());
    }

    if config.system.macos.validate_sudo_access {
        run("Validating", "macOS sudo access", macos_host::validate_sudo_access)?;
    }
    if config.system.macos.xcode.command_line_tools {
        run("Installing", "Command Line Tools for Xcode", macos_host::install_command_line_tools)?;
    }
    // install Homebrew and Stow before applying package-backed user configuration
    run("Installing", "Homebrew", homebrew::install)?;
    run("Installing", "Homebrew packages", || homebrew::install_packages(&formulae, &homebrew.casks))?;
    apply_tools(&config.tools, arch)?;
    if let Some(families) = nerd_fonts(&config.fonts) {
        run("Installing", "Nerd Fonts", || fonts::apply(families, false))?;
    }
    if let Some(packages) = dotfiles {
        run("Applying", "dotfiles", || dotfiles::apply(dotfiles_root, &packages, config.dotfiles.replace))?;
    }
    apply_vscode_extensions(&config.integrations.vscode.extensions)?;
    if theme.is_some() || desktop_config.is_some_and(MacosDesktop::has_intent) {
        run("Writing", "macOS defaults", || macos_defaults::write_defaults(theme, desktop_config))?;
    }
    Ok(())
}

fn apply_tools(tools: &Tools, arch: Arch) -> Result<()> {
    if let Some(selector) = tools.rust.as_deref() {
        run("Installing", "Rust toolchain", || rustup::install(selector))?;
        run("Installing", "cargo-binstall", cargo::install_binstall)?;
        run("Installing", "cargo-update", || cargo::install_crates(&["cargo-update".to_owned()]))?;
    }
    if let Some(selector) = tools.node.as_deref() {
        run("Installing", "fnm", fnm::install)?;
        run("Installing", "Node.js", || fnm::install_version(selector))?;
    }
    if let Some(selector) = &tools.python {
        run("Installing", "uv", uv::install)?;
        run("Installing", "Python", || uv::install_py(selector))?;
    }
    if let Some(selector) = tools.go.as_deref() {
        run("Installing", "Go toolchain", || go::install_toolchain(selector, arch))?;
    }
    if !tools.cargo.is_empty() {
        run("Installing", "Cargo crates", || cargo::install_crates(&tools.cargo))?;
    }
    if !tools.npm.is_empty() {
        run("Installing", "npm packages", || npm::install(&tools.npm))?;
    }
    Ok(())
}

fn linux_update(config: &Config, arch: Arch) -> Result<()> {
    let updates = &config.updates.packages.linux;
    let flatpak = updates.flatpak;
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    if flatpak {
        apt_prereqs.insert("flatpak");
    }

    // refresh package metadata before upgrades and prerequisite reconciliation
    run("Updating", "APT package metadata", apt::update)?;
    if let Some(policy) = updates.apt {
        run("Upgrading", "APT packages", || apt::upgrade(policy))?;
    }
    run("Installing", "APT prerequisites", || {
        apt::install(&apt_prereqs.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>())
    })?;
    if flatpak {
        run("Updating", "Flatpak apps", flatpak::update)?;
    }
    update_tools(&config.tools, &config.updates.tools, arch)?;
    if config.updates.fonts
        && let Some(families) = nerd_fonts(&config.fonts)
    {
        run("Updating", "Nerd Fonts", || fonts::apply(families, true))?;
    }
    Ok(())
}

fn macos_update(config: &Config, arch: Arch) -> Result<()> {
    let homebrew = &config.updates.packages.macos.homebrew;
    let formulae = homebrew.formulae;
    let casks = homebrew.casks;

    run("Installing", "Homebrew", homebrew::install)?;
    if formulae || casks {
        run("Updating", "Homebrew packages", || homebrew::update_and_upgrade(formulae, casks))?;
    }
    if config.updates.tools.npm && !config.updates.tools.node {
        run("Installing", "fnm", fnm::install)?;
    }
    update_tools(&config.tools, &config.updates.tools, arch)?;
    if config.updates.fonts
        && let Some(families) = nerd_fonts(&config.fonts)
    {
        run("Updating", "Nerd Fonts", || fonts::apply(families, true))?;
    }
    Ok(())
}

fn update_tools(tools: &Tools, updates: &ToolUpdates, arch: Arch) -> Result<()> {
    if updates.rust {
        run("Installing", "Rust toolchain", || rustup::install(tools.rust.as_deref().unwrap_or("stable")))?;
        run("Updating", "Rust toolchains", rustup::update_toolchains)?;
    }
    if updates.node {
        run("Installing", "fnm", fnm::install)?;
        run("Installing", "Node.js", || fnm::install_version(tools.node.as_deref().unwrap_or("latest")))?;
    }
    if updates.python {
        run("Installing", "uv", uv::install)?;
        run("Upgrading", "Python", uv::upgrade_py)?;
    }
    if updates.go {
        run("Updating", "Go toolchain", || go::update_toolchain(tools.go.as_deref().unwrap_or("latest"), arch))?;
    }
    if updates.cargo {
        run("Updating", "Cargo crates", cargo::update_crates)?;
    }
    if updates.npm {
        run("Updating", "npm packages", npm::update)?;
    }
    Ok(())
}

fn nerd_fonts(fonts: &Fonts) -> Option<&[String]> {
    (!fonts.nerd.is_empty()).then_some(fonts.nerd.as_slice())
}

fn dotfile_packages(dotfiles: &Dotfiles, platform_packages: &[String]) -> Option<Vec<String>> {
    let packages = dotfiles.packages.all.iter().chain(platform_packages).cloned().collect::<Vec<_>>();
    (!packages.is_empty()).then_some(packages)
}

fn linux_integrations(integrations: &LinuxIntegrations) -> Result<()> {
    if let Some(docker) = &integrations.docker {
        if docker.group {
            run("Configuring", "Docker group membership", || users::ensure_in_group("Docker", "docker", "docker"))?;
        }
        if let Some(logging) = &docker.logging {
            run("Configuring", "Docker local logging driver", || {
                docker::set_local_logging_driver(logging.max_size.as_deref())
            })?;
        }
    }
    if integrations.virtualbox.as_ref().is_some_and(|virtualbox| virtualbox.group) {
        run("Configuring", "VirtualBox group membership", || {
            users::ensure_in_group("VirtualBox", "VBoxManage", "vboxusers")
        })?;
    }
    Ok(())
}

fn apply_vscode_extensions(extensions: &[String]) -> Result<()> {
    if !extensions.is_empty() {
        run("Installing", "Visual Studio Code extensions", || vscode::install_extensions(extensions))?;
    }
    Ok(())
}

fn linux_desktop(theme: Option<Theme>, desktop: Option<&LinuxDesktop>) -> Result<()> {
    if let Some(theme) = theme {
        run("Setting", "desktop color scheme", || desktop::set_color_scheme(theme))?;
    }
    let Some(gnome) = desktop.and_then(|desktop| desktop.gnome.as_ref()).filter(|gnome| gnome.has_intent()) else {
        return Ok(());
    };
    if let Some(executable) = &gnome.terminal {
        run("Setting", "default terminal", || desktop::set_terminal(executable))?;
    }
    if let Some(idle) = &gnome.idle {
        if let Some(timeout) = idle.timeout {
            run("Setting", "idle timeout", || desktop::set_idle_delay(timeout.seconds()))?;
        }
        if let Some(enabled) = idle.dim {
            run("Setting", "idle dimming", || desktop::set_idle_dim(enabled))?;
        }
    }
    if !gnome.extensions.is_empty() {
        run_with_outcome("Applying", "GNOME extensions", || gnome::apply_extensions(&gnome.extensions))?;
    }
    if gnome.dash_to_dock {
        run_with_outcome("Installing", "Dash to Dock", gnome::apply_dash_to_dock)?;
    }
    if gnome.rounded_window_corners {
        run_with_outcome("Installing", "Rounded Window Corners", gnome::apply_rounded_window_corners)?;
    }
    Ok(())
}

fn run(status: &str, subject: &str, operation: impl FnOnce() -> Result<()>) -> Result<()> {
    anstream::eprintln!("{STATUS}{status:>12}{STATUS:#} {subject}");
    operation().with_context(|| format!("{} {subject}", status.to_lowercase()))
}

fn run_with_outcome(status: &str, subject: &str, operation: impl FnOnce() -> Result<gnome::Outcome>) -> Result<()> {
    anstream::eprintln!("{STATUS}{status:>12}{STATUS:#} {subject}");
    let action = status.to_lowercase();
    if operation().with_context(|| format!("{action} {subject}"))? == gnome::Outcome::LoginRequired {
        eprintln!("note: log out and back in to finish {action} {subject}");
    }
    Ok(())
}
