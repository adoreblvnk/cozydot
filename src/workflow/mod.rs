//! Platform workflows that derive prerequisites and execute typed operations in dependency order.

use crate::{
    config::{
        BinaryFormat, BinarySource, Config, EnabledDisabled, PlatformIdentity, Repo, Theme, resolve_platform_identity,
        select_distro_map, selected_repo_codename,
    },
    operations::{
        self, AptRepo, BinaryPackageOperation, BinarySourceOperation, DesktopEnvironment, DesktopSetting,
        GoToolchainSelector, NerdFontsMode, Operation,
    },
    platform::{Architecture, Platform},
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const LINUX_BASE_PREREQUISITES: [&str; 7] =
    ["ca-certificates", "curl", "fontconfig", "gnupg", "stow", "unzip", "xz-utils"];

#[derive(Default)]
struct ManagerInstallPlan {
    flatpak: bool,
    rustup: bool,
    fnm: bool,
    uv: bool,
    cargo_binstall: bool,
    cargo_update: bool,
}

struct LinuxApplyPlan {
    apt_prerequisites: BTreeSet<&'static str>,
    manager_installs: ManagerInstallPlan,
    update_apt: bool,
    repos: Vec<AptRepo>,
    repo_packages_to_purge: Vec<String>,
    repo_packages_to_install: Vec<String>,
    deb_binaries: Vec<BinaryPackageOperation>,
    appimages: Vec<BinaryPackageOperation>,
}

pub fn apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    if platform.is_macos() {
        macos_apply(config, platform.architecture, dotfiles_root)
    } else {
        linux_apply(config, platform, dotfiles_root)
    }
}

pub fn update(config: &Config, platform: &Platform) -> Result<()> {
    if platform.is_macos() { macos_update(config, platform.architecture) } else { linux_update(config, platform) }
}

pub fn dotfiles(config: &Config, platform: &Platform, root: &Path, replace: bool) -> Result<()> {
    let platform_packages =
        if platform.is_macos() { &config.macos.dotfiles.packages } else { &config.linux.dotfiles.packages };
    if let Some(operation) = dotfiles_operation(config, platform_packages, root, replace)? {
        run("Applying", operation)?;
    }
    Ok(())
}

fn linux_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    let plan = plan_linux_apply(config, platform)?;

    if platform.distro == "debian" {
        if config.linux.system.sudo_group == Some(true) {
            run("Applying", Operation::SudoGroup)?;
        }
        run("Applying", Operation::AddDebianAptComponents { release: platform.distro_codename.clone() })?;
    }
    if platform.distro == "ubuntu"
        && let Some(ubuntu) = &config.linux.system.ubuntu
    {
        if let Some(state) = ubuntu.unattended_upgrades {
            run("Applying", Operation::UnattendedUpgrades { enabled: state == EnabledDisabled::Enabled })?;
        }
        if let Some(state) = ubuntu.snapd {
            run("Applying", Operation::Snapd { enabled: state == EnabledDisabled::Enabled })?;
        }
        if ubuntu.restricted_extras {
            run("Applying", Operation::AptPackages { packages: vec!["ubuntu-restricted-extras".into()] })?;
        }
    }
    if plan.update_apt {
        run("Applying", Operation::AptUpdate)?;
    }
    run(
        "Applying",
        Operation::AptUpdateAndInstall {
            packages: plan.apt_prerequisites.iter().map(|value| (*value).to_owned()).collect(),
        },
    )?;
    if let Some(packages) =
        config.linux.packages.apt.as_ref().and_then(|apt| apt.install.as_ref()).filter(|packages| !packages.is_empty())
    {
        run("Applying", Operation::AptPackages { packages: packages.clone() })?;
    }
    if !plan.repos.is_empty() {
        for repo in plan.repos {
            run("Applying", Operation::AptRepo(Box::new(repo)))?;
        }
        run("Applying", Operation::AptUpdate)?;
    }
    if !plan.repo_packages_to_purge.is_empty() || !plan.repo_packages_to_install.is_empty() {
        run(
            "Applying",
            Operation::AptPurgeThenInstall {
                purge: plan.repo_packages_to_purge,
                install: plan.repo_packages_to_install,
            },
        )?;
    }
    if plan.manager_installs.flatpak {
        run("Applying", Operation::FlatpakAddFlathubRemote)?;
        run(
            "Applying",
            Operation::FlatpakInstall {
                refs: config.linux.packages.flatpak.as_ref().expect("Flatpak intent was derived").clone(),
            },
        )?;
    }
    apply_tools(config, platform.architecture, &plan.manager_installs)?;
    apply_packages(config)?;
    for binary in plan.deb_binaries {
        run("Applying", Operation::BinaryPackage(binary))?;
    }
    if !plan.appimages.is_empty() {
        run("Applying", Operation::Appimaged { architecture: platform.architecture })?;
        for binary in plan.appimages {
            run("Applying", Operation::BinaryPackage(binary))?;
        }
    }
    if let Some(families) = configured_fonts(config) {
        run("Applying", Operation::NerdFonts { families, mode: NerdFontsMode::Install })?;
    }
    if let Some(operation) = dotfiles_operation(config, &config.linux.dotfiles.packages, dotfiles_root, false)? {
        run("Applying", operation)?;
    }
    linux_integrations(config)?;
    linux_desktop(config, platform)?;
    Ok(())
}

