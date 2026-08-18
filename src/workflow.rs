//! Derive prerequisites & run each platform's operations in dependency order.

use crate::{
    config::{
        AptArchitecture, BinaryFormat, BinarySource, Config, DistroMapKey, EnabledDisabled, Gnome, Repo,
        select_distro_map, selected_repo_codename,
    },
    operations::{
        desktop::{self, DesktopEnvironment, fonts, gnome, macos as macos_defaults},
        dotfiles,
        host::{Host, macos as macos_host, users},
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
    platform::{Architecture, DesktopKind, Distro, Family, Platform, PlatformIdentity},
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const APT_PREREQS: [&str; 7] = ["ca-certificates", "curl", "fontconfig", "gnupg", "stow", "unzip", "xz-utils"];

pub fn apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    let host = Host::new()?;
    match platform.identity {
        PlatformIdentity::MacOS => macos_apply(&host, config, platform.architecture, dotfiles_root),
        PlatformIdentity::Linux { distro, family } => {
            linux_apply(&host, config, platform, distro, family, dotfiles_root)
        }
    }
}

pub fn dotfiles(config: &Config, platform: &Platform, root: &Path, replace: bool) -> Result<()> {
    let platform_packages = match platform.identity {
        PlatformIdentity::MacOS => &config.macos.dotfiles.packages,
        PlatformIdentity::Linux { .. } => &config.linux.dotfiles.packages,
    };
    if let Some(dotfiles) = dotfiles_packages(config, platform_packages) {
        let host = Host::new()?;
        run("Apply", "dotfiles apply", || dotfiles::apply(&host, root, &dotfiles, replace))?;
    }
    Ok(())
}

pub fn update(config: &Config, platform: &Platform) -> Result<()> {
    let host = Host::new()?;
    match platform.identity {
        PlatformIdentity::MacOS => macos_update(&host, config, platform.architecture),
        PlatformIdentity::Linux { .. } => linux_update(&host, config, platform),
    }
}

fn linux_apply(
    host: &Host,
    config: &Config,
    platform: &Platform,
    distro: Distro,
    family: Family,
    dotfiles_root: &Path,
) -> Result<()> {
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    let mut repos = Vec::new();
    let mut repo_packages_to_purge = Vec::new();
    let mut repo_packages_to_install = Vec::new();
    for (repo, key, source_url) in applicable_repos(config, distro, family, platform.architecture) {
        repos.push(add_repo(repo, platform, distro, key, source_url));
        repo_packages_to_purge.extend(repo.conflicts.iter().cloned());
        repo_packages_to_install.extend(repo.packages.iter().cloned());
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
    add_desktop_prereqs(config, platform, &mut apt_prereqs);

    // establish distro services and package prerequisites before third-party repositories
    if distro == Distro::Debian {
        if config.linux.system.sudo_group == Some(true) {
            run("Apply", "sudo group membership", || users::ensure_in_sudo_group(host))?;
        }
        run("Apply", "Debian APT component add", || repo::debian_components::add(host))?;
    }
    if distro == Distro::Ubuntu
        && let Some(ubuntu) = &config.linux.system.ubuntu
    {
        if let Some(state) = ubuntu.unattended_upgrades {
            run("Apply", "unattended-upgrades set", || {
                apt::set_unattended_upgrades(host, state == EnabledDisabled::Enabled)
            })?;
        }
        if let Some(state) = ubuntu.snapd {
            run("Apply", "snapd set", || snapd::set_enabled(host, state == EnabledDisabled::Enabled))?;
        }
        if ubuntu.restricted_extras {
            run("Apply", "APT package install", || apt::install(host, &["ubuntu-restricted-extras".into()]))?;
        }
    }
    run("Apply", "APT update", || apt::update(host))?;
    run("Apply", "APT package install", || {
        apt::install(host, &apt_prereqs.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>())
    })?;
    if let Some(packages) =
        config.linux.packages.apt.as_ref().and_then(|apt| apt.install.as_ref()).filter(|packages| !packages.is_empty())
    {
        run("Apply", "APT package install", || apt::install(host, packages))?;
    }
    // add repositories before changing packages supplied by them
    if !repos.is_empty() {
        for apt_repo in repos {
            run("Apply", "APT repo add", || repo::add(host, &apt_repo))?;
        }
        run("Apply", "APT update", || apt::update(host))?;
    }
    if !repo_packages_to_purge.is_empty() {
        run("Apply", "APT package purge", || apt::purge(host, &repo_packages_to_purge))?;
    }
    if !repo_packages_to_install.is_empty() {
        run("Apply", "APT package install", || apt::install(host, &repo_packages_to_install))?;
    }
    if let Some(refs) = flatpak_refs {
        run("Apply", "Flathub remote add", || flatpak::add_flathub_remote(host))?;
        run("Apply", "Flatpak app install", || flatpak::install(host, refs))?;
    }
    // install shared tools before binaries and user configuration
    apply_tools(host, config, platform.architecture)?;
    apply_packages(host, config)?;
    for package in deb_binaries {
        run("Apply", "binary package install", || binary::install(host, package, platform.architecture))?;
    }
    if !appimages.is_empty() {
        run("Apply", "appimaged install", || appimaged::install(host, platform.architecture))?;
        for package in appimages {
            run("Apply", "binary package install", || binary::install(host, package, platform.architecture))?;
        }
    }
    if let Some(families) = nerd_fonts(config) {
        run("Apply", "Nerd Fonts install", || fonts::apply(host, families, false))?;
    }
    if let Some(dotfiles) = dotfiles_packages(config, &config.linux.dotfiles.packages) {
        run("Apply", "dotfiles apply", || dotfiles::apply(host, dotfiles_root, &dotfiles, false))?;
    }
    linux_integrations(host, config)?;
    linux_desktop(host, config, platform)?;
    Ok(())
}

fn macos_apply(host: &Host, config: &Config, arch: Architecture, dotfiles_root: &Path) -> Result<()> {
    let dotfiles = dotfiles_packages(config, &config.macos.dotfiles.packages);
    let homebrew_packages =
        dotfiles.is_some() || !config.macos.homebrew.formulae.is_empty() || !config.macos.homebrew.casks.is_empty();

    if config.macos.system.validate_sudo_access == Some(true) {
        run("Apply", "macOS sudo access validation", || macos_host::validate_sudo_access(host))?;
    }
    if config.macos.system.xcode.command_line_tools == Some(true) {
        run("Apply", "Command Line Tools for Xcode install", || {
            macos_host::install_command_line_tools_for_xcode(host)
        })?;
    }
    // install Homebrew and Stow before applying package-backed user configuration
    run("Apply", "Homebrew install", || homebrew::install(host))?;
    if homebrew_packages {
        let mut formulae = config.macos.homebrew.formulae.clone();
        if dotfiles.is_some() && !formulae.iter().any(|formula| formula == "stow") {
            formulae.push("stow".into());
        }
        run("Apply", "Homebrew package install", || {
            homebrew::install_packages(host, &formulae, &config.macos.homebrew.casks)
        })?;
    }
    apply_tools(host, config, arch)?;
    apply_packages(host, config)?;
    if let Some(families) = nerd_fonts(config) {
        run("Apply", "Nerd Fonts install", || fonts::apply(host, families, false))?;
    }
    if let Some(dotfiles) = dotfiles {
        run("Apply", "dotfiles apply", || dotfiles::apply(host, dotfiles_root, &dotfiles, false))?;
    }
    vscode_extensions(host, config)?;
    macos_desktop(host, config)?;
    Ok(())
}

fn apply_tools(host: &Host, config: &Config, arch: Architecture) -> Result<()> {
    if let Some(selector) = config.shared.tools.rust.as_deref() {
        run("Apply", "Rust install", || rustup::install(host, selector))?;
        run("Apply", "cargo-binstall install", || cargo::install_binstall(host))?;
        run("Apply", "cargo-update install", || cargo::install_cargo_update(host))?;
    }
    if let Some(selector) = config.shared.tools.node.as_deref() {
        run("Apply", "fnm install", || fnm::install(host))?;
        run("Apply", "Node.js version install", || fnm::install_version(host, selector))?;
    }
    if let Some(selector) = &config.shared.tools.python {
        run("Apply", "uv install", || uv::install(host))?;
        run("Apply", "Python version install", || uv::install_py(host, selector))?;
    }
    if let Some(selector) = config.shared.tools.go.as_deref() {
        run("Apply", "Go toolchain install", || go::install_toolchain(host, selector, arch))?;
    }
    Ok(())
}

fn apply_packages(host: &Host, config: &Config) -> Result<()> {
    if let Some(crates) = config.shared.packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        run("Apply", "Cargo crate install", || cargo::install_crates(host, crates))?;
    }
    if let Some(npm_packages) = config.shared.packages.npm.as_ref().filter(|values| !values.is_empty()) {
        run("Apply", "npm package install", || npm::install(host, npm_packages))?;
    }
    Ok(())
}

fn linux_update(host: &Host, config: &Config, platform: &Platform) -> Result<()> {
    let updates = config.linux.updates.as_ref();
    let flatpak = updates.and_then(|updates| updates.flatpak) == Some(true);
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    if flatpak {
        apt_prereqs.insert("flatpak");
    }

    // refresh package metadata before upgrades and prerequisite reconciliation
    run("Update", "APT update", || apt::update(host))?;
    if let Some(policy) = updates.and_then(|updates| updates.apt) {
        run("Update", "APT upgrade", || apt::upgrade(host, policy))?;
    }
    run("Update", "APT package install", || {
        apt::install(host, &apt_prereqs.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>())
    })?;
    if flatpak {
        run("Update", "Flatpak update", || flatpak::update(host))?;
    }
    update_tools_and_packages(host, config, platform.architecture, false)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(config)
    {
        run("Update", "Nerd Fonts update", || fonts::apply(host, families, true))?;
    }
    Ok(())
}

fn macos_update(host: &Host, config: &Config, arch: Architecture) -> Result<()> {
    let homebrew_formulae = config.macos.updates.homebrew.formulae == Some(true);
    let homebrew_casks = config.macos.updates.homebrew.casks == Some(true);

    run("Update", "Homebrew install", || homebrew::install(host))?;
    if homebrew_formulae || homebrew_casks {
        run("Update", "Homebrew update and upgrade", || {
            homebrew::update_and_upgrade(host, homebrew_formulae, homebrew_casks)
        })?;
    }
    update_tools_and_packages(host, config, arch, true)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(config)
    {
        run("Update", "Nerd Fonts update", || fonts::apply(host, families, true))?;
    }
    Ok(())
}

fn update_tools_and_packages(host: &Host, config: &Config, arch: Architecture, macos: bool) -> Result<()> {
    let updates = &config.shared.updates;
    if updates.tools.rust == Some(true) {
        run("Update", "Rust install", || {
            rustup::install(host, config.shared.tools.rust.as_deref().unwrap_or("stable"))
        })?;
        run("Update", "Rust toolchain update", || rustup::update_toolchains(host))?;
    }
    if updates.tools.go == Some(true) {
        run("Update", "Go toolchain update", || {
            go::update_toolchain(host, config.shared.tools.go.as_deref().unwrap_or("latest"), arch)
        })?;
    }
    // macOS resolves npm via Homebrew fnm, so npm-only updates must ensure its formula first
    if updates.tools.node == Some(true) || (macos && updates.packages.npm == Some(true)) {
        run("Update", "fnm install", || fnm::install(host))?;
    }
    if updates.tools.node == Some(true) {
        run("Update", "Node.js version install", || {
            fnm::install_version(host, config.shared.tools.node.as_deref().unwrap_or("latest"))
        })?;
    }
    if updates.tools.python == Some(true) {
        run("Update", "uv install", || uv::install(host))?;
        run("Update", "Python version upgrade", || uv::upgrade_py_versions(host))?;
    }
    if updates.packages.cargo == Some(true) {
        run("Update", "Cargo crate update", || cargo::update_crates(host))?;
    }
    if updates.packages.npm == Some(true) {
        run("Update", "npm package update", || npm::update(host))?;
    }
    Ok(())
}

fn applicable_repos(
    config: &Config,
    distro: Distro,
    family: Family,
    architecture: Architecture,
) -> impl Iterator<Item = (&Repo, DistroMapKey, &String)> {
    config
        .linux
        .packages
        .apt
        .as_ref()
        .and_then(|apt| apt.repos.as_deref())
        .unwrap_or_default()
        .iter()
        .filter(move |repo| {
            repo.arch.as_ref().is_none_or(|values| {
                values.iter().any(|value| {
                    matches!(
                        (value, architecture),
                        (AptArchitecture::Amd64, Architecture::X86_64)
                            | (AptArchitecture::Arm64, Architecture::Aarch64)
                            | (AptArchitecture::Armhf, Architecture::Arm)
                    )
                })
            })
        })
        .filter_map(move |repo| select_distro_map(&repo.urls, distro, family).map(|(key, url)| (repo, key, url)))
}

fn add_repo(repo: &Repo, platform: &Platform, distro: Distro, key: DistroMapKey, source_url: &str) -> AptRepo {
    let suite = if repo.suite == "codename" {
        selected_repo_codename(key, platform, distro).to_owned()
    } else {
        repo.suite.clone()
    };
    AptRepo::new(
        repo.name.clone(),
        repo.key_url.clone(),
        source_url.to_owned(),
        platform.architecture,
        suite,
        repo.components.clone(),
        PathBuf::from(&repo.key_path),
    )
}

fn nerd_fonts(config: &Config) -> Option<&[String]> {
    config.shared.fonts.nerd.as_deref().filter(|families| !families.is_empty())
}

fn dotfiles_packages(config: &Config, platform_packages: &[String]) -> Option<Vec<String>> {
    let packages = config.shared.dotfiles.packages.iter().chain(platform_packages).cloned().collect::<Vec<_>>();
    (!packages.is_empty()).then_some(packages)
}

fn linux_integrations(host: &Host, config: &Config) -> Result<()> {
    if let Some(docker) = &config.linux.integrations.docker {
        if docker.group == Some(true) {
            run("Apply", "Docker group membership", || users::ensure_in_group(host, "Docker", "docker", "docker"))?;
        }
        if let Some(logging) = &docker.logging {
            run("Apply", "Docker local logging driver set", || {
                docker::set_local_logging_driver(host, logging.max_size.as_deref())
            })?;
        }
    }
    if config.linux.integrations.virtualbox.as_ref().is_some_and(|virtualbox| virtualbox.group == Some(true)) {
        run("Apply", "VirtualBox group membership", || {
            users::ensure_in_group(host, "VirtualBox", "VBoxManage", "vboxusers")
        })?;
    }
    vscode_extensions(host, config)
}

fn vscode_extensions(host: &Host, config: &Config) -> Result<()> {
    if !config.shared.integrations.vscode.extensions.is_empty() {
        run("Apply", "Visual Studio Code extension install", || {
            vscode::install_extensions(host, &config.shared.integrations.vscode.extensions)
        })?;
    }
    Ok(())
}

fn add_desktop_prereqs(config: &Config, platform: &Platform, apt_prereqs: &mut BTreeSet<&'static str>) {
    let Some(desktop) = config.linux.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return };
    apt_prereqs.extend(["dconf-cli", "libglib2.0-bin"]);
    if platform.desktop == DesktopKind::Gnome && desktop.gnome.as_ref().is_some_and(Gnome::has_intent) {
        apt_prereqs.insert("gnome-shell");
    }
}

