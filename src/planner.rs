use crate::{
    config::{
        AptUpdate, BinaryFormat, BinarySource, Config, EnabledDisabled, Theme, resolve_platform_identity,
        select_distro_map, selected_repository_codename,
    },
    operations::{
        AptRepositoryOperation, AptUpgradePolicy, BinaryPackageOperation, BinarySourceOperation, DesktopEnvironment,
        DesktopSetting, DesktopTheme, GoToolchainSelector, NerdFontsMode, Operation, ToolchainMode,
    },
    platform::{Architecture, Platform},
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PlannerPhase {
    AdministrativeVerification,
    PlatformFoundation,
    SystemMetadataRefresh,
    SystemState,
    SystemPrerequisites,
    SystemManagerBootstrap,
    SystemPackages,
    ThirdPartyRepositories,
    RepositoryMetadataRefresh,
    RepositoryPackages,
    ApplicationManagerBootstraps,
    ApplicationPackages,
    LanguageManagerBootstraps,
    LanguageToolchains,
    LanguagePackageManagerBootstrap,
    LanguagePackages,
    BinaryManagerBootstrap,
    BinaryPackages,
    Fonts,
    Integrations,
    Dotfiles,
    Desktop,
    Updates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ManagerBootstrap {
    Flatpak,
    Rustup,
    Fnm,
    Uv,
    CargoBinstall,
}

pub fn plan_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    if platform.is_macos() {
        return plan_macos_apply(config, platform.architecture, dotfiles_root);
    }
    let identity = resolve_platform_identity(platform)?;
    let mut phases = [
        (PlannerPhase::AdministrativeVerification, Vec::new()),
        (PlannerPhase::PlatformFoundation, Vec::new()),
        (PlannerPhase::SystemMetadataRefresh, Vec::new()),
        (PlannerPhase::SystemState, Vec::new()),
        (PlannerPhase::SystemPrerequisites, Vec::new()),
        (PlannerPhase::ApplicationManagerBootstraps, Vec::new()),
        (PlannerPhase::LanguageManagerBootstraps, Vec::new()),
        (PlannerPhase::LanguageToolchains, Vec::new()),
        (PlannerPhase::LanguagePackageManagerBootstrap, Vec::new()),
        (PlannerPhase::SystemPackages, Vec::new()),
        (PlannerPhase::ThirdPartyRepositories, Vec::new()),
        (PlannerPhase::RepositoryMetadataRefresh, Vec::new()),
        (PlannerPhase::RepositoryPackages, Vec::new()),
        (PlannerPhase::ApplicationPackages, Vec::new()),
        (PlannerPhase::LanguagePackages, Vec::new()),
        (PlannerPhase::BinaryManagerBootstrap, Vec::new()),
        (PlannerPhase::BinaryPackages, Vec::new()),
        (PlannerPhase::Fonts, Vec::new()),
        (PlannerPhase::Dotfiles, Vec::new()),
        (PlannerPhase::Integrations, Vec::new()),
        (PlannerPhase::Desktop, Vec::new()),
    ];
    let mut prerequisites = BTreeSet::new();
    let mut managers = BTreeSet::new();
    let mut needs_direct_apt_refresh = false;
    let mut needs_repository_refresh = false;

    let linux = &config.os.linux;
    let apt = linux.packages.apt.as_ref();

    if linux.system.ensure_admin == Some(true) {
        push_operation(&mut phases, PlannerPhase::AdministrativeVerification, Operation::EnsureAdmin);
    }

    if platform.distro == "debian" {
        push_operation(
            &mut phases,
            PlannerPhase::PlatformFoundation,
            Operation::EnsureDebianAptComponents { release: platform.distro_codename.clone() },
        );
    }

    plan_system_states(config, platform, &mut phases, &mut needs_direct_apt_refresh);
    plan_tools(config, platform, &mut phases, &mut prerequisites, &mut managers);

    if let Some(install) = apt.and_then(|apt| apt.install.as_ref()).filter(|values| !values.is_empty()) {
        push_operation(&mut phases, PlannerPhase::SystemPackages, Operation::AptPackages { packages: install.clone() });
        needs_direct_apt_refresh = true;
    }

    if let Some(repositories) = apt.and_then(|apt| apt.repositories.as_ref()).filter(|values| !values.is_empty()) {
        for repository in repositories {
            let Some(operation) = plan_repository(repository, platform, identity)? else {
                continue;
            };
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            prerequisites.insert("gnupg");
            push_operation(
                &mut phases,
                PlannerPhase::ThirdPartyRepositories,
                Operation::AptRepository(Box::new(operation)),
            );
            if !repository.packages.is_empty() {
                push_operation(
                    &mut phases,
                    PlannerPhase::RepositoryPackages,
                    Operation::AptRepositoryPackages {
                        conflicts: selected_repository_conflicts(repository, identity).unwrap_or_default(),
                        packages: repository.packages.clone(),
                    },
                );
            }
            needs_repository_refresh = true;
        }
    }

    if let Some(applications) = linux.packages.flatpak.as_ref().filter(|values| !values.is_empty()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Flatpak);
        push_operation(
            &mut phases,
            PlannerPhase::ApplicationPackages,
            Operation::FlatpakEnsureApps { refs: applications.clone() },
        );
    }

    if let Some(cargo) = config.shared.packages.cargo.as_ref().filter(|values| !values.is_empty()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Rustup);
        managers.insert(ManagerBootstrap::CargoBinstall);
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::CargoPackageSet { packages: cargo.clone() },
        );
    }
    if let Some(npm) = config.shared.packages.npm.as_ref().filter(|values| !values.is_empty()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(&mut phases, PlannerPhase::LanguagePackages, Operation::NpmPackageSet { packages: npm.clone() });
    }

    let mut needs_appimaged = false;
    if let Some(binaries) = linux.packages.binaries.as_ref().filter(|values| !values.is_empty()) {
        for binary in binaries {
            let Some(planned) = plan_binary(binary, platform.architecture) else {
                continue;
            };
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            match binary.format {
                BinaryFormat::Deb => needs_direct_apt_refresh = true,
                BinaryFormat::Appimage => needs_appimaged = true,
            }
            push_operation(&mut phases, PlannerPhase::BinaryPackages, Operation::BinaryPackage(planned));
        }
    }
    if needs_appimaged {
        push_operation(
            &mut phases,
            PlannerPhase::BinaryManagerBootstrap,
            Operation::Appimaged { architecture: platform.architecture },
        );
    }

    if let Some(fonts) = config.shared.fonts.nerd.as_ref().filter(|values| !values.is_empty()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        prerequisites.insert("tar");
        prerequisites.insert("xz-utils");
        prerequisites.insert("fontconfig");
        push_operation(
            &mut phases,
            PlannerPhase::Fonts,
            Operation::NerdFonts { families: fonts.clone(), mode: NerdFontsMode::EnsurePresent },
        );
    }

    let dotfiles =
        config.shared.dotfiles.packages.iter().chain(linux.dotfiles.packages.iter()).cloned().collect::<Vec<_>>();
    if !dotfiles.is_empty() {
        if dotfiles_root.as_os_str().is_empty() {
            anyhow::bail!("dotfiles root must not be empty");
        }
        prerequisites.insert("stow");
        push_operation(
            &mut phases,
            PlannerPhase::Dotfiles,
            Operation::Dotfiles { root: dotfiles_root.to_path_buf(), packages: dotfiles, replace: false },
        );
    }

    plan_integrations(config, &mut phases);
    plan_desktop(config, platform, &mut phases, &mut prerequisites);
    if needs_direct_apt_refresh {
        push_operation(&mut phases, PlannerPhase::SystemMetadataRefresh, Operation::AptMetadataRefresh);
    }

    if managers.contains(&ManagerBootstrap::Flatpak) {
        prerequisites.insert("flatpak");
    }
    if managers.contains(&ManagerBootstrap::Fnm) {
        prerequisites.insert("unzip");
    }
    if !prerequisites.is_empty() {
        push_operation(
            &mut phases,
            PlannerPhase::SystemPrerequisites,
            Operation::AptBootstrapPackages { packages: prerequisites.iter().map(|s| (*s).to_owned()).collect() },
        );
    }
    push_manager_bootstraps(&mut phases, &managers);

    if needs_repository_refresh {
        push_operation(&mut phases, PlannerPhase::RepositoryMetadataRefresh, Operation::AptMetadataRefresh);
    }

    Ok(flatten_phases(phases))
}

