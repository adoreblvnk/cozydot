use crate::{
    config::{
        AptUpgradeCommand, BinaryFormat, BinarySource, Config, EnabledDisabled, PlatformIdentity, Repository, Theme,
        resolve_platform_identity, select_distro_map, selected_repository_codename,
    },
    operations::{
        self, AptRepositoryOperation, AptUpgradeCommand as OperationAptUpgradeCommand, BinaryPackageOperation,
        BinarySourceOperation, ColorScheme, DesktopEnvironment, DesktopSetting, GoToolchainSelector, NerdFontsMode,
        Operation,
    },
    platform::{Architecture, Platform},
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const LINUX_FONT_PREREQUISITES: [&str; 5] = ["ca-certificates", "curl", "tar", "xz-utils", "fontconfig"];

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
    identity: PlatformIdentity,
    apt_prerequisites: BTreeSet<&'static str>,
    manager_installs: ManagerInstallPlan,
    update_apt: bool,
    has_applicable_repositories: bool,
    repository_packages_to_purge: Vec<String>,
    repository_packages_to_install: Vec<String>,
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
        execute("Applying", operation)?;
    }
    Ok(())
}

fn linux_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    let plan = plan_linux_apply(config, platform)?;

    if platform.distro == "debian" {
        if config.linux.system.add_user_to_sudo_group == Some(true) {
            execute("Applying", Operation::AddUserToSudoGroup)?;
        }
        execute("Applying", Operation::EnsureDebianAptComponents { release: platform.distro_codename.clone() })?;
    }
    if platform.distro == "ubuntu"
        && let Some(ubuntu) = &config.linux.system.ubuntu
    {
        if let Some(state) = ubuntu.unattended_upgrades {
            execute("Applying", Operation::UnattendedUpgrades { enabled: state == EnabledDisabled::Enabled })?;
        }
        if let Some(state) = ubuntu.snapd {
            execute("Applying", Operation::Snapd { enabled: state == EnabledDisabled::Enabled })?;
        }
        if ubuntu.restricted_extras {
            execute("Applying", Operation::AptPackages { packages: vec!["ubuntu-restricted-extras".into()] })?;
        }
    }
    if plan.update_apt {
        execute("Applying", Operation::AptUpdate)?;
    }
    if !plan.apt_prerequisites.is_empty() {
        execute(
            "Applying",
            Operation::AptUpdateAndInstall {
                packages: plan.apt_prerequisites.iter().map(|value| (*value).to_owned()).collect(),
            },
        )?;
    }
    if let Some(packages) =
        config.linux.packages.apt.as_ref().and_then(|apt| apt.install.as_ref()).filter(|packages| !packages.is_empty())
    {
        execute("Applying", Operation::AptPackages { packages: packages.clone() })?;
    }
    for repository in applicable_repositories(config, platform, plan.identity) {
        execute(
            "Applying",
            Operation::AptRepository(Box::new(repository_operation(repository, platform, plan.identity)?)),
        )?;
    }
    if plan.has_applicable_repositories {
        execute("Applying", Operation::AptUpdate)?;
    }
    if !plan.repository_packages_to_purge.is_empty() || !plan.repository_packages_to_install.is_empty() {
        execute(
            "Applying",
            Operation::AptPurgeThenInstall {
                purge: plan.repository_packages_to_purge,
                install: plan.repository_packages_to_install,
            },
        )?;
    }
    if plan.manager_installs.flatpak {
        execute("Applying", Operation::FlatpakAddFlathubRemote)?;
        execute(
            "Applying",
            Operation::FlatpakInstallMissingApps {
                refs: config.linux.packages.flatpak.as_ref().expect("Flatpak intent was derived").clone(),
            },
        )?;
    }
    apply_tools(config, platform.architecture, &plan.manager_installs)?;
    apply_packages(config)?;
    for binary in configured_binaries(config, platform.architecture, BinaryFormat::Deb) {
        execute("Applying", Operation::BinaryPackage(binary))?;
    }
    let appimages = configured_binaries(config, platform.architecture, BinaryFormat::Appimage);
    if !appimages.is_empty() {
        execute("Applying", Operation::Appimaged { architecture: platform.architecture })?;
        for binary in appimages {
            execute("Applying", Operation::BinaryPackage(binary))?;
        }
    }
    if let Some(families) = configured_fonts(config) {
        execute("Applying", Operation::NerdFonts { families, mode: NerdFontsMode::InstallMissing })?;
    }
    if let Some(operation) = dotfiles_operation(config, &config.linux.dotfiles.packages, dotfiles_root, false)? {
        execute("Applying", operation)?;
    }
    linux_integrations(config)?;
    linux_desktop(config, platform)?;
    Ok(())
}