fn plan_linux_apply(config: &Config, platform: &Platform) -> Result<LinuxApplyPlan> {
    let identity = resolve_platform_identity(platform)?;
    let mut apt_prerequisites = BTreeSet::from(LINUX_BASE_PREREQUISITES);
    let mut manager_installs = manager_install_plan(config);
    let apt = config.linux.packages.apt.as_ref();
    let mut update_apt = apt.and_then(|apt| apt.install.as_ref()).is_some_and(|values| !values.is_empty())
        || (platform.distro == "ubuntu"
            && config.linux.system.ubuntu.as_ref().is_some_and(|ubuntu| {
                ubuntu.unattended_upgrades.is_some() || ubuntu.snapd.is_some() || ubuntu.restricted_extras
            }));
    let mut repos = Vec::new();
    let mut repo_packages_to_purge = Vec::new();
    let mut repo_packages_to_install = Vec::new();
    for repo in applicable_repos(config, platform, identity) {
        repos.push(repo_operation(repo, platform, identity)?);
        repo_packages_to_purge.extend(repo.conflicts.iter().cloned());
        repo_packages_to_install.extend(repo.packages.iter().cloned());
    }
    if config.linux.packages.flatpak.as_ref().is_some_and(|values| !values.is_empty()) {
        apt_prerequisites.insert("flatpak");
        manager_installs.flatpak = true;
    }
    let mut deb_binaries = Vec::new();
    let mut appimages = Vec::new();
    for binary in config.linux.packages.binaries.as_deref().unwrap_or_default() {
        if let Some(operation) = binary_operation(binary, platform.architecture) {
            match binary.format {
                BinaryFormat::Deb => {
                    update_apt = true;
                    deb_binaries.push(operation);
                }
                BinaryFormat::Appimage => appimages.push(operation),
            }
        }
    }
    derive_desktop_prerequisites(config, platform, &mut apt_prerequisites);
    Ok(LinuxApplyPlan {
        apt_prerequisites,
        manager_installs,
        update_apt,
        repos,
        repo_packages_to_purge,
        repo_packages_to_install,
        deb_binaries,
        appimages,
    })
}