pub fn plan_dotfiles(
    config: &Config,
    platform: &Platform,
    dotfiles_root: &Path,
    replace: bool,
) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    let platform_packages =
        if platform.is_macos() { &config.os.macos.dotfiles.packages } else { &config.os.linux.dotfiles.packages };
    let packages = config.shared.dotfiles.packages.iter().chain(platform_packages).cloned().collect::<Vec<_>>();
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    if dotfiles_root.as_os_str().is_empty() {
        anyhow::bail!("dotfiles root must not be empty");
    }
    Ok(vec![Operation::Dotfiles { root: dotfiles_root.to_path_buf(), packages, replace }])
}

pub fn plan_update(config: &Config, platform: &Platform) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    if platform.is_macos() {
        return plan_macos_update(config, platform.architecture);
    }
    let linux = &config.os.linux;
    let updates = linux.updates.as_ref();
    let shared_updates = &config.shared.updates;
    let mut phases = [
        (PlannerPhase::PlatformFoundation, Vec::new()),
        (PlannerPhase::SystemPrerequisites, Vec::new()),
        (PlannerPhase::LanguageManagerBootstraps, Vec::new()),
        (PlannerPhase::ThirdPartyRepositories, Vec::new()),
        (PlannerPhase::RepositoryMetadataRefresh, Vec::new()),
        (PlannerPhase::SystemPackages, Vec::new()),
        (PlannerPhase::RepositoryPackages, Vec::new()),
        (PlannerPhase::Updates, Vec::new()),
        (PlannerPhase::ApplicationPackages, Vec::new()),
        (PlannerPhase::LanguageToolchains, Vec::new()),
        (PlannerPhase::LanguagePackages, Vec::new()),
        (PlannerPhase::Fonts, Vec::new()),
    ];
    let packages = &linux.packages;
    let tools = &config.shared.tools;
    let mut prerequisites = BTreeSet::new();
    let mut managers = BTreeSet::new();

    if let Some(policy) = updates.and_then(|updates| updates.apt) {
        if platform.distro == "debian" {
            push_operation(
                &mut phases,
                PlannerPhase::PlatformFoundation,
                Operation::EnsureDebianAptComponents { release: platform.distro_codename.clone() },
            );
        }
        let identity = resolve_platform_identity(platform)?;
        let apt = packages.apt.as_ref();
        let mut direct =
            apt.and_then(|apt| apt.install.as_ref()).into_iter().flatten().cloned().collect::<BTreeSet<_>>();
        if let Some(repositories) = apt.and_then(|apt| apt.repositories.as_ref()) {
            for repository in repositories {
                let Some(operation) = plan_repository(repository, platform, identity)? else {
                    continue;
                };
                prerequisites.extend(["ca-certificates", "curl", "gnupg"]);
                push_operation(
                    &mut phases,
                    PlannerPhase::ThirdPartyRepositories,
                    Operation::AptRepository(Box::new(operation)),
                );
                if !repository.packages.is_empty() {
                    push_operation(
                        &mut phases,
                        PlannerPhase::RepositoryPackages,
                        Operation::AptRepositoryPackages {
                            conflicts: selected_repository_conflicts(repository, identity).unwrap_or_default(),
                            packages: repository.packages.clone(),
                        },
                    );
                }
            }
        }
        if linux.system.ubuntu.as_ref().is_some_and(|ubuntu| ubuntu.codecs) && platform.upstream == "ubuntu" {
            direct.insert("ubuntu-restricted-extras".into());
        }

        push_operation(&mut phases, PlannerPhase::RepositoryMetadataRefresh, Operation::AptMetadataRefresh);
        if !direct.is_empty() {
            push_operation(
                &mut phases,
                PlannerPhase::SystemPackages,
                Operation::AptPackages { packages: direct.into_iter().collect() },
            );
        }
        push_operation(
            &mut phases,
            PlannerPhase::Updates,
            Operation::AptUpgrade {
                policy: match policy {
                    AptUpdate::Standard => AptUpgradePolicy::Standard,
                    AptUpdate::Full => AptUpgradePolicy::Full,
                },
            },
        );
    }
    if updates.and_then(|updates| updates.flatpak) == Some(true) {
        prerequisites.insert("flatpak");
        push_operation(&mut phases, PlannerPhase::ApplicationPackages, Operation::FlatpakUpdateApps);
    }

    let tool_updates = Some(&shared_updates.tools);
    let rust_update = tool_updates.is_some_and(|updates| updates.rust == Some(true));
    let go_update = tool_updates.is_some_and(|updates| updates.go == Some(true));
    let node_update = tool_updates.is_some_and(|updates| updates.node == Some(true));
    let python_update = tool_updates.is_some_and(|updates| updates.python == Some(true));
    let package_updates = Some(&shared_updates.packages);
    let cargo_update = package_updates.is_some_and(|updates| updates.cargo == Some(true));
    let npm_update = package_updates.is_some_and(|updates| updates.npm == Some(true));

    if rust_update {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Rustup);
        let selector = tools.rust.clone();
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::RustToolchain { selector, mode: ToolchainMode::ConvergeLatest },
        );
    }
    if go_update {
        prerequisites.extend(["ca-certificates", "curl", "tar"]);
        let selector = tools.go.as_deref().unwrap_or("latest");
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::GoToolchain {
                selector: go_selector_main(selector),
                architecture: platform.architecture,
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
    if node_update {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Fnm);
        let selector = tools.node.clone().unwrap_or_else(|| "latest".to_owned());
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::NodeToolchain { selector, mode: ToolchainMode::ConvergeLatest },
        );
    }
    if python_update {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Uv);
        let version = tools.python.clone().unwrap_or_else(|| "3".to_owned());
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::PythonToolchain { version, mode: ToolchainMode::ConvergeLatest },
        );
    }
    if cargo_update {
        push_operation(&mut phases, PlannerPhase::LanguagePackages, Operation::CargoPackageUpdate);
    }
    if npm_update {
        push_operation(&mut phases, PlannerPhase::LanguagePackages, Operation::NpmPackageUpdate);
    }
    if shared_updates.fonts == Some(true) {
        let families = config.shared.fonts.nerd.clone().unwrap_or_default();
        if !families.is_empty() {
            prerequisites.extend(["ca-certificates", "curl", "tar", "xz-utils", "fontconfig"]);
            push_operation(
                &mut phases,
                PlannerPhase::Fonts,
                Operation::NerdFonts { families, mode: NerdFontsMode::Update },
            );
        }
    }

    if managers.contains(&ManagerBootstrap::Flatpak) {
        prerequisites.insert("flatpak");
    }
    if managers.contains(&ManagerBootstrap::Fnm) {
        prerequisites.insert("unzip");
    }
    if !prerequisites.is_empty() {
        push_operation(
            &mut phases,
            PlannerPhase::SystemPrerequisites,
            Operation::AptBootstrapPackages {
                packages: prerequisites.iter().map(|value| (*value).to_owned()).collect(),
            },
        );
    }
    push_manager_bootstraps(&mut phases, &managers);
    Ok(flatten_phases(phases))
}