fn plan_linux_apply(config: &Config, platform: &Platform) -> Result<LinuxApplyPlan> {
    let identity = resolve_platform_identity(platform)?;
    let mut apt_prerequisites = BTreeSet::new();
    let mut manager_installs = manager_install_plan(config);
    let apt = config.linux.packages.apt.as_ref();
    let mut update_apt = apt.and_then(|apt| apt.install.as_ref()).is_some_and(|values| !values.is_empty())
        || (platform.distro == "ubuntu"
            && config.linux.system.ubuntu.as_ref().is_some_and(|ubuntu| {
                ubuntu.unattended_upgrades.is_some() || ubuntu.snapd.is_some() || ubuntu.restricted_extras
            }));
    let mut applicable = false;
    let mut repository_packages_to_purge = Vec::new();
    let mut repository_packages_to_install = Vec::new();
    for repository in applicable_repositories(config, platform, identity) {
        repository_operation(repository, platform, identity)?;
        apt_prerequisites.extend(["ca-certificates", "curl", "gnupg"]);
        repository_packages_to_purge.extend(repository.conflicts.iter().cloned());
        repository_packages_to_install.extend(repository.packages.iter().cloned());
        applicable = true;
    }
    if config.linux.packages.flatpak.as_ref().is_some_and(|values| !values.is_empty()) {
        apt_prerequisites.extend(["ca-certificates", "curl", "flatpak"]);
        manager_installs.flatpak = true;
    }
    if config.shared.tools.rust.is_some() || config.shared.tools.node.is_some() || config.shared.tools.python.is_some()
    {
        apt_prerequisites.extend(["ca-certificates", "curl"]);
    }
    if config.shared.tools.go.is_some() {
        apt_prerequisites.extend(["ca-certificates", "curl", "tar"]);
    }
    if config.shared.packages.cargo.as_ref().is_some_and(|values| !values.is_empty())
        || config.shared.packages.npm.as_ref().is_some_and(|values| !values.is_empty())
    {
        apt_prerequisites.extend(["ca-certificates", "curl"]);
    }
    if manager_installs.fnm {
        apt_prerequisites.insert("unzip");
    }
    for binary in config.linux.packages.binaries.as_deref().unwrap_or_default() {
        if binary_operation(binary, platform.architecture).is_some() {
            apt_prerequisites.extend(["ca-certificates", "curl"]);
            update_apt |= binary.format == BinaryFormat::Deb;
        }
    }
    if configured_fonts(config).is_some() {
        apt_prerequisites.extend(LINUX_FONT_PREREQUISITES);
    }
    if !config.shared.dotfiles.packages.is_empty() || !config.linux.dotfiles.packages.is_empty() {
        apt_prerequisites.insert("stow");
    }
    derive_desktop_prerequisites(config, platform, &mut apt_prerequisites);
    Ok(LinuxApplyPlan {
        identity,
        apt_prerequisites,
        manager_installs,
        update_apt,
        has_applicable_repositories: applicable,
        repository_packages_to_purge,
        repository_packages_to_install,
    })
}