fn macos_apply(config: &Config, architecture: Architecture, dotfiles_root: &Path) -> Result<()> {
    let managers = manager_install_plan(config);
    let dotfiles = !config.shared.dotfiles.packages.is_empty() || !config.macos.dotfiles.packages.is_empty();
    let homebrew_packages =
        dotfiles || !config.macos.homebrew.formulae.is_empty() || !config.macos.homebrew.casks.is_empty();

    if config.macos.system.validate_sudo_access == Some(true) {
        run("Applying", Operation::ValidateMacosSudoAccess)?;
    }
    if config.macos.system.xcode.command_line_tools == Some(true) {
        run("Applying", Operation::InstallCommandLineToolsForXcode)?;
    }
    run("Applying", Operation::InstallHomebrew)?;
    if homebrew_packages {
        let mut formulae = config.macos.homebrew.formulae.clone();
        if dotfiles && !formulae.iter().any(|formula| formula == "stow") {
            formulae.push("stow".into());
        }
        run("Applying", Operation::InstallHomebrewPackages { formulae, casks: config.macos.homebrew.casks.clone() })?;
    }
    apply_tools(config, architecture, &managers)?;
    apply_packages(config)?;
    if let Some(families) = configured_fonts(config) {
        run("Applying", Operation::UserNerdFonts { families, mode: NerdFontsMode::Install })?;
    }
    if let Some(operation) = dotfiles_operation(config, &config.macos.dotfiles.packages, dotfiles_root, false)? {
        run("Applying", operation)?;
    }
    vscode_extensions(config)?;
    macos_desktop(config)?;
    Ok(())
}

fn apply_tools(config: &Config, architecture: Architecture, managers: &ManagerInstallPlan) -> Result<()> {
    if managers.rustup {
        run("Applying", Operation::InstallRustup)?;
    }
    if let Some(selector) = config.shared.tools.rust.as_deref() {
        run("Applying", Operation::RustToolchain { selector: selector.to_owned() })?;
    }
    if managers.cargo_binstall {
        run("Applying", Operation::InstallCargoBinstall)?;
    }
    if managers.cargo_update {
        run("Applying", Operation::InstallCargoUpdate)?;
    }
    if managers.fnm {
        run("Applying", Operation::InstallFnm)?;
    }
    if let Some(selector) = config.shared.tools.node.as_deref() {
        run("Applying", Operation::NodeToolchain { selector: selector.to_owned() })?;
    }
    if managers.uv {
        run("Applying", Operation::InstallUv)?;
    }
    if let Some(version) = &config.shared.tools.python {
        run("Applying", Operation::PythonToolchain { version: version.clone() })?;
    }
    if let Some(selector) = config.shared.tools.go.as_deref() {
        run("Applying", Operation::GoToolchain { selector: go_selector(selector), architecture })?;
    }
    Ok(())
}

fn apply_packages(config: &Config) -> Result<()> {
    if let Some(packages) = config.shared.packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        run("Applying", Operation::CargoInstall { packages: packages.clone() })?;
    }
    if let Some(packages) = config.shared.packages.npm.as_ref().filter(|values| !values.is_empty()) {
        run("Applying", Operation::NpmInstall { packages: packages.clone() })?;
    }
    Ok(())
}

fn manager_install_plan(config: &Config) -> ManagerInstallPlan {
    let rust = config.shared.tools.rust.is_some();
    let cargo = config.shared.packages.cargo.as_ref().is_some_and(|values| !values.is_empty());
    ManagerInstallPlan {
        rustup: rust || cargo,
        fnm: config.shared.tools.node.is_some()
            || config.shared.packages.npm.as_ref().is_some_and(|values| !values.is_empty()),
        uv: config.shared.tools.python.is_some(),
        cargo_binstall: rust || cargo,
        cargo_update: rust,
        ..ManagerInstallPlan::default()
    }
}

fn linux_update(config: &Config, platform: &Platform) -> Result<()> {
    let mut prerequisites = BTreeSet::from(LINUX_BASE_PREREQUISITES);
    if config.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        prerequisites.insert("flatpak");
    }

    if let Some(policy) = config.linux.updates.as_ref().and_then(|updates| updates.apt) {
        run("Updating", Operation::AptUpdate)?;
        run("Updating", Operation::AptUpgrade { command: policy })?;
    }
    run(
        "Updating",
        Operation::AptUpdateAndInstall { packages: prerequisites.iter().map(|value| (*value).to_owned()).collect() },
    )?;
    if config.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        run("Updating", Operation::FlatpakUpdateApps)?;
    }
    update_tools_and_packages(config, platform.architecture, false)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = configured_fonts(config)
    {
        run("Updating", Operation::NerdFonts { families, mode: NerdFontsMode::Update })?;
    }
    Ok(())
}

