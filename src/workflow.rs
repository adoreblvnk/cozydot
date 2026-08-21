//! Derive prerequisites & run each platform's operations in dependency order.

use crate::{
    config::{
        AptArchitecture, AptRepoConfig, BinaryFormat, BinarySource, Config, DistroMapKey, Enablement, Fonts, Gnome,
        LinuxDesktop, LinuxIntegrations, MacDesktop, SharedConfig, SharedPackages, Tools, select_distro_entry,
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
    platform::{Architecture, DesktopKind, Distro, Platform, PlatformIdentity},
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
        run("Apply", "dotfiles apply", || dotfiles::apply(root, &dotfiles, replace))?;
    }
    Ok(())
}

pub fn update(config: &Config, platform: &Platform) -> Result<()> {
    host::home()?;
    match platform.identity {
        PlatformIdentity::Macos => macos_update(config, platform.architecture),
        PlatformIdentity::Linux { .. } => linux_update(config, platform),
    }
}

fn linux_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    let PlatformIdentity::Linux { distro, .. } = platform.identity else { unreachable!() };
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
            repos.push(build_apt_repo(repo, platform, key, source_uri));
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
    add_desktop_prereqs(config.linux.desktop.as_ref(), platform.desktop, &mut apt_prereqs);

    // establish distro services and package prerequisites before third-party repositories
    if distro == Distro::Debian {
        if config.linux.system.sudo_group == Some(true) {
            run("Apply", "sudo group membership", users::ensure_in_sudo_group)?;
        }
        run("Apply", "Debian APT component add", repo::debian_components::add)?;
    }
    if distro == Distro::Ubuntu
        && let Some(ubuntu) = &config.linux.system.ubuntu
    {
        if let Some(state) = ubuntu.unattended_upgrades {
            run("Apply", "unattended-upgrades set", || apt::set_unattended_upgrades(state == Enablement::Enabled))?;
        }
        if let Some(state) = ubuntu.snapd {
            run("Apply", "snapd set", || snapd::set_enabled(state == Enablement::Enabled))?;
        }
        if ubuntu.restricted_extras {
            run("Apply", "APT package install", || apt::install(&["ubuntu-restricted-extras".into()]))?;
        }
    }
    run("Apply", "APT update", apt::update)?;
    run("Apply", "APT package install", || {
        apt::install(&apt_prereqs.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>())
    })?;
    if let Some(apt) = &config.linux.packages.apt
        && let Some(packages) = &apt.install
        && !packages.is_empty()
    {
        run("Apply", "APT package install", || apt::install(packages))?;
    }
    // add repositories before changing packages supplied by them
    if !repos.is_empty() {
        for apt_repo in repos {
            run("Apply", "APT repo add", || repo::add(&apt_repo))?;
        }
        run("Apply", "APT update", apt::update)?;
    }
    if !repo_packages_to_purge.is_empty() {
        run("Apply", "APT package purge", || apt::purge(&repo_packages_to_purge))?;
    }
    if !repo_packages_to_install.is_empty() {
        run("Apply", "APT package install", || apt::install(&repo_packages_to_install))?;
    }
    if let Some(refs) = flatpak_refs {
        run("Apply", "Flathub remote add", flatpak::add_flathub_remote)?;
        run("Apply", "Flatpak app install", || flatpak::install(refs))?;
    }
    // install shared tools before binaries and user configuration
    apply_tools(&config.shared.tools, platform.architecture)?;
    apply_packages(&config.shared.packages)?;
    for package in deb_binaries {
        run("Apply", "binary package install", || binary::install(package, platform.architecture))?;
    }
    if !appimages.is_empty() {
        run("Apply", "appimaged install", || appimaged::install(platform.architecture))?;
        for package in appimages {
            run("Apply", "binary package install", || binary::install(package, platform.architecture))?;
        }
    }
    if let Some(families) = nerd_fonts(&config.shared.fonts) {
        run("Apply", "Nerd Fonts install", || fonts::apply(families, false))?;
    }
    if let Some(dotfiles) = dotfiles_packages(&config.shared.dotfiles.packages, &config.linux.dotfiles.packages) {
        run("Apply", "dotfiles apply", || dotfiles::apply(dotfiles_root, &dotfiles, false))?;
    }
    linux_integrations(&config.linux.integrations)?;
    apply_vscode_extensions(&config.shared.integrations.vscode.extensions)?;
    linux_desktop(config.linux.desktop.as_ref(), platform.desktop)?;
    Ok(())
}

