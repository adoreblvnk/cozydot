//! Derive prerequisites & run each platform's operations in dependency order.

use crate::{
    config::{
        AptArchitecture, BinaryFormat, BinarySource, Config, DistroMapKey, EnabledDisabled, Repo, Theme,
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

#[derive(Default)]
struct ToolInstallPlan {
    rustup: bool,
    fnm: bool,
    uv: bool,
    cargo_binstall: bool,
    cargo_update: bool,
}

struct LinuxApplyPlan {
    apt_prereqs: BTreeSet<&'static str>,
    tool_installs: ToolInstallPlan,
    flatpak_refs: Option<Vec<String>>,
    update_apt: bool,
    repos: Vec<AptRepo>,
    repo_packages_to_purge: Vec<String>,
    repo_packages_to_install: Vec<String>,
    deb_binaries: Vec<BinaryPackageOperation>,
    appimages: Vec<BinaryPackageOperation>,
}

pub fn apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<()> {
    match platform.identity {
        PlatformIdentity::MacOs => macos_apply(config, platform.architecture, dotfiles_root),
        PlatformIdentity::Linux { distro, family } => linux_apply(config, platform, distro, family, dotfiles_root),
    }
}

pub fn update(config: &Config, platform: &Platform) -> Result<()> {
    match platform.identity {
        PlatformIdentity::MacOs => macos_update(config, platform.architecture),
        PlatformIdentity::Linux { .. } => linux_update(config, platform),
    }
}

pub fn dotfiles(config: &Config, platform: &Platform, root: &Path, replace: bool) -> Result<()> {
    let platform_packages = match platform.identity {
        PlatformIdentity::MacOs => &config.macos.dotfiles.packages,
        PlatformIdentity::Linux { .. } => &config.linux.dotfiles.packages,
    };
    if let Some(operation) = dotfiles_operation(config, platform_packages, root, replace) {
        run("Apply", operation)?;
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
    let plan = plan_linux_apply(config, platform, distro, family);

    if distro == Distro::Debian {
        if config.linux.system.sudo_group == Some(true) {
            run("Apply", Operation::SudoGroupEnsure)?;
        }
        run("Apply", Operation::DebianAptComponentsAdd)?;
    }
    if distro == Distro::Ubuntu
        && let Some(ubuntu) = &config.linux.system.ubuntu
    {
        if let Some(state) = ubuntu.unattended_upgrades {
            run("Apply", Operation::UnattendedUpgradesSet { enabled: state == EnabledDisabled::Enabled })?;
        }
        if let Some(state) = ubuntu.snapd {
            run("Apply", Operation::SnapdSet { enabled: state == EnabledDisabled::Enabled })?;
        }
        if ubuntu.restricted_extras {
            run("Apply", Operation::AptPackagesInstall { packages: vec!["ubuntu-restricted-extras".into()] })?;
        }
    }
    if plan.update_apt {
        run("Apply", Operation::AptUpdate)?;
    }
    run(
        "Apply",
        Operation::AptPackagesUpdateAndInstall {
            packages: plan.apt_prereqs.iter().map(|value| (*value).to_owned()).collect(),
        },
    )?;
    if let Some(packages) =
        config.linux.packages.apt.as_ref().and_then(|apt| apt.install.as_ref()).filter(|packages| !packages.is_empty())
    {
        run("Apply", Operation::AptPackagesInstall { packages: packages.clone() })?;
    }
    if !plan.repos.is_empty() {
        for repo in plan.repos {
            run("Apply", Operation::AptRepoAdd(Box::new(repo)))?;
        }
        run("Apply", Operation::AptUpdate)?;
    }
    if !plan.repo_packages_to_purge.is_empty() || !plan.repo_packages_to_install.is_empty() {
        run(
            "Apply",
            Operation::AptPackagesPurgeThenInstall {
                purge: plan.repo_packages_to_purge,
                install: plan.repo_packages_to_install,
            },
        )?;
    }
    if let Some(refs) = plan.flatpak_refs {
        run("Apply", Operation::FlatpakFlathubRemoteAdd)?;
        run("Apply", Operation::FlatpakApplicationsInstall { refs })?;
    }
    apply_tools(config, platform.architecture, &plan.tool_installs)?;
    apply_packages(config)?;
    for binary in plan.deb_binaries {
        run("Apply", Operation::BinaryPackageInstall(binary))?;
    }
    if !plan.appimages.is_empty() {
        run("Apply", Operation::AppimagedInstall { architecture: platform.architecture })?;
        for binary in plan.appimages {
            run("Apply", Operation::BinaryPackageInstall(binary))?;
        }
    }
    if let Some(families) = nerd_fonts(config) {
        run("Apply", Operation::NerdFontsInstall { families })?;
    }
    if let Some(operation) = dotfiles_operation(config, &config.linux.dotfiles.packages, dotfiles_root, false) {
        run("Apply", operation)?;
    }
    linux_integrations(config)?;
    linux_desktop(config, platform)?;
    Ok(())
}

fn plan_linux_apply(config: &Config, platform: &Platform, distro: Distro, family: Family) -> LinuxApplyPlan {
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    let tool_installs = tool_install_plan(config);
    let apt = config.linux.packages.apt.as_ref();
    let mut update_apt = apt.and_then(|apt| apt.install.as_ref()).is_some_and(|values| !values.is_empty())
        || (distro == Distro::Ubuntu
            && config.linux.system.ubuntu.as_ref().is_some_and(|ubuntu| {
                ubuntu.unattended_upgrades.is_some() || ubuntu.snapd.is_some() || ubuntu.restricted_extras
            }));
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
                BinaryFormat::Deb => {
                    update_apt = true;
                    deb_binaries.push(operation);
                }
                BinaryFormat::Appimage => appimages.push(operation),
            }
        }
    }
    add_desktop_prereqs(config, platform, &mut apt_prereqs);
    LinuxApplyPlan {
        apt_prereqs,
        tool_installs,
        flatpak_refs,
        update_apt,
        repos,
        repo_packages_to_purge,
        repo_packages_to_install,
        deb_binaries,
        appimages,
    }
}

