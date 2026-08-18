//! Derive prerequisites & run each platform's operations in dependency order.

use crate::{
    config::{
        AptArchitecture, BinaryFormat, BinarySource, Config, DistroMapKey, EnabledDisabled, Gnome, Repo, Theme,
        select_distro_map, selected_repo_codename,
    },
    operations::{
        self, AptRepo, BinaryPackageOperation, BinarySourceOperation, DesktopEnvironment, DesktopSetting,
        GoToolchainSelector, Operation,
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
    match platform.identity {
        PlatformIdentity::MacOS => macos_apply(config, platform.architecture, dotfiles_root),
        PlatformIdentity::Linux { distro, family } => linux_apply(config, platform, distro, family, dotfiles_root),
    }
}

pub fn update(config: &Config, platform: &Platform) -> Result<()> {
    match platform.identity {
        PlatformIdentity::MacOS => macos_update(config, platform.architecture),
        PlatformIdentity::Linux { .. } => linux_update(config, platform),
    }
}

pub fn dotfiles(config: &Config, platform: &Platform, root: &Path, replace: bool) -> Result<()> {
    let platform_packages = match platform.identity {
        PlatformIdentity::MacOS => &config.macos.dotfiles.packages,
        PlatformIdentity::Linux { .. } => &config.linux.dotfiles.packages,
    };
    if let Some(operation) = dotfiles_operation(config, platform_packages, root, replace) {
        run("Apply", "dotfiles apply", operation)?;
    }
    Ok(())
}

fn linux_apply(
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
    let flatpak_refs = config.linux.packages.flatpak.as_ref().filter(|values| !values.is_empty()).cloned();
    if flatpak_refs.is_some() {
        apt_prereqs.insert("flatpak");
    }
    let mut deb_binaries = Vec::new();
    let mut appimages = Vec::new();
    for binary in config.linux.packages.binaries.as_deref().unwrap_or_default() {
        if let Some(operation) = binary_operation(binary, platform.architecture) {
            match binary.format {
                BinaryFormat::Deb => deb_binaries.push(operation),
                BinaryFormat::AppImage => appimages.push(operation),
            }
        }
    }
    add_desktop_prereqs(config, platform, &mut apt_prereqs);

    if distro == Distro::Debian {
        if config.linux.system.sudo_group == Some(true) {
            run("Apply", "sudo group membership", Operation::SudoGroupEnsure)?;
        }
        run("Apply", "Debian APT component add", Operation::DebianAptComponentsAdd)?;
    }
    if distro == Distro::Ubuntu
        && let Some(ubuntu) = &config.linux.system.ubuntu
    {
        if let Some(state) = ubuntu.unattended_upgrades {
            run(
                "Apply",
                "unattended-upgrades set",
                Operation::UnattendedUpgradesSet { enabled: state == EnabledDisabled::Enabled },
            )?;
        }
        if let Some(state) = ubuntu.snapd {
            run("Apply", "snapd set", Operation::SnapdSet { enabled: state == EnabledDisabled::Enabled })?;
        }
        if ubuntu.restricted_extras {
            run(
                "Apply",
                "APT package install",
                Operation::AptPackagesInstall { packages: vec!["ubuntu-restricted-extras".into()] },
            )?;
        }
    }
    run("Apply", "APT update", Operation::AptUpdate)?;
    run(
        "Apply",
        "APT package install",
        Operation::AptPackagesInstall { packages: apt_prereqs.iter().map(|value| (*value).to_owned()).collect() },
    )?;
    if let Some(packages) =
        config.linux.packages.apt.as_ref().and_then(|apt| apt.install.as_ref()).filter(|packages| !packages.is_empty())
    {
        run("Apply", "APT package install", Operation::AptPackagesInstall { packages: packages.clone() })?;
    }
    if !repos.is_empty() {
        for repo in repos {
            run("Apply", "APT repo add", Operation::AptRepoAdd(Box::new(repo)))?;
        }
        run("Apply", "APT update", Operation::AptUpdate)?;
    }
    if !repo_packages_to_purge.is_empty() {
        run("Apply", "APT package purge", Operation::AptPackagesPurge { packages: repo_packages_to_purge })?;
    }
    if !repo_packages_to_install.is_empty() {
        run("Apply", "APT package install", Operation::AptPackagesInstall { packages: repo_packages_to_install })?;
    }
    if let Some(refs) = flatpak_refs {
        run("Apply", "Flathub remote add", Operation::FlatpakFlathubRemoteAdd)?;
        run("Apply", "Flatpak app install", Operation::FlatpakAppsInstall { refs })?;
    }
    apply_tools(config, platform.architecture)?;
    apply_packages(config)?;
    for binary in deb_binaries {
        run("Apply", "binary package install", Operation::BinaryPackageInstall(binary))?;
    }
    if !appimages.is_empty() {
        run("Apply", "appimaged install", Operation::AppimagedInstall { architecture: platform.architecture })?;
        for binary in appimages {
            run("Apply", "binary package install", Operation::BinaryPackageInstall(binary))?;
        }
    }
    if let Some(families) = nerd_fonts(config) {
        run("Apply", "Nerd Fonts install", Operation::NerdFontsApply { families, force: false })?;
    }
    if let Some(operation) = dotfiles_operation(config, &config.linux.dotfiles.packages, dotfiles_root, false) {
        run("Apply", "dotfiles apply", operation)?;
    }
    linux_integrations(config)?;
    linux_desktop(config, platform)?;
    Ok(())
}

fn macos_apply(config: &Config, arch: Architecture, dotfiles_root: &Path) -> Result<()> {
    let dotfiles = !config.shared.dotfiles.packages.is_empty() || !config.macos.dotfiles.packages.is_empty();
    let homebrew_packages =
        dotfiles || !config.macos.homebrew.formulae.is_empty() || !config.macos.homebrew.casks.is_empty();

    if config.macos.system.validate_sudo_access == Some(true) {
        run("Apply", "macOS sudo access validation", Operation::MacOSSudoAccessValidate)?;
    }
    if config.macos.system.xcode.command_line_tools == Some(true) {
        run("Apply", "Command Line Tools for Xcode install", Operation::CommandLineToolsForXcodeInstall)?;
    }
    run("Apply", "Homebrew install", Operation::HomebrewInstall)?;
    if homebrew_packages {
        let mut formulae = config.macos.homebrew.formulae.clone();
        if dotfiles && !formulae.iter().any(|formula| formula == "stow") {
            formulae.push("stow".into());
        }
        run(
            "Apply",
            "Homebrew package install",
            Operation::HomebrewPackagesInstall { formulae, casks: config.macos.homebrew.casks.clone() },
        )?;
    }
    apply_tools(config, arch)?;
    apply_packages(config)?;
    if let Some(families) = nerd_fonts(config) {
        run("Apply", "Nerd Fonts install", Operation::NerdFontsApply { families, force: false })?;
    }
    if let Some(operation) = dotfiles_operation(config, &config.macos.dotfiles.packages, dotfiles_root, false) {
        run("Apply", "dotfiles apply", operation)?;
    }
    vscode_extensions(config)?;
    macos_desktop(config)?;
    Ok(())
}

fn apply_tools(config: &Config, arch: Architecture) -> Result<()> {
    if let Some(selector) = config.shared.tools.rust.as_deref() {
        run("Apply", "Rust install", Operation::RustInstall { selector: selector.to_owned() })?;
        run("Apply", "cargo-binstall install", Operation::CargoBinstallInstall)?;
        run("Apply", "cargo-update install", Operation::CargoUpdateInstall)?;
    }
    if let Some(selector) = config.shared.tools.node.as_deref() {
        run("Apply", "fnm install", Operation::FnmInstall)?;
        run("Apply", "Node.js version install", Operation::NodeVersionInstall { selector: selector.to_owned() })?;
    }
    if let Some(selector) = &config.shared.tools.python {
        run("Apply", "uv install", Operation::UvInstall)?;
        run("Apply", "Python version install", Operation::PythonVersionInstall { selector: selector.clone() })?;
    }
    if let Some(selector) = config.shared.tools.go.as_deref() {
        run(
            "Apply",
            "Go toolchain install",
            Operation::GoToolchainInstall { selector: go_selector(selector), architecture: arch },
        )?;
    }
    Ok(())
}

fn apply_packages(config: &Config) -> Result<()> {
    if let Some(crates) = config.shared.packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        run("Apply", "Cargo crate install", Operation::CargoCratesInstall { crates: crates.clone() })?;
    }
    if let Some(packages) = config.shared.packages.npm.as_ref().filter(|values| !values.is_empty()) {
        run("Apply", "npm package install", Operation::NpmPackagesInstall { packages: packages.clone() })?;
    }
    Ok(())
}

