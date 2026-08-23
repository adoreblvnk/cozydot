//! Derive prerequisites & run each platform's operations in dependency order.

use crate::{
    config::{
        AptArchitecture, AptRepoConfig, BinaryFormat, BinarySource, Config, DistroMapKey, Enablement, Fonts, Gnome,
        LinuxDesktop, LinuxIntegrations, MacDesktop, SharedConfig, SharedPackages, Theme, Tools, select_distro_entry,
        select_repo_codename,
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
    platform::{Architecture, Distro, Platform, PlatformIdentity},
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
        PlatformIdentity::Macos => macos_apply(config, platform.architecture, dotfiles_root),
        PlatformIdentity::Linux { .. } => linux_apply(config, platform, dotfiles_root),
    }
}

pub fn dotfiles(config: &Config, platform: &Platform, root: &Path, replace: bool) -> Result<()> {
    let platform_packages = match platform.identity {
        PlatformIdentity::Macos => &config.macos.dotfiles.packages,
        PlatformIdentity::Linux { .. } => &config.linux.dotfiles.packages,
    };
    if let Some(dotfiles) = dotfiles_packages(&config.shared.dotfiles.packages, platform_packages) {
        host::home()?;
        run("Applying", "dotfiles", || dotfiles::apply(root, &dotfiles, replace))?;
    }
    Ok(())
}

pub fn update(config: &Config, platform: &Platform) -> Result<()> {
    host::home()?;
    match platform.identity {
        PlatformIdentity::Macos => macos_update(config, platform.architecture),
        PlatformIdentity::Linux { .. } => linux_update(config, platform.architecture),
    }
}

fn linux_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    let PlatformIdentity::Linux { distro, .. } = platform.identity else { unreachable!() };
    let theme = config.shared.desktop.as_ref().and_then(|desktop| desktop.theme);
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    let mut repos = Vec::new();
    let mut repo_packages_to_purge = Vec::new();
    let mut repo_packages_to_install = Vec::new();
    let apt_architecture = match platform.architecture {
        Architecture::X86_64 => AptArchitecture::Amd64,
        Architecture::Aarch64 => AptArchitecture::Arm64,
    };
    if let Some(apt) = &config.linux.packages.apt
        && let Some(configured_repos) = &apt.repos
    {
        for repo in configured_repos {
            if repo.arch.as_ref().is_some_and(|values| !values.contains(&apt_architecture)) {
                continue;
            }
            let Some((key, source_uri)) = select_distro_entry(&repo.uris, platform.identity) else { continue };
            repos.push((repo.name.as_str(), build_apt_repo(repo, platform, key, source_uri)));
            repo_packages_to_purge.extend(repo.conflicts.iter().cloned());
            repo_packages_to_install.extend(repo.packages.iter().cloned());
        }
    }
    let flatpak_refs = config.linux.packages.flatpak.as_deref().filter(|values| !values.is_empty());
    if flatpak_refs.is_some() {
        apt_prereqs.insert("flatpak");
    }
    let mut deb_binaries = Vec::new();
    let mut appimages = Vec::new();
    for binary in config.linux.packages.binaries.as_deref().unwrap_or_default() {
        let supported = match &binary.source {
            BinarySource::GitHub { assets, .. } => assets.get(platform.architecture).is_some(),
            BinarySource::Url { urls } => urls.get(platform.architecture).is_some(),
        };
        if supported {
            match binary.format {
                BinaryFormat::Deb => deb_binaries.push(binary),
                BinaryFormat::AppImage => appimages.push(binary),
            }
        }
    }
    add_desktop_prereqs(theme, config.linux.desktop.as_ref(), &mut apt_prereqs);

    // establish distro services and package prerequisites before third-party repositories
    if distro == Distro::Debian {
        if config.linux.system.debian.as_ref().is_some_and(|debian| debian.sudo_group == Some(true)) {
            run("Configuring", "sudo group membership", users::ensure_in_sudo_group)?;
        }
        run("Enabling", "Debian APT components", repo::debian_components::add)?;
    }
    if distro == Distro::Ubuntu
        && let Some(ubuntu) = &config.linux.system.ubuntu
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
    if let Some(apt) = &config.linux.packages.apt
        && let Some(packages) = &apt.install
        && !packages.is_empty()
    {
        run("Installing", "configured APT packages", || apt::install(packages))?;
    }
    // add repositories before changing packages supplied by them
    if !repos.is_empty() {
        for (name, apt_repo) in repos {
            let subject = format!("{name} APT repository");
            run("Adding", &subject, || repo::add(&apt_repo))?;
        }
        run("Updating", "APT package metadata", apt::update)?;
    }
    if !repo_packages_to_purge.is_empty() {
        run("Removing", "conflicting APT packages", || apt::purge(&repo_packages_to_purge))?;
    }
    if !repo_packages_to_install.is_empty() {
        run("Installing", "APT repository packages", || apt::install(&repo_packages_to_install))?;
    }
    if let Some(refs) = flatpak_refs {
        run("Adding", "Flathub remote", flatpak::add_flathub_remote)?;
        run("Installing", "Flatpak apps", || flatpak::install(refs))?;
    }
    // install shared tools before binaries and user configuration
    apply_tools(&config.shared.tools, platform.architecture)?;
    apply_packages(&config.shared.packages)?;
    for package in deb_binaries {
        let subject = format!("{} binary package", package.name);
        run("Installing", &subject, || binary::install(package, platform.architecture))?;
    }
    if !appimages.is_empty() {
        run("Installing", "appimaged", || appimaged::install(platform.architecture))?;
        for package in appimages {
            let subject = format!("{} binary package", package.name);
            run("Installing", &subject, || binary::install(package, platform.architecture))?;
        }
    }
    if let Some(families) = nerd_fonts(&config.shared.fonts) {
        run("Installing", "Nerd Fonts", || fonts::apply(families, false))?;
    }
    if let Some(dotfiles) = dotfiles_packages(&config.shared.dotfiles.packages, &config.linux.dotfiles.packages) {
        run("Applying", "dotfiles", || dotfiles::apply(dotfiles_root, &dotfiles, false))?;
    }
    linux_integrations(&config.linux.integrations)?;
    apply_vscode_extensions(&config.shared.integrations.vscode.extensions)?;
    linux_desktop(theme, config.linux.desktop.as_ref())?;
    Ok(())
}