fn macos_update(config: &Config, architecture: Architecture) -> Result<()> {
    let homebrew_formulae = config.macos.updates.homebrew.formulae == Some(true);
    let homebrew_casks = config.macos.updates.homebrew.casks == Some(true);

    run("Updating", Operation::InstallHomebrew)?;
    if homebrew_formulae || homebrew_casks {
        run("Updating", Operation::HomebrewUpdate { formulae: homebrew_formulae, casks: homebrew_casks })?;
    }
    update_tools_and_packages(config, architecture, true)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = configured_fonts(config)
    {
        run("Updating", Operation::UserNerdFonts { families, mode: NerdFontsMode::Update })?;
    }
    Ok(())
}

fn update_tools_and_packages(config: &Config, architecture: Architecture, macos: bool) -> Result<()> {
    let updates = &config.shared.updates;
    if updates.tools.rust == Some(true) {
        run("Updating", Operation::InstallRustup)?;
        run("Updating", Operation::RustToolchainUpdate)?;
    }
    if updates.tools.go == Some(true) {
        run(
            "Updating",
            Operation::GoToolchainUpdate {
                selector: go_selector(config.shared.tools.go.as_deref().unwrap_or("latest")),
                architecture,
            },
        )?;
    }
    // macOS resolves npm through Homebrew's FNM, so npm-only updates must ensure the formula first.
    if updates.tools.node == Some(true) || (macos && updates.packages.npm == Some(true)) {
        run("Updating", Operation::InstallFnm)?;
    }
    if updates.tools.node == Some(true) {
        run(
            "Updating",
            Operation::NodeToolchainUpdate {
                selector: config.shared.tools.node.clone().unwrap_or_else(|| "latest".to_owned()),
            },
        )?;
    }
    if updates.tools.python == Some(true) {
        run("Updating", Operation::InstallUv)?;
        run("Updating", Operation::PythonToolchainUpdate)?;
    }
    if updates.packages.cargo == Some(true) {
        run("Updating", Operation::CargoPackageUpdate)?;
    }
    if updates.packages.npm == Some(true) {
        run("Updating", Operation::NpmPackageUpdate)?;
    }
    Ok(())
}

fn applicable_repos<'a>(
    config: &'a Config,
    platform: &Platform,
    identity: PlatformIdentity,
) -> impl Iterator<Item = &'a Repo> {
    config
        .linux
        .packages
        .apt
        .as_ref()
        .and_then(|apt| apt.repos.as_deref())
        .unwrap_or_default()
        .iter()
        .filter(move |repo| repo.applies_to(identity.distro, identity.upstream, platform.architecture))
}

fn repo_operation(repo: &Repo, platform: &Platform, identity: PlatformIdentity) -> Result<AptRepo> {
    let (key, source_url) =
        select_distro_map(&repo.urls, identity.distro, identity.upstream).expect("applicable repo has a selected URL");
    let suite = if repo.suite == "system" {
        selected_repo_codename(key, platform, identity.distro).to_owned()
    } else {
        repo.suite.clone()
    };
    AptRepo::new(
        repo.name.clone(),
        repo.key.clone(),
        source_url.clone(),
        platform.architecture,
        suite,
        repo.components.clone(),
        PathBuf::from(&repo.key_path),
    )
}

fn binary_operation(
    binary: &crate::config::BinaryPackage,
    architecture: Architecture,
) -> Option<BinaryPackageOperation> {
    let source = match &binary.source {
        BinarySource::Github { repo, assets } => {
            BinarySourceOperation::GithubLatest { repo: repo.clone(), selector: assets.get(architecture)?.to_owned() }
        }
        BinarySource::Url { urls } => BinarySourceOperation::Url { url: urls.get(architecture)?.to_owned() },
    };
    Some(BinaryPackageOperation::new(binary.name.clone(), binary.format, architecture, source))
}

fn configured_fonts(config: &Config) -> Option<Vec<String>> {
    config.shared.fonts.nerd.as_ref().filter(|families| !families.is_empty()).cloned()
}