fn macos_apply(config: &Config, arch: Architecture, dotfiles_root: &Path) -> Result<()> {
    let dotfiles = dotfiles_packages(&config.shared.dotfiles.packages, &config.macos.dotfiles.packages);
    let mut formulae = config.macos.homebrew.formulae.clone();
    if !formulae.iter().any(|formula| formula == "stow") {
        formulae.push("stow".into());
    }

    if config.macos.system.validate_sudo_access == Some(true) {
        run("Apply", "macOS sudo access validation", macos_host::validate_sudo_access)?;
    }
    if config.macos.system.xcode.command_line_tools == Some(true) {
        run("Apply", "Command Line Tools for Xcode install", macos_host::install_command_line_tools_for_xcode)?;
    }
    // install Homebrew and Stow before applying package-backed user configuration
    run("Apply", "Homebrew install", homebrew::install)?;
    run("Apply", "Homebrew package install", || homebrew::install_packages(&formulae, &config.macos.homebrew.casks))?;
    apply_tools(&config.shared.tools, arch)?;
    apply_packages(&config.shared.packages)?;
    if let Some(families) = nerd_fonts(&config.shared.fonts) {
        run("Apply", "Nerd Fonts install", || fonts::apply(families, false))?;
    }
    if let Some(dotfiles) = dotfiles {
        run("Apply", "dotfiles apply", || dotfiles::apply(dotfiles_root, &dotfiles, false))?;
    }
    apply_vscode_extensions(&config.shared.integrations.vscode.extensions)?;
    macos_desktop(&config.macos.desktop)?;
    Ok(())
}

fn apply_tools(tools: &Tools, arch: Architecture) -> Result<()> {
    if let Some(selector) = tools.rust.as_deref() {
        run("Apply", "Rust install", || rustup::install(selector))?;
        run("Apply", "cargo-binstall install", cargo::install_binstall)?;
        run("Apply", "cargo-update install", || cargo::install_crates(&["cargo-update".to_owned()]))?;
    }
    if let Some(selector) = tools.node.as_deref() {
        run("Apply", "fnm install", fnm::install)?;
        run("Apply", "Node.js version install", || fnm::install_version(selector))?;
    }
    if let Some(selector) = &tools.python {
        run("Apply", "uv install", uv::install)?;
        run("Apply", "Python version install", || uv::install_py(selector))?;
    }
    if let Some(selector) = tools.go.as_deref() {
        run("Apply", "Go toolchain install", || go::install_toolchain(selector, arch))?;
    }
    Ok(())
}

fn apply_packages(packages: &SharedPackages) -> Result<()> {
    if let Some(crates) = packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        run("Apply", "Cargo crate install", || cargo::install_crates(crates))?;
    }
    if let Some(npm_packages) = packages.npm.as_ref().filter(|values| !values.is_empty()) {
        run("Apply", "npm package install", || npm::install(npm_packages))?;
    }
    Ok(())
}

fn linux_update(config: &Config, platform: &Platform) -> Result<()> {
    let updates = config.linux.updates.as_ref();
    let flatpak = updates.and_then(|updates| updates.flatpak) == Some(true);
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    if flatpak {
        apt_prereqs.insert("flatpak");
    }

    // refresh package metadata before upgrades and prerequisite reconciliation
    run("Update", "APT update", apt::update)?;
    if let Some(policy) = updates.and_then(|updates| updates.apt) {
        run("Update", "APT upgrade", || apt::upgrade(policy))?;
    }
    run("Update", "APT package install", || {
        apt::install(&apt_prereqs.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>())
    })?;
    if flatpak {
        run("Update", "Flatpak update", flatpak::update)?;
    }
    update_tools_and_packages(&config.shared, platform.architecture, false)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(&config.shared.fonts)
    {
        run("Update", "Nerd Fonts update", || fonts::apply(families, true))?;
    }
    Ok(())
}

fn macos_update(config: &Config, arch: Architecture) -> Result<()> {
    let homebrew_formulae = config.macos.updates.homebrew.formulae == Some(true);
    let homebrew_casks = config.macos.updates.homebrew.casks == Some(true);

    run("Update", "Homebrew install", homebrew::install)?;
    if homebrew_formulae || homebrew_casks {
        run("Update", "Homebrew update and upgrade", || {
            homebrew::update_and_upgrade(homebrew_formulae, homebrew_casks)
        })?;
    }
    update_tools_and_packages(&config.shared, arch, true)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(&config.shared.fonts)
    {
        run("Update", "Nerd Fonts update", || fonts::apply(families, true))?;
    }
    Ok(())
}