fn macos_apply(config: &Config, arch: Architecture, dotfiles_root: &Path) -> Result<()> {
    let theme = config.shared.desktop.as_ref().and_then(|desktop| desktop.theme);
    let dotfiles = dotfiles_packages(&config.shared.dotfiles.packages, &config.macos.dotfiles.packages);
    let mut formulae = config.macos.homebrew.formulae.clone();
    if !formulae.iter().any(|formula| formula == "stow") {
        formulae.push("stow".into());
    }

    if config.macos.system.validate_sudo_access == Some(true) {
        run("Validating", "macOS sudo access", macos_host::validate_sudo_access)?;
    }
    if config.macos.system.xcode.command_line_tools == Some(true) {
        run("Installing", "Command Line Tools for Xcode", macos_host::install_command_line_tools_for_xcode)?;
    }
    // install Homebrew and Stow before applying package-backed user configuration
    run("Installing", "Homebrew", homebrew::install)?;
    run("Installing", "Homebrew packages", || homebrew::install_packages(&formulae, &config.macos.homebrew.casks))?;
    apply_tools(&config.shared.tools, arch)?;
    apply_packages(&config.shared.packages)?;
    if let Some(families) = nerd_fonts(&config.shared.fonts) {
        run("Installing", "Nerd Fonts", || fonts::apply(families, false))?;
    }
    if let Some(dotfiles) = dotfiles {
        run("Applying", "dotfiles", || dotfiles::apply(dotfiles_root, &dotfiles, false))?;
    }
    apply_vscode_extensions(&config.shared.integrations.vscode.extensions)?;
    macos_desktop(theme, config.macos.desktop.as_ref())?;
    Ok(())
}