fn dotfiles_operation(
    config: &Config,
    platform_packages: &[String],
    root: &Path,
    replace: bool,
) -> Result<Option<Operation>> {
    let packages = config.shared.dotfiles.packages.iter().chain(platform_packages).cloned().collect::<Vec<_>>();
    if packages.is_empty() {
        return Ok(None);
    }
    if root.as_os_str().is_empty() {
        anyhow::bail!("dotfiles root must not be empty");
    }
    Ok(Some(Operation::Dotfiles { root: root.to_path_buf(), packages, replace }))
}

fn linux_integrations(config: &Config) -> Result<()> {
    if let Some(docker) = &config.linux.integrations.docker {
        if docker.group == Some(true) {
            run("Applying", Operation::DockerGroup)?;
        }
        if let Some(logging) = &docker.logging {
            run("Applying", Operation::DockerLocalLoggingDriver { max_size: logging.max_size.clone() })?;
        }
    }
    if config.linux.integrations.virtualbox.as_ref().is_some_and(|virtualbox| virtualbox.group == Some(true)) {
        run("Applying", Operation::VirtualBoxGroup)?;
    }
    vscode_extensions(config)
}

fn vscode_extensions(config: &Config) -> Result<()> {
    if !config.shared.integrations.vscode.extensions.is_empty() {
        run(
            "Applying",
            Operation::VsCodeInstallExtensions { extensions: config.shared.integrations.vscode.extensions.clone() },
        )?;
    }
    Ok(())
}

fn derive_desktop_prerequisites(config: &Config, platform: &Platform, prerequisites: &mut BTreeSet<&'static str>) {
    let Some(desktop) = config.linux.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return };
    prerequisites.extend(["dconf-cli", "libglib2.0-bin"]);
    if platform.desktop == "gnome"
        && desktop.gnome.as_ref().is_some_and(|gnome| {
            gnome.extensions.as_ref().is_some_and(|values| !values.is_empty())
                || gnome.dash_to_dock == Some(true)
                || gnome.rounded_window_corners == Some(true)
        })
    {
        prerequisites.insert("gnome-shell");
    }
}

fn linux_desktop(config: &Config, platform: &Platform) -> Result<()> {
    let Some(desktop) = config.linux.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return Ok(()) };
    let target = match platform.desktop.as_str() {
        "gnome" => DesktopEnvironment::Gnome,
        "cinnamon" => DesktopEnvironment::Cinnamon,
        _ => unreachable!("platform validation rejects unsupported desktop intent"),
    };
    if let Some(theme) = desktop.theme {
        run("Applying", Operation::DesktopSetting { target, setting: DesktopSetting::ColorScheme(theme) })?;
    }
    if let Some(executable) = &desktop.terminal {
        run("Applying", Operation::DesktopSetting { target, setting: DesktopSetting::Terminal(executable.clone()) })?;
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = idle.timeout {
            run(
                "Applying",
                Operation::DesktopSetting { target, setting: DesktopSetting::IdleDelaySeconds(timeout.seconds()) },
            )?;
        }
        if let Some(enabled) = idle.dim {
            run("Applying", Operation::DesktopSetting { target, setting: DesktopSetting::IdleDim(enabled) })?;
        }
    }
    if target == DesktopEnvironment::Gnome
        && let Some(gnome) = &desktop.gnome
    {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            run("Applying", Operation::GnomeExtensions { extensions: extensions.clone() })?;
        }
        if gnome.dash_to_dock == Some(true) {
            run("Applying", Operation::InstallDashToDock)?;
        }
        if gnome.rounded_window_corners == Some(true) {
            run("Applying", Operation::InstallRoundedWindowCorners)?;
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
        run("Applying", Operation::MacDefaults { settings })?;
    }
    Ok(())
}

fn go_selector(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}

fn run(progress: &str, operation: Operation) -> Result<()> {
    let label = operation.label();
    println!("{progress} {label}");
    if matches!(
        operations::run(&operation).with_context(|| format!("{} {label}", progress.to_lowercase()))?,
        operations::OperationOutcome::LoginRequired
    ) {
        println!("Login required to finish {label}");
    }
    Ok(())
}