fn plan_macos_apply(config: &Config, architecture: Architecture, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    let mac = config.macos();
    let mut phases = [
        (PlannerPhase::AdministrativeVerification, Vec::new()),
        (PlannerPhase::PlatformFoundation, Vec::new()),
        (PlannerPhase::SystemManagerBootstrap, Vec::new()),
        (PlannerPhase::SystemPackages, Vec::new()),
        (PlannerPhase::LanguageManagerBootstraps, Vec::new()),
        (PlannerPhase::LanguageToolchains, Vec::new()),
        (PlannerPhase::LanguagePackageManagerBootstrap, Vec::new()),
        (PlannerPhase::LanguagePackages, Vec::new()),
        (PlannerPhase::Fonts, Vec::new()),
        (PlannerPhase::Integrations, Vec::new()),
        (PlannerPhase::Dotfiles, Vec::new()),
        (PlannerPhase::Desktop, Vec::new()),
    ];
    let mut managers = BTreeSet::new();
    if mac.system.ensure_admin == Some(true) {
        push_operation(&mut phases, PlannerPhase::AdministrativeVerification, Operation::MacEnsureAdmin);
    }
    if mac.system.xcode.command_line_tools == Some(true) {
        push_operation(&mut phases, PlannerPhase::PlatformFoundation, Operation::XcodeCommandLineTools);
    }
    if mac.system.rosetta == Some(true) {
        push_operation(&mut phases, PlannerPhase::PlatformFoundation, Operation::Rosetta);
    }
    let packages =
        config.shared.dotfiles.packages.iter().chain(mac.dotfiles.packages.iter()).cloned().collect::<Vec<_>>();
    let mut formulae = mac.homebrew.formulae.clone();
    if !packages.is_empty() && !formulae.iter().any(|formula| formula == "stow") {
        formulae.push("stow".into());
    }
    if !formulae.is_empty() || !mac.homebrew.casks.is_empty() {
        push_operation(&mut phases, PlannerPhase::SystemManagerBootstrap, Operation::HomebrewBootstrap);
        push_operation(
            &mut phases,
            PlannerPhase::SystemPackages,
            Operation::HomebrewPackages { formulae, casks: mac.homebrew.casks.clone() },
        );
    }
    plan_shared_portable(config, architecture, &mut phases, &mut managers);
    if !config.shared.integrations.vscode.extensions.is_empty() {
        push_operation(
            &mut phases,
            PlannerPhase::Integrations,
            Operation::VsCodeExtensionSet { extensions: config.shared.integrations.vscode.extensions.clone() },
        );
    }
    if !packages.is_empty() {
        push_operation(
            &mut phases,
            PlannerPhase::Dotfiles,
            Operation::Dotfiles { root: dotfiles_root.to_path_buf(), packages, replace: false },
        );
    }
    let mut settings = Vec::new();
    if let Some(value) = mac.desktop.appearance {
        settings.push(crate::operations::macos::MacDefault::Appearance(value == Theme::Dark));
    }
    if let Some(dock) = &mac.desktop.dock {
        if let Some(value) = dock.autohide {
            settings.push(crate::operations::macos::MacDefault::DockAutohide(value));
        }
        if let Some(value) = dock.show_recent_applications {
            settings.push(crate::operations::macos::MacDefault::DockRecentApplications(value));
        }
    }
    if let Some(finder) = &mac.desktop.finder {
        if let Some(value) = finder.show_filename_extensions {
            settings.push(crate::operations::macos::MacDefault::FinderExtensions(value));
        }
        if let Some(value) = finder.show_hidden_files {
            settings.push(crate::operations::macos::MacDefault::FinderHiddenFiles(value));
        }
    }
    if let Some(keyboard) = &mac.desktop.keyboard {
        if let Some(value) = keyboard.key_repeat {
            settings.push(crate::operations::macos::MacDefault::KeyRepeat(value));
        }
        if let Some(value) = keyboard.initial_key_repeat {
            settings.push(crate::operations::macos::MacDefault::InitialKeyRepeat(value));
        }
    }
    if let Some(trackpad) = &mac.desktop.trackpad
        && let Some(value) = trackpad.tap_to_click
    {
        settings.push(crate::operations::macos::MacDefault::TrackpadTapToClick(value));
    }
    if !settings.is_empty() {
        push_operation(&mut phases, PlannerPhase::Desktop, Operation::MacDefaults { settings });
    }
    push_manager_bootstraps(&mut phases, &managers);
    Ok(flatten_phases(phases))
}