fn apply_tools(tools: &Tools, arch: Architecture) -> Result<()> {
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
    Ok(())
}

fn apply_packages(packages: &SharedPackages) -> Result<()> {
    if let Some(crates) = packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        run("Installing", "Cargo crates", || cargo::install_crates(crates))?;
    }
    if let Some(npm_packages) = packages.npm.as_ref().filter(|values| !values.is_empty()) {
        run("Installing", "npm packages", || npm::install(npm_packages))?;
    }
    Ok(())
}

fn linux_update(config: &Config, architecture: Architecture) -> Result<()> {
    let updates = config.linux.updates.as_ref();
    let flatpak = updates.and_then(|updates| updates.flatpak) == Some(true);
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    if flatpak {
        apt_prereqs.insert("flatpak");
    }

    // refresh package metadata before upgrades and prerequisite reconciliation
    run("Updating", "APT package metadata", apt::update)?;
    if let Some(policy) = updates.and_then(|updates| updates.apt) {
        run("Upgrading", "APT packages", || apt::upgrade(policy))?;
    }
    run("Installing", "APT prerequisites", || {
        apt::install(&apt_prereqs.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>())
    })?;
    if flatpak {
        run("Updating", "Flatpak apps", flatpak::update)?;
    }
    update_tools_and_packages(&config.shared, architecture, false)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(&config.shared.fonts)
    {
        run("Updating", "Nerd Fonts", || fonts::apply(families, true))?;
    }
    Ok(())
}

fn macos_update(config: &Config, arch: Architecture) -> Result<()> {
    let homebrew_formulae = config.macos.updates.homebrew.formulae == Some(true);
    let homebrew_casks = config.macos.updates.homebrew.casks == Some(true);

    run("Installing", "Homebrew", homebrew::install)?;
    if homebrew_formulae || homebrew_casks {
        run("Updating", "Homebrew packages", || homebrew::update_and_upgrade(homebrew_formulae, homebrew_casks))?;
    }
    update_tools_and_packages(&config.shared, arch, true)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(&config.shared.fonts)
    {
        run("Updating", "Nerd Fonts", || fonts::apply(families, true))?;
    }
    Ok(())
}

fn update_tools_and_packages(shared: &SharedConfig, arch: Architecture, macos: bool) -> Result<()> {
    let updates = &shared.updates;
    if updates.tools.rust == Some(true) {
        run("Installing", "Rust toolchain", || rustup::install(shared.tools.rust.as_deref().unwrap_or("stable")))?;
        run("Updating", "Rust toolchains", rustup::update_toolchains)?;
    }
    if updates.tools.go == Some(true) {
        run("Updating", "Go toolchain", || go::update_toolchain(shared.tools.go.as_deref().unwrap_or("latest"), arch))?;
    }
    // macOS resolves npm via Homebrew fnm, so npm-only updates must ensure its formula first
    if updates.tools.node == Some(true) || (macos && updates.packages.npm == Some(true)) {
        run("Installing", "fnm", fnm::install)?;
    }
    if updates.tools.node == Some(true) {
        run("Installing", "Node.js", || fnm::install_version(shared.tools.node.as_deref().unwrap_or("latest")))?;
    }
    if updates.tools.python == Some(true) {
        run("Installing", "uv", uv::install)?;
        run("Upgrading", "Python", uv::upgrade_py)?;
    }
    if updates.packages.cargo == Some(true) {
        run("Updating", "Cargo crates", cargo::update_crates)?;
    }
    if updates.packages.npm == Some(true) {
        run("Updating", "npm packages", npm::update)?;
    }
    Ok(())
}

fn build_apt_repo(repo: &AptRepoConfig, platform: &Platform, key: DistroMapKey, source_uri: &str) -> AptRepo {
    let suite =
        if repo.suite == "codename" { select_repo_codename(key, platform).to_owned() } else { repo.suite.clone() };
    AptRepo::new(
        repo.name.clone(),
        repo.key_url.clone(),
        source_uri.to_owned(),
        platform.architecture,
        suite,
        repo.components.clone(),
        PathBuf::from(&repo.key_path),
    )
}