fn linux_desktop(host: &Host, config: &Config, platform: &Platform) -> Result<()> {
    let Some(desktop) = config.linux.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return Ok(()) };
    let Some(environment) = (match platform.desktop {
        DesktopKind::Gnome => Some(DesktopEnvironment::Gnome),
        DesktopKind::Cinnamon => Some(DesktopEnvironment::Cinnamon),
        DesktopKind::None => None,
    }) else {
        return Ok(());
    };
    if let Some(theme) = desktop.theme {
        run("Apply", "desktop setting set", || desktop::set_color_scheme(host, environment, theme))?;
    }
    if let Some(executable) = &desktop.terminal {
        run("Apply", "desktop setting set", || desktop::set_terminal(host, environment, executable))?;
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = idle.timeout {
            run("Apply", "desktop setting set", || desktop::set_idle_delay(host, environment, timeout.seconds()))?;
        }
        if let Some(enabled) = idle.dim {
            run("Apply", "desktop setting set", || desktop::set_idle_dim(host, environment, enabled))?;
        }
    }
    if environment == DesktopEnvironment::Gnome
        && let Some(gnome) = &desktop.gnome
    {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            run_with_outcome("Apply", "GNOME extension apply", || gnome::apply_extensions(host, extensions))?;
        }
        if gnome.dash_to_dock == Some(true) {
            run_with_outcome("Apply", "Dash to Dock install", || gnome::install_dash_to_dock(host))?;
        }
        if gnome.rounded_window_corners == Some(true) {
            run_with_outcome("Apply", "Rounded Window Corners install", || {
                gnome::install_rounded_window_corners(host)
            })?;
        }
    }
    Ok(())
}

fn macos_desktop(host: &Host, config: &Config) -> Result<()> {
    let desktop = &config.macos.desktop;
    if desktop.has_intent() {
        run("Apply", "macOS defaults write", || macos_defaults::write_defaults(host, desktop))?;
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
