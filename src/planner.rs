use crate::{
    config::{
        AptUpdate, BinaryFormat, BinarySource, Config, EnabledDisabled, SourceMode, Theme, resolve_platform_identity,
        select_distro_map, selected_repository_codename,
    },
    operations::{
        AptRepositoryOperation, AptUpgradePolicy, BinaryPackageOperation, BinarySourceOperation, CargoPackageMode,
        DesktopEnvironment, DesktopSetting, DesktopTheme, GoToolchainSelector, NerdFontsMode, NpmPackageMode,
        Operation, ToolchainMode,
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
    OfficialAptSources,
    DirectAptMetadataRefresh,
    SystemPackageStates,
    SystemPrerequisites,
    ManagerBootstraps,
    LanguageToolchains,
    CargoBinstallBootstrap,
    DirectAptPackages,
    ThirdPartyRepositories,
    RepositoryMetadataRefresh,
    RepositoryPackages,
    FlatpakApplications,
    LanguagePackages,
    AppImageManager,
    BinaryPackages,
    Fonts,
    Dotfiles,
    Integrations,
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
    validate_binary_integrations(config, platform.architecture)?;
    let identity = resolve_platform_identity(platform)?;
    let mut phases = [
        (PlannerPhase::AdministrativeVerification, Vec::new()),
        (PlannerPhase::OfficialAptSources, Vec::new()),
        (PlannerPhase::DirectAptMetadataRefresh, Vec::new()),
        (PlannerPhase::SystemPackageStates, Vec::new()),
        (PlannerPhase::SystemPrerequisites, Vec::new()),
        (PlannerPhase::ManagerBootstraps, Vec::new()),
        (PlannerPhase::LanguageToolchains, Vec::new()),
        (PlannerPhase::CargoBinstallBootstrap, Vec::new()),
        (PlannerPhase::DirectAptPackages, Vec::new()),
        (PlannerPhase::ThirdPartyRepositories, Vec::new()),
        (PlannerPhase::RepositoryMetadataRefresh, Vec::new()),
        (PlannerPhase::RepositoryPackages, Vec::new()),
        (PlannerPhase::FlatpakApplications, Vec::new()),
        (PlannerPhase::LanguagePackages, Vec::new()),
        (PlannerPhase::AppImageManager, Vec::new()),
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

    if let Some(sources) = linux.system.apt.as_ref().and_then(|apt| apt.sources.as_ref())
        && sources.mode == SourceMode::Managed
    {
        let managed =
            sources.resolve_managed(platform, identity)?.expect("managed source resolution returns an intent");
        push_operation(&mut phases, PlannerPhase::OfficialAptSources, Operation::ManagedAptSources(managed));
    }

    plan_system_states(config, platform, &mut phases, &mut needs_direct_apt_refresh);
    plan_tools(config, platform, &mut phases, &mut prerequisites, &mut managers);

    if let Some(install) = apt.and_then(|apt| apt.install.as_ref()).filter(|values| !values.is_empty()) {
        push_operation(
            &mut phases,
            PlannerPhase::DirectAptPackages,
            Operation::AptPackages { packages: install.clone() },
        );
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
            PlannerPhase::FlatpakApplications,
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
            Operation::CargoPackageSet { packages: cargo.clone(), mode: CargoPackageMode::EnsurePresent },
        );
    }
    if let Some(npm) = config.shared.packages.npm.as_ref().filter(|values| !values.is_empty()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::NpmPackageSet { packages: npm.clone(), mode: NpmPackageMode::EnsurePresent },
        );
    }

    plan_appimaged(config, platform, &mut phases, &mut prerequisites);

    if let Some(binaries) = linux.packages.binaries.as_ref().filter(|values| !values.is_empty()) {
        for binary in binaries {
            let Some(planned) = plan_binary(binary, platform.architecture) else {
                continue;
            };
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            if binary.format == BinaryFormat::Deb {
                needs_direct_apt_refresh = true;
            }
            push_operation(&mut phases, PlannerPhase::BinaryPackages, Operation::BinaryPackage(planned));
        }
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
            Operation::Dotfiles { root: dotfiles_root.to_path_buf(), packages: dotfiles },
        );
    }

    plan_integrations(config, &mut phases);
    plan_desktop(config, platform, &mut phases, &mut prerequisites);
    if needs_direct_apt_refresh {
        push_operation(&mut phases, PlannerPhase::DirectAptMetadataRefresh, Operation::AptMetadataRefresh);
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
    for manager in &managers {
        let (phase, operation) = match manager {
            ManagerBootstrap::Flatpak => (PlannerPhase::ManagerBootstraps, Operation::FlatpakEnsureFlathub),
            ManagerBootstrap::Rustup => (PlannerPhase::ManagerBootstraps, Operation::RustupBootstrap),
            ManagerBootstrap::Fnm => (PlannerPhase::ManagerBootstraps, Operation::FnmBootstrap),
            ManagerBootstrap::Uv => (PlannerPhase::ManagerBootstraps, Operation::UvBootstrap),
            ManagerBootstrap::CargoBinstall => {
                (PlannerPhase::CargoBinstallBootstrap, Operation::CargoBinstallBootstrap)
            }
        };
        push_operation(&mut phases, phase, operation);
    }

    if needs_repository_refresh {
        push_operation(&mut phases, PlannerPhase::RepositoryMetadataRefresh, Operation::AptMetadataRefresh);
    }

    Ok(phases.into_iter().flat_map(|(_, operations)| operations).collect())
}