fn plan_shared_portable(
    config: &Config,
    architecture: Architecture,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    if let Some(selector) = &config.shared.tools.rust {
        managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::RustToolchain { selector: Some(selector.clone()), mode: ToolchainMode::EnsurePresent },
        );
    }
    if let Some(selector) = &config.shared.tools.go {
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::GoToolchain {
                selector: go_selector_main(selector),
                architecture,
                mode: ToolchainMode::EnsurePresent,
            },
        );
    }
    if let Some(selector) = &config.shared.tools.node {
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::NodeToolchain { selector: selector.clone(), mode: ToolchainMode::EnsurePresent },
        );
    }
    if let Some(selector) = &config.shared.tools.python {
        managers.insert(ManagerBootstrap::Uv);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::PythonToolchain { version: selector.clone(), mode: ToolchainMode::EnsurePresent },
        );
    }
    if let Some(packages) = config.shared.packages.cargo.as_ref().filter(|packages| !packages.is_empty()) {
        managers.insert(ManagerBootstrap::CargoBinstall);
        push_operation(
            phases,
            PlannerPhase::LanguagePackages,
            Operation::CargoPackageSet { packages: packages.clone() },
        );
    }
    if let Some(packages) = config.shared.packages.npm.as_ref().filter(|packages| !packages.is_empty()) {
        push_operation(phases, PlannerPhase::LanguagePackages, Operation::NpmPackageSet { packages: packages.clone() });
    }
    if !config.shared.fonts.nerd.as_deref().unwrap_or_default().is_empty() {
        push_operation(
            phases,
            PlannerPhase::Fonts,
            Operation::UserNerdFonts {
                families: config.shared.fonts.nerd.clone().unwrap_or_default(),
                mode: NerdFontsMode::EnsurePresent,
            },
        );
    }
}