fn macos_apply(config: &Config, arch: Architecture, dotfiles_root: &Path) -> Result<()> {
    let tools = tool_install_plan(config);
    let dotfiles = !config.shared.dotfiles.packages.is_empty() || !config.macos.dotfiles.packages.is_empty();
    let homebrew_packages =
        dotfiles || !config.macos.homebrew.formulae.is_empty() || !config.macos.homebrew.casks.is_empty();

    if config.macos.system.validate_sudo_access == Some(true) {
        run("Apply", Operation::MacosSudoAccessValidate)?;
    }
    if config.macos.system.xcode.command_line_tools == Some(true) {
        run("Apply", Operation::CommandLineToolsForXcodeInstall)?;
    }
    run("Apply", Operation::HomebrewInstall)?;
    if homebrew_packages {
        let mut formulae = config.macos.homebrew.formulae.clone();
        if dotfiles && !formulae.iter().any(|formula| formula == "stow") {
            formulae.push("stow".into());
        }
        run("Apply", Operation::HomebrewPackagesInstall { formulae, casks: config.macos.homebrew.casks.clone() })?;
    }
    apply_tools(config, arch, &tools)?;
    apply_packages(config)?;
    if let Some(families) = nerd_fonts(config) {
        run("Apply", Operation::UserNerdFontsInstall { families })?;
    }
    if let Some(operation) = dotfiles_operation(config, &config.macos.dotfiles.packages, dotfiles_root, false) {
        run("Apply", operation)?;
    }
    vscode_extensions(config)?;
    macos_desktop(config)?;
    Ok(())
}

fn apply_tools(config: &Config, arch: Architecture, tools: &ToolInstallPlan) -> Result<()> {
    if tools.rustup {
        run("Apply", Operation::RustupInstall)?;
    }
    if let Some(selector) = config.shared.tools.rust.as_deref() {
        run("Apply", Operation::RustToolchainInstall { selector: selector.to_owned() })?;
    }
    if tools.cargo_binstall {
        run("Apply", Operation::CargoBinstallInstall)?;
    }
    if tools.cargo_update {
        run("Apply", Operation::CargoUpdateInstall)?;
    }
    if tools.fnm {
        run("Apply", Operation::FnmInstall)?;
    }
    if let Some(selector) = config.shared.tools.node.as_deref() {
        run("Apply", Operation::NodeVersionInstall { selector: selector.to_owned() })?;
    }
    if tools.uv {
        run("Apply", Operation::UvInstall)?;
    }
    if let Some(selector) = &config.shared.tools.python {
        run("Apply", Operation::PythonVersionInstall { selector: selector.clone() })?;
    }
    if let Some(selector) = config.shared.tools.go.as_deref() {
        run("Apply", Operation::GoToolchainInstall { selector: go_selector(selector), architecture: arch })?;
    }
    Ok(())
}