fn linux_update(config: &Config, platform: &Platform) -> Result<()> {
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    if config.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        apt_prereqs.insert("flatpak");
    }

    run("Update", "APT update", Operation::AptUpdate)?;
    if let Some(policy) = config.linux.updates.as_ref().and_then(|updates| updates.apt) {
        run("Update", "APT upgrade", Operation::AptUpgrade { command: policy })?;
    }
    run(
        "Update",
        "APT package install",
        Operation::AptPackagesInstall { packages: apt_prereqs.iter().map(|value| (*value).to_owned()).collect() },
    )?;
    if config.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        run("Update", "Flatpak update", Operation::FlatpakUpdate)?;
    }
    update_tools_and_packages(config, platform.architecture, false)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(config)
    {
        run("Update", "Nerd Fonts update", Operation::NerdFontsApply { families, force: true })?;
    }
    Ok(())
}

fn macos_update(config: &Config, arch: Architecture) -> Result<()> {
    let homebrew_formulae = config.macos.updates.homebrew.formulae == Some(true);
    let homebrew_casks = config.macos.updates.homebrew.casks == Some(true);

    run("Update", "Homebrew install", Operation::HomebrewInstall)?;
    if homebrew_formulae || homebrew_casks {
        run(
            "Update",
            "Homebrew update and upgrade",
            Operation::HomebrewUpdateAndUpgrade { formulae: homebrew_formulae, casks: homebrew_casks },
        )?;
    }
    update_tools_and_packages(config, arch, true)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(config)
    {
        run("Update", "Nerd Fonts update", Operation::NerdFontsApply { families, force: true })?;
    }
    Ok(())
}