fn macos_apply(config: &Config, architecture: Architecture, dotfiles_root: &Path) -> Result<()> {
    let managers = manager_install_plan(config);
    let dotfiles = !config.shared.dotfiles.packages.is_empty() || !config.macos.dotfiles.packages.is_empty();
    let homebrew_packages =
        dotfiles || !config.macos.homebrew.formulae.is_empty() || !config.macos.homebrew.casks.is_empty();
    let needs_homebrew = homebrew_packages || managers.fnm || managers.cargo_binstall;

    if config.macos.system.validate_sudo_access == Some(true) {
        execute("Applying", Operation::ValidateMacosSudoAccess)?;
    }
    if config.macos.system.xcode.command_line_tools == Some(true) {
        execute("Applying", Operation::InstallCommandLineToolsForXcode)?;
    }
    if config.macos.system.rosetta == Some(true) {
        execute("Applying", Operation::InstallRosetta)?;
    }
    if needs_homebrew {
        execute("Applying", Operation::InstallHomebrew)?;
    }
    if homebrew_packages {
        let mut formulae = config.macos.homebrew.formulae.clone();
        if dotfiles && !formulae.iter().any(|formula| formula == "stow") {
            formulae.push("stow".into());
        }
        execute(
            "Applying",
            Operation::InstallHomebrewPackages { formulae, casks: config.macos.homebrew.casks.clone() },
        )?;
    }
    apply_tools(config, architecture, &managers)?;
    apply_packages(config)?;
    if let Some(families) = configured_fonts(config) {
        execute("Applying", Operation::UserNerdFonts { families, mode: NerdFontsMode::InstallMissing })?;
    }
    if let Some(operation) = dotfiles_operation(config, &config.macos.dotfiles.packages, dotfiles_root, false)? {
        execute("Applying", operation)?;
    }
    vscode_extensions(config)?;
    macos_desktop(config)?;
    Ok(())
}

fn apply_tools(config: &Config, architecture: Architecture, managers: &ManagerInstallPlan) -> Result<()> {
    if managers.rustup {
        execute("Applying", Operation::InstallRustup)?;
    }
    if let Some(selector) = config.shared.tools.rust.as_deref() {
        execute("Applying", Operation::RustToolchain { selector: selector.to_owned() })?;
    }
    if managers.fnm {
        execute("Applying", Operation::InstallFnm)?;
    }
    if let Some(selector) = config.shared.tools.node.as_deref() {
        execute("Applying", Operation::NodeToolchain { selector: selector.to_owned() })?;
    }
    if managers.uv {
        execute("Applying", Operation::InstallUv)?;
    }
    if let Some(version) = &config.shared.tools.python {
        execute("Applying", Operation::PythonToolchain { version: version.clone() })?;
    }
    if let Some(selector) = config.shared.tools.go.as_deref() {
        execute("Applying", Operation::GoToolchain { selector: go_selector(selector), architecture })?;
    }
    if managers.cargo_binstall {
        execute("Applying", Operation::InstallCargoBinstall)?;
    }
    if managers.cargo_update {
        execute("Applying", Operation::InstallCargoUpdate)?;
    }
    Ok(())
}