pub fn plan_update(config: &Config, platform: &Platform) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    if platform.is_macos() {
        return plan_macos_update(config, platform.architecture);
    }
    validate_binary_integrations(config, platform.architecture)?;
    let linux = &config.os.linux;
    let updates = linux.updates.as_ref();
    let shared_updates = &config.shared.updates;
    let mut phases = [
        (PlannerPhase::SystemPrerequisites, Vec::new()),
        (PlannerPhase::ManagerBootstraps, Vec::new()),
        (PlannerPhase::ThirdPartyRepositories, Vec::new()),
        (PlannerPhase::RepositoryMetadataRefresh, Vec::new()),
        (PlannerPhase::DirectAptPackages, Vec::new()),
        (PlannerPhase::RepositoryPackages, Vec::new()),
        (PlannerPhase::Updates, Vec::new()),
        (PlannerPhase::FlatpakApplications, Vec::new()),
        (PlannerPhase::LanguageToolchains, Vec::new()),
        (PlannerPhase::LanguagePackages, Vec::new()),
        (PlannerPhase::Fonts, Vec::new()),
    ];
    let packages = &linux.packages;
    let tools = &config.shared.tools;
    let mut prerequisites = BTreeSet::new();
    let mut managers = BTreeSet::new();

    if let Some(policy) = updates.and_then(|updates| updates.apt) {
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
                PlannerPhase::DirectAptPackages,
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
        push_operation(&mut phases, PlannerPhase::FlatpakApplications, Operation::FlatpakUpdateApps);
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
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::CargoPackageSet { packages: Vec::new(), mode: CargoPackageMode::UpdateCurrent },
        );
    }
    if npm_update {
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::NpmPackageSet { packages: Vec::new(), mode: NpmPackageMode::UpdateCurrent },
        );
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
    for manager in managers {
        let operation = match manager {
            ManagerBootstrap::Flatpak => Operation::FlatpakEnsureFlathub,
            ManagerBootstrap::Rustup => Operation::RustupBootstrap,
            ManagerBootstrap::Fnm => Operation::FnmBootstrap,
            ManagerBootstrap::Uv => Operation::UvBootstrap,
            ManagerBootstrap::CargoBinstall => unreachable!("updates do not use cargo-binstall"),
        };
        push_operation(&mut phases, PlannerPhase::ManagerBootstraps, operation);
    }
    Ok(phases.into_iter().flat_map(|(_, operations)| operations).collect())
}