fn update_tools_and_packages(config: &Config, arch: Architecture, macos: bool) -> Result<()> {
    let updates = &config.shared.updates;
    if updates.tools.rust == Some(true) {
        run(
            "Update",
            "Rust install",
            Operation::RustInstall {
                selector: config.shared.tools.rust.clone().unwrap_or_else(|| "stable".to_owned()),
            },
        )?;
        run("Update", "Rust toolchain update", Operation::RustToolchainUpdate)?;
    }
    if updates.tools.go == Some(true) {
        run(
            "Update",
            "Go toolchain update",
            Operation::GoToolchainUpdate {
                selector: go_selector(config.shared.tools.go.as_deref().unwrap_or("latest")),
                architecture: arch,
            },
        )?;
    }
    // macOS resolves npm via Homebrew fnm, so npm-only updates must ensure its formula first
    if updates.tools.node == Some(true) || (macos && updates.packages.npm == Some(true)) {
        run("Update", "fnm install", Operation::FnmInstall)?;
    }
    if updates.tools.node == Some(true) {
        run(
            "Update",
            "Node.js version install",
            Operation::NodeVersionInstall {
                selector: config.shared.tools.node.clone().unwrap_or_else(|| "latest".to_owned()),
            },
        )?;
    }
    if updates.tools.python == Some(true) {
        run("Update", "uv install", Operation::UvInstall)?;
        run("Update", "Python version upgrade", Operation::PythonVersionUpgrade)?;
    }
    if updates.packages.cargo == Some(true) {
        run("Update", "Cargo crate update", Operation::CargoCratesUpdate)?;
    }
    if updates.packages.npm == Some(true) {
        run("Update", "npm package update", Operation::NpmPackagesUpdate)?;
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

fn binary_operation(binary: &crate::config::BinaryPackage, arch: Architecture) -> Option<BinaryPackageOperation> {
    let source = match &binary.source {
        BinarySource::GitHub { repo, assets } => {
            BinarySourceOperation::GitHubLatest { repo: repo.clone(), asset_pattern: assets.get(arch)?.to_owned() }
        }
        BinarySource::Url { urls } => BinarySourceOperation::Url { url: urls.get(arch)?.to_owned() },
    };
    Some(BinaryPackageOperation::new(binary.name.clone(), binary.format, arch, source))
}

fn nerd_fonts(config: &Config) -> Option<Vec<String>> {
    config.shared.fonts.nerd.as_ref().filter(|families| !families.is_empty()).cloned()
}

fn dotfiles_operation(config: &Config, platform_packages: &[String], root: &Path, replace: bool) -> Option<Operation> {
    let packages = config.shared.dotfiles.packages.iter().chain(platform_packages).cloned().collect::<Vec<_>>();
    if packages.is_empty() {
        return None;
    }
    Some(Operation::DotfilesApply { root: root.to_path_buf(), packages, replace })
}

fn linux_integrations(config: &Config) -> Result<()> {
    if let Some(docker) = &config.linux.integrations.docker {
        if docker.group == Some(true) {
            run("Apply", "Docker group membership", Operation::DockerGroupEnsure)?;
        }
        if let Some(logging) = &docker.logging {
            run(
                "Apply",
                "Docker local logging driver set",
                Operation::DockerLocalLoggingDriverSet { max_size: logging.max_size.clone() },
            )?;
        }
    }
    if config.linux.integrations.virtualbox.as_ref().is_some_and(|virtualbox| virtualbox.group == Some(true)) {
        run("Apply", "VirtualBox group membership", Operation::VirtualBoxGroupEnsure)?;
    }
    vscode_extensions(config)
}

fn vscode_extensions(config: &Config) -> Result<()> {
    if !config.shared.integrations.vscode.extensions.is_empty() {
        run(
            "Apply",
            "Visual Studio Code extension install",
            Operation::VsCodeExtensionsInstall { extensions: config.shared.integrations.vscode.extensions.clone() },
        )?;
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

fn linux_desktop(config: &Config, platform: &Platform) -> Result<()> {
    let Some(desktop) = config.linux.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return Ok(()) };
    let Some(environment) = (match platform.desktop {
        DesktopKind::Gnome => Some(DesktopEnvironment::Gnome),
        DesktopKind::Cinnamon => Some(DesktopEnvironment::Cinnamon),
        DesktopKind::None => None,
    }) else {
        return Ok(());
    };
    if let Some(theme) = desktop.theme {
        run(
            "Apply",
            "desktop setting set",
            Operation::DesktopSettingSet { environment, setting: DesktopSetting::ColorScheme(theme) },
        )?;
    }
    if let Some(executable) = &desktop.terminal {
        run(
            "Apply",
            "desktop setting set",
            Operation::DesktopSettingSet { environment, setting: DesktopSetting::Terminal(executable.clone()) },
        )?;
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = idle.timeout {
            run(
                "Apply",
                "desktop setting set",
                Operation::DesktopSettingSet {
                    environment,
                    setting: DesktopSetting::IdleDelaySeconds(timeout.seconds()),
                },
            )?;
        }
        if let Some(enabled) = idle.dim {
            run(
                "Apply",
                "desktop setting set",
                Operation::DesktopSettingSet { environment, setting: DesktopSetting::IdleDim(enabled) },
            )?;
        }
    }
    if environment == DesktopEnvironment::Gnome
        && let Some(gnome) = &desktop.gnome
    {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            run("Apply", "GNOME extension apply", Operation::GnomeExtensionsApply { extensions: extensions.clone() })?;
        }
        if gnome.dash_to_dock == Some(true) {
            run("Apply", "Dash to Dock install", Operation::GnomeDashToDockInstall)?;
        }
        if gnome.rounded_window_corners == Some(true) {
            run("Apply", "Rounded Window Corners install", Operation::GnomeRoundedWindowCornersInstall)?;
        }
    }
    Ok(())
}

fn macos_desktop(config: &Config) -> Result<()> {
    let desktop = &config.macos.desktop;
    let mut settings = Vec::new();
    if let Some(value) = desktop.appearance {
        settings.push(operations::macos::MacDefault::DarkMode(value == Theme::Dark));
    }
    if let Some(dock) = &desktop.dock {
        if let Some(value) = dock.autohide {
            settings.push(operations::macos::MacDefault::DockAutohide(value));
        }
        if let Some(value) = dock.show_recent_applications {
            settings.push(operations::macos::MacDefault::DockRecentApplications(value));
        }
    }
    if let Some(finder) = &desktop.finder {
        if let Some(value) = finder.show_filename_extensions {
            settings.push(operations::macos::MacDefault::ShowAllFilenameExtensions(value));
        }
        if let Some(value) = finder.show_hidden_files {
            settings.push(operations::macos::MacDefault::FinderHiddenFiles(value));
        }
    }
    if let Some(keyboard) = &desktop.keyboard {
        if let Some(value) = keyboard.key_repeat {
            settings.push(operations::macos::MacDefault::KeyRepeat(value));
        }
        if let Some(value) = keyboard.initial_key_repeat {
            settings.push(operations::macos::MacDefault::InitialKeyRepeat(value));
        }
    }
    if let Some(trackpad) = &desktop.trackpad
        && let Some(value) = trackpad.tap_to_click
    {
        settings.push(operations::macos::MacDefault::TrackpadTapToClick(value));
    }
    if !settings.is_empty() {
        run("Apply", "macOS defaults write", Operation::MacDefaultsWrite { settings })?;
    }
    Ok(())
}

fn go_selector(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}

fn run(progress: &str, label: &str, operation: Operation) -> Result<()> {
    println!("{progress}: {label}");
    if matches!(
        operations::run(&operation).with_context(|| format!("{}: {label}", progress.to_lowercase()))?,
        operations::OperationOutcome::LoginRequired
    ) {
        println!("Login required to finish {label}");
    }
    Ok(())
}