fn update_tools_and_packages(shared: &SharedConfig, arch: Architecture, macos: bool) -> Result<()> {
    let updates = &shared.updates;
    if updates.tools.rust == Some(true) {
        run("Update", "Rust install", || rustup::install(shared.tools.rust.as_deref().unwrap_or("stable")))?;
        run("Update", "Rust toolchain update", rustup::update_toolchains)?;
    }
    if updates.tools.go == Some(true) {
        run("Update", "Go toolchain update", || {
            go::update_toolchain(shared.tools.go.as_deref().unwrap_or("latest"), arch)
        })?;
    }
    // macOS resolves npm via Homebrew fnm, so npm-only updates must ensure its formula first
    if updates.tools.node == Some(true) || (macos && updates.packages.npm == Some(true)) {
        run("Update", "fnm install", fnm::install)?;
    }
    if updates.tools.node == Some(true) {
        run("Update", "Node.js version install", || {
            fnm::install_version(shared.tools.node.as_deref().unwrap_or("latest"))
        })?;
    }
    if updates.tools.python == Some(true) {
        run("Update", "uv install", uv::install)?;
        run("Update", "Python version upgrade", uv::upgrade_py)?;
    }
    if updates.packages.cargo == Some(true) {
        run("Update", "Cargo crate update", cargo::update_crates)?;
    }
    if updates.packages.npm == Some(true) {
        run("Update", "npm package update", npm::update)?;
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
            run("Apply", "Docker group membership", || users::ensure_in_group("Docker", "docker", "docker"))?;
        }
        if let Some(logging) = &docker.logging {
            run("Apply", "Docker local logging driver set", || {
                docker::set_local_logging_driver(logging.max_size.as_deref())
            })?;
        }
    }
    if integrations.virtualbox.as_ref().is_some_and(|virtualbox| virtualbox.group == Some(true)) {
        run("Apply", "VirtualBox group membership", || {
            users::ensure_in_group("VirtualBox", "VBoxManage", "vboxusers")
        })?;
    }
    Ok(())
}

fn apply_vscode_extensions(extensions: &[String]) -> Result<()> {
    if !extensions.is_empty() {
        run("Apply", "Visual Studio Code extension install", || vscode::install_extensions(extensions))?;
    }
    Ok(())
}

fn add_desktop_prereqs(
    desktop: Option<&LinuxDesktop>,
    desktop_kind: DesktopKind,
    apt_prereqs: &mut BTreeSet<&'static str>,
) {
    let Some(desktop) = desktop.filter(|desktop| desktop.has_intent()) else { return };
    apt_prereqs.extend(["dconf-cli", "libglib2.0-bin"]);
    if desktop_kind == DesktopKind::Gnome && desktop.gnome.as_ref().is_some_and(Gnome::has_intent) {
        apt_prereqs.insert("gnome-shell");
    }
}

fn linux_desktop(desktop: Option<&LinuxDesktop>, desktop_kind: DesktopKind) -> Result<()> {
    let Some(desktop) = desktop.filter(|desktop| desktop.has_intent()) else { return Ok(()) };
    if desktop_kind != DesktopKind::Gnome {
        return Ok(());
    }
    if let Some(theme) = desktop.theme {
        run("Apply", "desktop setting set", || desktop::set_color_scheme(theme))?;
    }
    if let Some(executable) = &desktop.terminal {
        run("Apply", "desktop setting set", || desktop::set_terminal(executable))?;
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = idle.timeout {
            run("Apply", "desktop setting set", || desktop::set_idle_delay(timeout.seconds()))?;
        }
        if let Some(enabled) = idle.dim {
            run("Apply", "desktop setting set", || desktop::set_idle_dim(enabled))?;
        }
    }
    if let Some(gnome) = &desktop.gnome {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            run_with_outcome("Apply", "GNOME extension apply", || gnome::apply_extensions(extensions))?;
        }
        if gnome.dash_to_dock == Some(true) {
            run_with_outcome("Apply", "Dash to Dock install", gnome::apply_dash_to_dock)?;
        }
        if gnome.rounded_window_corners == Some(true) {
            run_with_outcome("Apply", "Rounded Window Corners install", gnome::apply_rounded_window_corners)?;
        }
    }
    Ok(())
}

fn macos_desktop(desktop: &MacDesktop) -> Result<()> {
    if desktop.has_intent() {
        run("Apply", "macOS defaults write", || macos_defaults::write_defaults(desktop))?;
    }
    Ok(())
}

fn run(progress: &str, label: &str, operation: impl FnOnce() -> Result<()>) -> Result<()> {
    println!("{progress}: {label}");
    operation().with_context(|| format!("{}: {label}", progress.to_lowercase()))
}

fn run_with_outcome(progress: &str, label: &str, operation: impl FnOnce() -> Result<gnome::Outcome>) -> Result<()> {
    println!("{progress}: {label}");
    if operation().with_context(|| format!("{}: {label}", progress.to_lowercase()))? == gnome::Outcome::LoginRequired {
        println!("Login required to finish {label}");
    }
    Ok(())
}