fn apply_packages(config: &Config) -> Result<()> {
    if let Some(crates) = config.shared.packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        run("Apply", Operation::CargoCratesInstall { crates: crates.clone() })?;
    }
    if let Some(packages) = config.shared.packages.npm.as_ref().filter(|values| !values.is_empty()) {
        run("Apply", Operation::NpmPackagesInstall { packages: packages.clone() })?;
    }
    Ok(())
}

fn tool_install_plan(config: &Config) -> ToolInstallPlan {
    let rust = config.shared.tools.rust.is_some();
    ToolInstallPlan {
        rustup: rust,
        fnm: config.shared.tools.node.is_some(),
        uv: config.shared.tools.python.is_some(),
        cargo_binstall: rust,
        cargo_update: rust,
    }
}

fn linux_update(config: &Config, platform: &Platform) -> Result<()> {
    let mut apt_prereqs = BTreeSet::from(APT_PREREQS);
    if config.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        apt_prereqs.insert("flatpak");
    }

    if let Some(policy) = config.linux.updates.as_ref().and_then(|updates| updates.apt) {
        run("Update", Operation::AptUpdate)?;
        run("Update", Operation::AptUpgrade { command: policy })?;
    }
    run(
        "Update",
        Operation::AptPackagesUpdateAndInstall {
            packages: apt_prereqs.iter().map(|value| (*value).to_owned()).collect(),
        },
    )?;
    if config.linux.updates.as_ref().and_then(|updates| updates.flatpak) == Some(true) {
        run("Update", Operation::FlatpakApplicationsUpdate)?;
    }
    update_tools_and_packages(config, platform.architecture, false)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(config)
    {
        run("Update", Operation::NerdFontsUpdate { families })?;
    }
    Ok(())
}

fn macos_update(config: &Config, arch: Architecture) -> Result<()> {
    let homebrew_formulae = config.macos.updates.homebrew.formulae == Some(true);
    let homebrew_casks = config.macos.updates.homebrew.casks == Some(true);

    run("Update", Operation::HomebrewInstall)?;
    if homebrew_formulae || homebrew_casks {
        run("Update", Operation::HomebrewUpdateAndUpgrade { formulae: homebrew_formulae, casks: homebrew_casks })?;
    }
    update_tools_and_packages(config, arch, true)?;
    if config.shared.updates.fonts == Some(true)
        && let Some(families) = nerd_fonts(config)
    {
        run("Update", Operation::UserNerdFontsUpdate { families })?;
    }
    Ok(())
}

fn update_tools_and_packages(config: &Config, arch: Architecture, macos: bool) -> Result<()> {
    let updates = &config.shared.updates;
    if updates.tools.rust == Some(true) {
        run("Update", Operation::RustupInstall)?;
        run("Update", Operation::RustToolchainUpdate)?;
    }
    if updates.tools.go == Some(true) {
        run(
            "Update",
            Operation::GoToolchainUpdate {
                selector: go_selector(config.shared.tools.go.as_deref().unwrap_or("latest")),
                architecture: arch,
            },
        )?;
    }
    // macOS resolves npm via Homebrew FNM, so npm-only updates must ensure its formula first
    if updates.tools.node == Some(true) || (macos && updates.packages.npm == Some(true)) {
        run("Update", Operation::FnmInstall)?;
    }
    if updates.tools.node == Some(true) {
        run(
            "Update",
            Operation::NodeVersionUpdate {
                selector: config.shared.tools.node.clone().unwrap_or_else(|| "latest".to_owned()),
            },
        )?;
    }
    if updates.tools.python == Some(true) {
        run("Update", Operation::UvInstall)?;
        run("Update", Operation::PythonVersionUpgrade)?;
    }
    if updates.packages.cargo == Some(true) {
        run("Update", Operation::CargoCratesUpdate)?;
    }
    if updates.packages.npm == Some(true) {
        run("Update", Operation::NpmPackagesUpdate)?;
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
                        (AptArchitecture::Amd64, Architecture::Amd64)
                            | (AptArchitecture::Arm64, Architecture::Arm64)
                            | (AptArchitecture::Armhf, Architecture::Arm32)
                    )
                })
            })
        })
        .filter_map(move |repo| select_distro_map(&repo.urls, distro, family).map(|(key, url)| (repo, key, url)))
}