fn plan_macos_update(config: &Config, architecture: Architecture) -> Result<Vec<Operation>> {
    let updates = &config.macos().updates.homebrew;
    let formulae = updates.formulae == Some(true);
    let casks = updates.casks == Some(true);
    let mut phases = [
        (PlannerPhase::Updates, Vec::new()),
        (PlannerPhase::LanguageManagerBootstraps, Vec::new()),
        (PlannerPhase::LanguageToolchains, Vec::new()),
        (PlannerPhase::LanguagePackages, Vec::new()),
        (PlannerPhase::Fonts, Vec::new()),
    ];
    let mut managers = BTreeSet::new();
    if formulae || casks {
        push_operation(&mut phases, PlannerPhase::Updates, Operation::HomebrewUpdate { formulae, casks });
    }
    let tools = &config.shared.updates.tools;
    if tools.rust == Some(true) {
        managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::RustToolchain {
                selector: config.shared.tools.rust.clone(),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
    if tools.go == Some(true) {
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::GoToolchain {
                selector: go_selector_main(config.shared.tools.go.as_deref().unwrap_or("latest")),
                architecture,
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
    if tools.node == Some(true) {
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::NodeToolchain {
                selector: config.shared.tools.node.clone().unwrap_or_else(|| "latest".into()),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
    if tools.python == Some(true) {
        managers.insert(ManagerBootstrap::Uv);
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::PythonToolchain {
                version: config.shared.tools.python.clone().unwrap_or_else(|| "latest".into()),
                mode: ToolchainMode::ConvergeLatest,
            },
        );
    }
    let packages = &config.shared.updates.packages;
    if packages.cargo == Some(true) {
        push_operation(&mut phases, PlannerPhase::LanguagePackages, Operation::CargoPackageUpdate);
    }
    if packages.npm == Some(true) {
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(&mut phases, PlannerPhase::LanguagePackages, Operation::NpmPackageUpdate);
    }
    if config.shared.updates.fonts == Some(true) {
        let families = config.shared.fonts.nerd.clone().unwrap_or_default();
        if !families.is_empty() {
            push_operation(
                &mut phases,
                PlannerPhase::Fonts,
                Operation::UserNerdFonts { families, mode: NerdFontsMode::Update },
            );
        }
    }
    push_manager_bootstraps(&mut phases, &managers);
    Ok(flatten_phases(phases))
}

fn push_operation(phases: &mut [(PlannerPhase, Vec<Operation>)], phase: PlannerPhase, op: Operation) {
    phases.iter_mut().find(|(p, _)| *p == phase).expect("phase exists").1.push(op);
}

fn push_manager_bootstraps(phases: &mut [(PlannerPhase, Vec<Operation>)], managers: &BTreeSet<ManagerBootstrap>) {
    for manager in managers {
        let (phase, operation) = match manager {
            ManagerBootstrap::Flatpak => (PlannerPhase::ApplicationManagerBootstraps, Operation::FlatpakEnsureFlathub),
            ManagerBootstrap::Rustup => (PlannerPhase::LanguageManagerBootstraps, Operation::RustupBootstrap),
            ManagerBootstrap::Fnm => (PlannerPhase::LanguageManagerBootstraps, Operation::FnmBootstrap),
            ManagerBootstrap::Uv => (PlannerPhase::LanguageManagerBootstraps, Operation::UvBootstrap),
            ManagerBootstrap::CargoBinstall => {
                (PlannerPhase::LanguagePackageManagerBootstrap, Operation::CargoBinstallBootstrap)
            }
        };
        push_operation(phases, phase, operation);
    }
}

fn flatten_phases<const N: usize>(phases: [(PlannerPhase, Vec<Operation>); N]) -> Vec<Operation> {
    phases.into_iter().flat_map(|(_, operations)| operations).collect()
}

fn plan_system_states(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    needs_apt_refresh: &mut bool,
) {
    let system = &config.os.linux.system;
    if let Some(state) = system.apt.as_ref().and_then(|apt| apt.unattended_upgrades) {
        push_operation(phases, PlannerPhase::SystemState, Operation::UnattendedUpgrades { enabled: enabled(state) });
        *needs_apt_refresh = true;
    }
    let Some(ubuntu) = &system.ubuntu else { return };
    let ubuntu_family = platform.upstream == "ubuntu";
    if let Some(state) = ubuntu.snap
        && ubuntu_family
    {
        *needs_apt_refresh = true;
        push_operation(phases, PlannerPhase::SystemState, Operation::UbuntuSnap { enabled: enabled(state) });
    }
    if ubuntu.codecs && ubuntu_family {
        *needs_apt_refresh = true;
        push_operation(
            phases,
            PlannerPhase::SystemState,
            Operation::AptPackages { packages: vec!["ubuntu-restricted-extras".into()] },
        );
    }
}

fn enabled(state: EnabledDisabled) -> bool {
    state == EnabledDisabled::Enabled
}

fn plan_tools(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    let tools = &config.shared.tools;
    if let Some(selector) = tools.rust.as_deref() {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::RustToolchain { selector: Some(selector.to_owned()), mode: ToolchainMode::EnsurePresent },
        );
    }
    if let Some(selector) = tools.go.as_deref() {
        prerequisites.extend(["ca-certificates", "curl", "tar"]);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::GoToolchain {
                selector: go_selector_main(selector),
                architecture: platform.architecture,
                mode: ToolchainMode::EnsurePresent,
            },
        );
    }
    if let Some(selector) = tools.node.as_deref() {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::NodeToolchain { selector: selector.to_owned(), mode: ToolchainMode::EnsurePresent },
        );
    }
    if let Some(selector) = &tools.python {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Uv);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::PythonToolchain { version: selector.clone(), mode: ToolchainMode::EnsurePresent },
        );
    }
}