fn apply_packages(config: &Config) -> Result<()> {
    if let Some(packages) = config.shared.packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        execute("Applying", Operation::CargoInstallMissing { packages: packages.clone() })?;
    }
    if let Some(packages) = config.shared.packages.npm.as_ref().filter(|values| !values.is_empty()) {
        execute("Applying", Operation::NpmInstallMissing { packages: packages.clone() })?;
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
    let mut prerequisites = BTreeSet::new();
    let updates = &config.shared.updates.tools;
    if updates.rust == Some(true) || updates.node == Some(true) || updates.python == Some(true) {
        prerequisites.extend(["ca-certificates", "curl"]);
    }
    if updates.go == Some(true) {
        prerequisites.extend(["ca-certificates", "curl", "tar"]);
    }
    if updates.node == Some(true) {
        prerequisites.insert("unzip");
    }
    if config.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        prerequisites.insert("flatpak");
    }
    if config.shared.updates.fonts == Some(true) && configured_fonts(config).is_some() {
        prerequisites.extend(LINUX_FONT_PREREQUISITES);
    }

    if let Some(policy) = config.linux.updates.as_ref().and_then(|updates| updates.apt) {
        execute("Updating", Operation::AptUpdate)?;
        execute(
            "Updating",
            Operation::AptUpgrade {
                command: match policy {
                    AptUpgradeCommand::Upgrade => OperationAptUpgradeCommand::Upgrade,
                    AptUpgradeCommand::FullUpgrade => OperationAptUpgradeCommand::FullUpgrade,
                },
            },
        )?;
    }
    if !prerequisites.is_empty() {
        execute(
            "Updating",
            Operation::AptUpdateAndInstall {
                packages: prerequisites.iter().map(|value| (*value).to_owned()).collect(),
            },
        )?;
    }
    if config.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        execute("Updating", Operation::FlatpakUpdateApps)?;
    }
    update_tools_and_packages(config, platform.architecture, false)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = configured_fonts(config)
    {
        execute("Updating", Operation::NerdFonts { families, mode: NerdFontsMode::Update })?;
    }
    Ok(())
}

fn macos_update(config: &Config, architecture: Architecture) -> Result<()> {
    let homebrew_formulae = config.macos.updates.homebrew.formulae == Some(true);
    let homebrew_casks = config.macos.updates.homebrew.casks == Some(true);
    let needs_fnm = config.shared.updates.tools.node == Some(true) || config.shared.updates.packages.npm == Some(true);

    if needs_fnm {
        execute("Updating", Operation::InstallHomebrew)?;
    }
    if homebrew_formulae || homebrew_casks {
        execute("Updating", Operation::HomebrewUpdate { formulae: homebrew_formulae, casks: homebrew_casks })?;
    }
    update_tools_and_packages(config, architecture, true)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = configured_fonts(config)
    {
        execute("Updating", Operation::UserNerdFonts { families, mode: NerdFontsMode::Update })?;
    }
    Ok(())
}

fn update_tools_and_packages(config: &Config, architecture: Architecture, macos: bool) -> Result<()> {
    let updates = &config.shared.updates;
    if updates.tools.rust == Some(true) {
        execute("Updating", Operation::InstallRustup)?;
        execute("Updating", Operation::RustToolchainUpdate)?;
    }
    if updates.tools.go == Some(true) {
        execute(
            "Updating",
            Operation::GoToolchainUpdate {
                selector: go_selector(config.shared.tools.go.as_deref().unwrap_or("latest")),
                architecture,
            },
        )?;
    }
    if updates.tools.node == Some(true) || (macos && updates.packages.npm == Some(true)) {
        execute("Updating", Operation::InstallFnm)?;
    }
    if updates.tools.node == Some(true) {
        execute(
            "Updating",
            Operation::NodeToolchainUpdate {
                selector: config.shared.tools.node.clone().unwrap_or_else(|| "latest".to_owned()),
            },
        )?;
    }
    if updates.tools.python == Some(true) {
        execute("Updating", Operation::InstallUv)?;
        execute("Updating", Operation::PythonToolchainUpdate)?;
    }
    if updates.packages.cargo == Some(true) {
        execute("Updating", Operation::CargoPackageUpdate)?;
    }
    if updates.packages.npm == Some(true) {
        execute("Updating", Operation::NpmPackageUpdate)?;
    }
    Ok(())
}

fn applicable_repositories<'a>(
    config: &'a Config,
    platform: &Platform,
    identity: PlatformIdentity,
) -> impl Iterator<Item = &'a Repository> {
    config
        .linux
        .packages
        .apt
        .as_ref()
        .and_then(|apt| apt.repositories.as_deref())
        .unwrap_or_default()
        .iter()
        .filter(move |repository| repository.applies_to(identity.distro, identity.upstream, platform.architecture))
}