fn add_repo(repo: &Repo, platform: &Platform, distro: Distro, key: DistroMapKey, source_url: &str) -> AptRepo {
    let suite = if repo.suite == "system" {
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
        BinarySource::Github { repo, assets } => {
            BinarySourceOperation::GithubLatest { repo: repo.clone(), asset_pattern: assets.get(arch)?.to_owned() }
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
            run("Apply", Operation::DockerGroupEnsure)?;
        }
        if let Some(logging) = &docker.logging {
            run("Apply", Operation::DockerLocalLoggingDriverSet { max_size: logging.max_size.clone() })?;
        }
    }
    if config.linux.integrations.virtualbox.as_ref().is_some_and(|virtualbox| virtualbox.group == Some(true)) {
        run("Apply", Operation::VirtualBoxGroupEnsure)?;
    }
    vscode_extensions(config)
}

fn vscode_extensions(config: &Config) -> Result<()> {
    if !config.shared.integrations.vscode.extensions.is_empty() {
        run(
            "Apply",
            Operation::VsCodeExtensionsInstall { extensions: config.shared.integrations.vscode.extensions.clone() },
        )?;
    }
    Ok(())
}

fn add_desktop_prereqs(config: &Config, platform: &Platform, apt_prereqs: &mut BTreeSet<&'static str>) {
    let Some(desktop) = config.linux.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return };
    apt_prereqs.extend(["dconf-cli", "libglib2.0-bin"]);
    if platform.desktop == DesktopKind::Gnome
        && desktop.gnome.as_ref().is_some_and(|gnome| {
            gnome.extensions.as_ref().is_some_and(|values| !values.is_empty())
                || gnome.dash_to_dock == Some(true)
                || gnome.rounded_window_corners == Some(true)
        })
    {
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
        run("Apply", Operation::DesktopSettingSet { environment, setting: DesktopSetting::ColorScheme(theme) })?;
    }
    if let Some(executable) = &desktop.terminal {
        run(
            "Apply",
            Operation::DesktopSettingSet { environment, setting: DesktopSetting::Terminal(executable.clone()) },
        )?;
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = idle.timeout {
            run(
                "Apply",
                Operation::DesktopSettingSet {
                    environment,
                    setting: DesktopSetting::IdleDelaySeconds(timeout.seconds()),
                },
            )?;
        }
        if let Some(enabled) = idle.dim {
            run("Apply", Operation::DesktopSettingSet { environment, setting: DesktopSetting::IdleDim(enabled) })?;
        }
    }
    if environment == DesktopEnvironment::Gnome
        && let Some(gnome) = &desktop.gnome
    {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            run("Apply", Operation::GnomeExtensionsApply { extensions: extensions.clone() })?;
        }
        if gnome.dash_to_dock == Some(true) {
            run("Apply", Operation::GnomeDashToDockInstall)?;
        }
        if gnome.rounded_window_corners == Some(true) {
            run("Apply", Operation::GnomeRoundedWindowCornersInstall)?;
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
        run("Apply", Operation::MacDefaultsWrite { settings })?;
    }
    Ok(())
}

fn go_selector(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}

fn run(progress: &str, operation: Operation) -> Result<()> {
    let label = operation.label();
    println!("{progress}: {label}");
    if matches!(
        operations::run(&operation).with_context(|| format!("{}: {label}", progress.to_lowercase()))?,
        operations::OperationOutcome::LoginRequired
    ) {
        println!("Login required to finish {label}");
    }
    Ok(())
}