fn nerd_fonts(fonts: &Fonts) -> Option<&[String]> {
    fonts.nerd.as_deref().filter(|families| !families.is_empty())
}

fn dotfiles_packages(shared_packages: &[String], platform_packages: &[String]) -> Option<Vec<String>> {
    let packages = shared_packages.iter().chain(platform_packages).cloned().collect::<Vec<_>>();
    // return Some(packages) only when at least 1 package is configured
    (!packages.is_empty()).then_some(packages)
}

fn linux_integrations(integrations: &LinuxIntegrations) -> Result<()> {
    if let Some(docker) = &integrations.docker {
        if docker.group == Some(true) {
            run("Configuring", "Docker group membership", || users::ensure_in_group("Docker", "docker", "docker"))?;
        }
        if let Some(logging) = &docker.logging {
            run("Configuring", "Docker local logging driver", || {
                docker::set_local_logging_driver(logging.max_size.as_deref())
            })?;
        }
    }
    if integrations.virtualbox.as_ref().is_some_and(|virtualbox| virtualbox.group == Some(true)) {
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

fn add_desktop_prereqs(theme: Option<Theme>, desktop: Option<&LinuxDesktop>, apt_prereqs: &mut BTreeSet<&'static str>) {
    if theme.is_none() && !desktop.is_some_and(LinuxDesktop::has_intent) {
        return;
    }
    apt_prereqs.extend(["dconf-cli", "libglib2.0-bin"]);
    if desktop.and_then(|desktop| desktop.gnome.as_ref()).is_some_and(Gnome::has_intent) {
        apt_prereqs.insert("gnome-shell");
    }
}

fn linux_desktop(theme: Option<Theme>, desktop: Option<&LinuxDesktop>) -> Result<()> {
    if let Some(theme) = theme {
        run("Setting", "desktop color scheme", || desktop::set_color_scheme(theme))?;
    }
    let Some(desktop) = desktop.filter(|desktop| desktop.has_intent()) else { return Ok(()) };
    if let Some(executable) = &desktop.terminal {
        run("Setting", "default terminal", || desktop::set_terminal(executable))?;
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = idle.timeout {
            run("Setting", "idle timeout", || desktop::set_idle_delay(timeout.seconds()))?;
        }
        if let Some(enabled) = idle.dim {
            run("Setting", "idle dimming", || desktop::set_idle_dim(enabled))?;
        }
    }
    if let Some(gnome) = &desktop.gnome {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            run_with_outcome("Applying", "GNOME extensions", || gnome::apply_extensions(extensions))?;
        }
        if gnome.dash_to_dock == Some(true) {
            run_with_outcome("Installing", "Dash to Dock", gnome::apply_dash_to_dock)?;
        }
        if gnome.rounded_window_corners == Some(true) {
            run_with_outcome("Installing", "Rounded Window Corners", gnome::apply_rounded_window_corners)?;
        }
    }
    Ok(())
}

fn macos_desktop(theme: Option<Theme>, desktop: Option<&MacDesktop>) -> Result<()> {
    if theme.is_some() || desktop.is_some_and(MacDesktop::has_intent) {
        run("Writing", "macOS defaults", || macos_defaults::write_defaults(theme, desktop))?;
    }
    Ok(())
}

fn run(status: &str, subject: &str, operation: impl FnOnce() -> Result<()>) -> Result<()> {
    eprintln!("{status:>12} {subject}");
    operation().with_context(|| format!("{} {subject}", status.to_lowercase()))
}

fn run_with_outcome(status: &str, subject: &str, operation: impl FnOnce() -> Result<gnome::Outcome>) -> Result<()> {
    eprintln!("{status:>12} {subject}");
    let action = status.to_lowercase();
    if operation().with_context(|| format!("{action} {subject}"))? == gnome::Outcome::LoginRequired {
        eprintln!("note: log out and back in to finish {action} {subject}");
    }
    Ok(())
}