fn plan_macos_apply(config: &Config, architecture: Architecture, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    let mac = config.macos();
    let mut operations = Vec::new();
    if mac.system.ensure_admin == Some(true) {
        operations.push(Operation::EnsureAdmin);
    }
    if mac.system.xcode.command_line_tools == Some(true) {
        operations.push(Operation::XcodeCommandLineTools);
    }
    if mac.system.rosetta == Some(true) {
        operations.push(Operation::Rosetta);
    }
    if !mac.homebrew.formulae.is_empty() || !mac.homebrew.casks.is_empty() {
        operations.push(Operation::HomebrewBootstrap);
        operations.push(Operation::HomebrewPackages {
            formulae: mac.homebrew.formulae.clone(),
            casks: mac.homebrew.casks.clone(),
        });
    }
    plan_shared_portable(config, architecture, &mut operations);
    if !config.shared.integrations.vscode.extensions.is_empty() {
        operations
            .push(Operation::VsCodeExtensionSet { extensions: config.shared.integrations.vscode.extensions.clone() });
    }
    let packages =
        config.shared.dotfiles.packages.iter().chain(mac.dotfiles.packages.iter()).cloned().collect::<Vec<_>>();
    if !packages.is_empty() {
        operations.push(Operation::Dotfiles { root: dotfiles_root.to_path_buf(), packages });
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
        operations.push(Operation::MacDefaults { settings });
    }
    Ok(operations)
}

fn plan_shared_portable(config: &Config, architecture: Architecture, operations: &mut Vec<Operation>) {
    if let Some(selector) = &config.shared.tools.rust {
        operations.push(Operation::RustupBootstrap);
        operations
            .push(Operation::RustToolchain { selector: Some(selector.clone()), mode: ToolchainMode::EnsurePresent });
    }
    if let Some(selector) = &config.shared.tools.go {
        operations.push(Operation::GoToolchain {
            selector: go_selector_main(selector),
            architecture,
            mode: ToolchainMode::EnsurePresent,
        });
    }
    if let Some(selector) = &config.shared.tools.node {
        operations.push(Operation::FnmBootstrap);
        operations.push(Operation::NodeToolchain { selector: selector.clone(), mode: ToolchainMode::EnsurePresent });
    }
    if let Some(selector) = &config.shared.tools.python {
        operations.push(Operation::UvBootstrap);
        operations.push(Operation::PythonToolchain { version: selector.clone(), mode: ToolchainMode::EnsurePresent });
    }
    if let Some(packages) = &config.shared.packages.cargo {
        if !packages.is_empty() {
            operations.push(Operation::CargoBinstallBootstrap);
        }
        operations
            .push(Operation::CargoPackageSet { packages: packages.clone(), mode: CargoPackageMode::EnsurePresent });
    }
    if let Some(packages) = &config.shared.packages.npm {
        operations.push(Operation::NpmPackageSet { packages: packages.clone(), mode: NpmPackageMode::EnsurePresent });
    }
    if !config.shared.fonts.nerd.as_deref().unwrap_or_default().is_empty() {
        operations.push(Operation::UserNerdFonts {
            families: config.shared.fonts.nerd.clone().unwrap_or_default(),
            mode: NerdFontsMode::EnsurePresent,
        });
    }
}