fn go_selector_main(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}

fn plan_repository(
    repository: &crate::config::Repository,
    platform: &Platform,
    identity: crate::config::PlatformIdentity,
) -> Result<Option<AptRepositoryOperation>> {
    let Some((key, source_url)) = select_distro_map(&repository.urls, identity.distro, identity.upstream) else {
        return Ok(None);
    };
    let suite = repository.suite.as_ref().map(|suite| {
        if suite == "system" {
            selected_repository_codename(key, platform, identity.distro).to_owned()
        } else {
            suite.clone()
        }
    });
    AptRepositoryOperation::new(
        repository.name.clone(),
        repository.key.clone(),
        source_url.clone(),
        platform.architecture,
        suite,
        repository.components.clone().unwrap_or_default(),
        repository.path.clone(),
        PathBuf::from(&repository.key_path),
    )
    .map(Some)
}

fn selected_repository_conflicts(
    repository: &crate::config::Repository,
    identity: crate::config::PlatformIdentity,
) -> Option<Vec<String>> {
    repository
        .conflicts
        .as_ref()
        .and_then(|conflicts| select_distro_map(conflicts, identity.distro, identity.upstream))
        .map(|(_, packages)| packages.clone())
}

fn plan_binary(binary: &crate::config::BinaryPackage, architecture: Architecture) -> Option<BinaryPackageOperation> {
    let source = match &binary.source {
        BinarySource::Github { repository, assets } => {
            let selector = assets.get(architecture)?;
            BinarySourceOperation::GithubLatest { repository: repository.clone(), selector: selector.to_owned() }
        }
        BinarySource::Url { urls } => BinarySourceOperation::Url { url: urls.get(architecture)?.to_owned() },
    };
    Some(BinaryPackageOperation::new(binary.name.clone(), binary.format, architecture, source))
}