fn repository_operation(
    repository: &Repository,
    platform: &Platform,
    identity: PlatformIdentity,
) -> Result<AptRepositoryOperation> {
    let (key, source_url) = select_distro_map(&repository.urls, identity.distro, identity.upstream)
        .expect("applicable repository has a selected URL");
    let suite = if repository.suite == "system" {
        selected_repository_codename(key, platform, identity.distro).to_owned()
    } else {
        repository.suite.clone()
    };
    AptRepositoryOperation::new(
        repository.name.clone(),
        repository.key.clone(),
        source_url.clone(),
        platform.architecture,
        suite,
        repository.components.clone(),
        PathBuf::from(&repository.key_path),
    )
}

fn configured_binaries(
    config: &Config,
    architecture: Architecture,
    format: BinaryFormat,
) -> Vec<BinaryPackageOperation> {
    config
        .linux
        .packages
        .binaries
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|binary| binary.format == format)
        .filter_map(|binary| binary_operation(binary, architecture))
        .collect()
}

fn binary_operation(
    binary: &crate::config::BinaryPackage,
    architecture: Architecture,
) -> Option<BinaryPackageOperation> {
    let source = match &binary.source {
        BinarySource::Github { repository, assets } => BinarySourceOperation::GithubLatest {
            repository: repository.clone(),
            selector: assets.get(architecture)?.to_owned(),
        },
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
        if docker.add_user_to_group == Some(true) {
            execute("Applying", Operation::DockerGroup)?;
        }
        if let Some(logging) = &docker.logging {
            execute("Applying", Operation::DockerLocalLoggingDriver { max_size: logging.max_size.clone() })?;
        }
    }
    if config
        .linux
        .integrations
        .virtualbox
        .as_ref()
        .is_some_and(|virtualbox| virtualbox.add_user_to_group == Some(true))
    {
        execute("Applying", Operation::VirtualBoxGroup)?;
    }
    vscode_extensions(config)
}

fn vscode_extensions(config: &Config) -> Result<()> {
    if !config.shared.integrations.vscode.extensions.is_empty() {
        execute(
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
        execute(
            "Applying",
            Operation::DesktopSetting {
                target,
                setting: DesktopSetting::ColorScheme(match theme {
                    Theme::Light => ColorScheme::Light,
                    Theme::Dark => ColorScheme::Dark,
                }),
            },
        )?;
    }
    if let Some(executable) = &desktop.terminal {
        execute(
            "Applying",
            Operation::DesktopSetting { target, setting: DesktopSetting::Terminal(executable.clone()) },
        )?;
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = idle.timeout {
            execute(
                "Applying",
                Operation::DesktopSetting { target, setting: DesktopSetting::IdleDelaySeconds(timeout.seconds()) },
            )?;
        }
        if let Some(enabled) = idle.dim {
            execute("Applying", Operation::DesktopSetting { target, setting: DesktopSetting::IdleDim(enabled) })?;
        }
    }
    if target == DesktopEnvironment::Gnome
        && let Some(gnome) = &desktop.gnome
    {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            execute("Applying", Operation::GnomeExtensions { extensions: extensions.clone() })?;
        }
        if gnome.dash_to_dock == Some(true) {
            execute("Applying", Operation::InstallDashToDock)?;
        }
        if gnome.rounded_window_corners == Some(true) {
            execute("Applying", Operation::InstallRoundedWindowCorners)?;
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
        execute("Applying", Operation::MacDefaults { settings })?;
    }
    Ok(())
}

fn go_selector(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}

fn execute(progress: &str, operation: Operation) -> Result<()> {
    let label = operation.label();
    println!("{progress} {label}");
    if matches!(
        operations::execute(&operation).with_context(|| format!("{} {label}", progress.to_lowercase()))?,
        operations::OperationOutcome::LoginRequired
    ) {
        println!("Login required to finish {label}");
    }
    Ok(())
}