fn plan_macos_update(config: &Config, architecture: Architecture) -> Result<Vec<Operation>> {
    let updates = &config.macos().updates.homebrew;
    let formulae = updates.formulae == Some(true);
    let casks = updates.casks == Some(true);
    let mut operations = Vec::new();
    if formulae || casks {
        operations.push(Operation::HomebrewUpdate { formulae, casks });
    }
    let tools = &config.shared.updates.tools;
    if tools.rust == Some(true) {
        operations.push(Operation::RustToolchain {
            selector: config.shared.tools.rust.clone(),
            mode: ToolchainMode::ConvergeLatest,
        });
    }
    if tools.go == Some(true) {
        operations.push(Operation::GoToolchain {
            selector: go_selector_main(config.shared.tools.go.as_deref().unwrap_or("latest")),
            architecture,
            mode: ToolchainMode::ConvergeLatest,
        });
    }
    if tools.node == Some(true) {
        operations.push(Operation::NodeToolchain {
            selector: config.shared.tools.node.clone().unwrap_or_else(|| "latest".into()),
            mode: ToolchainMode::ConvergeLatest,
        });
    }
    if tools.python == Some(true) {
        operations.push(Operation::PythonToolchain {
            version: config.shared.tools.python.clone().unwrap_or_else(|| "latest".into()),
            mode: ToolchainMode::ConvergeLatest,
        });
    }
    let packages = &config.shared.updates.packages;
    if packages.cargo == Some(true) {
        operations.push(Operation::CargoPackageSet { packages: Vec::new(), mode: CargoPackageMode::UpdateCurrent });
    }
    if packages.npm == Some(true) {
        operations.push(Operation::NpmPackageSet { packages: Vec::new(), mode: NpmPackageMode::UpdateCurrent });
    }
    if config.shared.updates.fonts == Some(true) {
        operations.push(Operation::UserNerdFonts {
            families: config.shared.fonts.nerd.clone().unwrap_or_default(),
            mode: NerdFontsMode::Update,
        });
    }
    Ok(operations)
}

fn push_operation(phases: &mut [(PlannerPhase, Vec<Operation>)], phase: PlannerPhase, op: Operation) {
    phases.iter_mut().find(|(p, _)| *p == phase).expect("phase exists").1.push(op);
}

fn validate_binary_integrations(config: &Config, architecture: Architecture) -> Result<()> {
    let has_appimage = config.os.linux.packages.binaries.as_ref().is_some_and(|binaries| {
        binaries
            .iter()
            .any(|binary| binary.format == BinaryFormat::Appimage && plan_binary(binary, architecture).is_some())
    });
    if has_appimage && config.os.linux.integrations.appimaged != Some(true) {
        anyhow::bail!("packages.binaries: AppImages require integrations.appimaged: true");
    }
    Ok(())
}

fn plan_system_states(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    needs_apt_refresh: &mut bool,
) {
    let system = &config.os.linux.system;
    if let Some(state) = system.apt.as_ref().and_then(|apt| apt.unattended_upgrades) {
        push_operation(
            phases,
            PlannerPhase::SystemPackageStates,
            Operation::UnattendedUpgrades { enabled: enabled(state) },
        );
        *needs_apt_refresh = true;
    }
    let Some(ubuntu) = &system.ubuntu else { return };
    let ubuntu_family = platform.upstream == "ubuntu";
    if let Some(state) = ubuntu.snap
        && ubuntu_family
    {
        *needs_apt_refresh = true;
        push_operation(phases, PlannerPhase::SystemPackageStates, Operation::UbuntuSnap { enabled: enabled(state) });
    }
    if ubuntu.codecs && ubuntu_family {
        *needs_apt_refresh = true;
        push_operation(
            phases,
            PlannerPhase::SystemPackageStates,
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

fn plan_appimaged(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
) {
    if config.os.linux.integrations.appimaged == Some(true) {
        prerequisites.extend(["ca-certificates", "curl"]);
        push_operation(
            phases,
            PlannerPhase::AppImageManager,
            Operation::Appimaged { architecture: platform.architecture },
        );
    }
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

    #[test]
    fn full_example_parses_macos_configuration() {
        let config = Config::parse(include_str!("../examples/full.yaml")).unwrap();
        assert_eq!(config.macos().homebrew.formulae[0], "cmake");
        assert_eq!(config.macos().desktop.appearance, Some(Theme::Dark));
    }

    #[test]
    fn macos_planner_emits_native_operations() {
        let mut config = Config::parse(include_str!("../examples/full.yaml")).unwrap();
        config.os.macos.system.rosetta = Some(true);
        let operations = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();

        assert!(operations.contains(&Operation::HomebrewBootstrap));
        assert!(operations.contains(&Operation::XcodeCommandLineTools));
        assert!(operations.contains(&Operation::Rosetta));
        assert!(
            operations
                .iter()
                .any(|operation| matches!(operation, Operation::MacDefaults { settings } if settings.len() == 8))
        );
    }
}