fn plan_integrations(config: &Config, phases: &mut [(PlannerPhase, Vec<Operation>)]) {
    let integrations = &config.os.linux.integrations;
    if let Some(docker) = &integrations.docker {
        if docker.add_user_to_group == Some(true) {
            push_operation(phases, PlannerPhase::Integrations, Operation::DockerGroup);
        }
        if let Some(logging) = &docker.logging {
            push_operation(
                phases,
                PlannerPhase::Integrations,
                Operation::DockerLocalLog { max_size: logging.max_size.clone() },
            );
        }
    }
    if integrations.virtualbox.as_ref().is_some_and(|virtualbox| virtualbox.add_user_to_group == Some(true)) {
        push_operation(phases, PlannerPhase::Integrations, Operation::VirtualBoxGroup);
    }
    if !config.shared.integrations.vscode.extensions.is_empty() {
        let extensions = config.shared.integrations.vscode.extensions.clone();
        push_operation(phases, PlannerPhase::Integrations, Operation::VsCodeExtensionSet { extensions });
    }
}

fn plan_desktop(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
) {
    let Some(desktop) = config.os.linux.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return };
    let target = match platform.desktop.as_str() {
        "gnome" => DesktopEnvironment::Gnome,
        "cinnamon" => DesktopEnvironment::Cinnamon,
        _ => unreachable!("platform validation rejects unsupported desktop intent"),
    };
    prerequisites.extend(["dconf-cli", "libglib2.0-bin"]);
    if let Some(theme) = desktop.theme {
        push_operation(
            phases,
            PlannerPhase::Desktop,
            Operation::DesktopSetting {
                target,
                setting: DesktopSetting::Theme(match theme {
                    Theme::Light => DesktopTheme::Light,
                    Theme::Dark => DesktopTheme::Dark,
                }),
            },
        );
    }
    if let Some(executable) = &desktop.terminal {
        push_operation(
            phases,
            PlannerPhase::Desktop,
            Operation::DesktopSetting { target, setting: DesktopSetting::Terminal(executable.clone()) },
        );
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = &idle.timeout {
            push_operation(
                phases,
                PlannerPhase::Desktop,
                Operation::DesktopSetting { target, setting: DesktopSetting::IdleTimeoutSeconds(timeout.seconds()) },
            );
        }
        if let Some(enabled) = idle.dim {
            push_operation(
                phases,
                PlannerPhase::Desktop,
                Operation::DesktopSetting { target, setting: DesktopSetting::IdleDim(enabled) },
            );
        }
    }
    if target == DesktopEnvironment::Gnome
        && let Some(gnome) = &desktop.gnome
    {
        if let Some(extensions) = gnome.extensions.as_ref().filter(|values| !values.is_empty()) {
            prerequisites.insert("gnome-shell");
            push_operation(
                phases,
                PlannerPhase::Desktop,
                Operation::GnomeExtensions { extensions: extensions.clone() },
            );
        }
        if gnome.dock == Some(true) {
            prerequisites.insert("gnome-shell");
            push_operation(phases, PlannerPhase::Desktop, Operation::GnomeDock);
        }
        if gnome.rounded_corners == Some(true) {
            prerequisites.insert("gnome-shell");
            push_operation(phases, PlannerPhase::Desktop, Operation::GnomeRoundedCorners);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn macos_platform() -> Platform {
        Platform::from_release_parts(
            "macos".into(),
            "macos".into(),
            String::new(),
            String::new(),
            "none".into(),
            "aarch64",
        )
        .unwrap()
    }

    fn debian_platform() -> Platform {
        Platform::from_release_parts(
            "debian".into(),
            "debian".into(),
            "bookworm".into(),
            "bookworm".into(),
            "gnome".into(),
            "amd64",
        )
        .unwrap()
    }

    fn headless_ubuntu_platform() -> Platform {
        Platform::from_release_parts(
            "ubuntu".into(),
            "ubuntu".into(),
            "noble".into(),
            "noble".into(),
            "none".into(),
            "amd64",
        )
        .unwrap()
    }

    #[test]
    fn full_example_parses_macos_configuration() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        assert_eq!(config.macos().homebrew.formulae[0], "cmake");
        assert_eq!(config.macos().desktop.appearance, Some(Theme::Dark));
    }

    #[test]
    fn macos_planner_emits_native_operations() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        config.os.macos.system.rosetta = Some(true);
        let operations = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();

        assert!(operations.contains(&Operation::HomebrewBootstrap));
        assert!(operations.contains(&Operation::MacEnsureAdmin));
        assert!(operations.contains(&Operation::XcodeCommandLineTools));
        assert!(operations.contains(&Operation::Rosetta));
        assert!(operations.iter().any(
            |operation| matches!(operation, Operation::HomebrewPackages { formulae, .. } if formulae.iter().any(|formula| formula == "stow"))
        ));
        assert_eq!(operations.iter().filter(|operation| **operation == Operation::FnmBootstrap).count(), 1);
        assert!(operations.iter().any(|operation| matches!(operation, Operation::Dotfiles { replace: false, .. })));
        assert!(
            operations
                .iter()
                .any(|operation| matches!(operation, Operation::MacDefaults { settings } if settings.len() == 8))
        );

        let dotfiles = plan_dotfiles(&config, &macos_platform(), Path::new("/tmp/dotfiles"), true).unwrap();
        assert!(matches!(dotfiles.as_slice(), [Operation::Dotfiles { replace: true, .. }]));
    }

    #[test]
    fn macos_apply_phases_preserve_dependency_order() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        config.os.macos.system.rosetta = Some(true);
        let operations = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();
        let position = |predicate: fn(&Operation) -> bool| operations.iter().position(predicate).unwrap();

        let admin = position(|operation| matches!(operation, Operation::MacEnsureAdmin));
        let xcode = position(|operation| matches!(operation, Operation::XcodeCommandLineTools));
        let rosetta = position(|operation| matches!(operation, Operation::Rosetta));
        let homebrew_bootstrap = position(|operation| matches!(operation, Operation::HomebrewBootstrap));
        let homebrew_packages = position(|operation| matches!(operation, Operation::HomebrewPackages { .. }));
        let rustup = position(|operation| matches!(operation, Operation::RustupBootstrap));
        let rust = position(|operation| matches!(operation, Operation::RustToolchain { .. }));
        let fnm = position(|operation| matches!(operation, Operation::FnmBootstrap));
        let node = position(|operation| matches!(operation, Operation::NodeToolchain { .. }));
        let uv = position(|operation| matches!(operation, Operation::UvBootstrap));
        let python = position(|operation| matches!(operation, Operation::PythonToolchain { .. }));
        let cargo_binstall = position(|operation| matches!(operation, Operation::CargoBinstallBootstrap));
        let cargo = position(|operation| matches!(operation, Operation::CargoPackageSet { .. }));
        let vscode = position(|operation| matches!(operation, Operation::VsCodeExtensionSet { .. }));
        let dotfiles = position(|operation| matches!(operation, Operation::Dotfiles { .. }));
        let desktop = position(|operation| matches!(operation, Operation::MacDefaults { .. }));

        assert!(admin < xcode);
        assert!(xcode < rosetta);
        assert!(rosetta < homebrew_bootstrap);
        assert!(homebrew_bootstrap < homebrew_packages);
        assert!(homebrew_packages < rustup);
        assert!(rustup < rust);
        assert!(fnm < node);
        assert!(uv < python);
        assert!(rust < cargo_binstall);
        assert!(cargo_binstall < cargo);
        assert!(cargo < vscode);
        assert!(vscode < dotfiles);
        assert!(dotfiles < desktop);
    }

    #[test]
    fn macos_update_phases_deduplicate_manager_bootstraps() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let operations = plan_update(&config, &macos_platform()).unwrap();
        assert_eq!(operations.iter().filter(|operation| **operation == Operation::FnmBootstrap).count(), 1);

        let fnm = operations.iter().position(|operation| *operation == Operation::FnmBootstrap).unwrap();
        let node =
            operations.iter().position(|operation| matches!(operation, Operation::NodeToolchain { .. })).unwrap();
        let npm = operations.iter().position(|operation| *operation == Operation::NpmPackageUpdate).unwrap();
        assert!(fnm < node);
        assert!(node < npm);
    }

    #[test]
    fn debian_apply_always_ensures_required_apt_components() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let operations = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();
        assert!(operations.contains(&Operation::EnsureDebianAptComponents { release: "bookworm".into() }));
    }

    #[test]
    fn cli_preset_plans_on_a_headless_host() {
        let config = Config::parse(include_str!("../configs/cli.yaml")).unwrap();
        let operations = plan_apply(&config, &headless_ubuntu_platform(), Path::new("/tmp/dotfiles")).unwrap();
        assert!(!operations.iter().any(|operation| matches!(operation, Operation::VsCodeExtensionSet { .. })));
    }

    #[test]
    fn macos_planner_skips_empty_portable_package_and_font_sets() {
        let mut config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        config.shared.packages.cargo = Some(Vec::new());
        config.shared.packages.npm = Some(Vec::new());
        config.shared.fonts.nerd = Some(Vec::new());

        let apply = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();
        assert!(!apply.iter().any(|operation| matches!(operation, Operation::CargoPackageSet { .. })));
        assert!(!apply.iter().any(|operation| matches!(operation, Operation::NpmPackageSet { .. })));

        let update = plan_update(&config, &macos_platform()).unwrap();
        assert!(!update.iter().any(|operation| matches!(operation, Operation::UserNerdFonts { .. })));
    }

    #[test]
    fn debian_update_ensures_components_before_refreshing_metadata() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let operations = plan_update(&config, &debian_platform()).unwrap();
        let components = operations
            .iter()
            .position(|operation| matches!(operation, Operation::EnsureDebianAptComponents { .. }))
            .unwrap();
        let refresh = operations.iter().position(|operation| *operation == Operation::AptMetadataRefresh).unwrap();
        assert!(components < refresh);
    }

    #[test]
    fn package_update_flags_plan_update_all_operations() {
        let config = Config::parse(include_str!("../configs/full.yaml")).unwrap();
        let operations = plan_update(&config, &debian_platform()).unwrap();
        assert!(operations.contains(&Operation::CargoPackageUpdate));
        assert!(operations.contains(&Operation::NpmPackageUpdate));
    }
}
