use crate::{
    config::{
        resolve_platform_identity, AptUpdate, BinaryFormat, Config, EnabledDisabled, InstalledState,
        ResolvedNativeBinary, SourceMode, Theme,
    },
    operations::{
        AptRepositoryOperation, AptRepositoryPath, AptRepositorySourceLayout, AptRepositoryToken, AptUpgradePolicy,
        BinaryPackageFormat, BinaryPackageMode, BinaryPackageOperation, BinaryPackageSelector, BinarySha256,
        BinarySourceOperation, CargoPackageMode, DesktopEnvironment, DesktopSetting, DesktopTheme, GithubRepository,
        GoToolchainSelector, NerdFontsMode, NodeToolchainSelector, NpmPackageMode, Operation, RustToolchainSelector,
        ToolMutationMode,
    },
    platform::{Architecture, Platform},
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ManagerBootstrap {
    Flatpak,
    Rustup,
    CargoBinstall,
    Fnm,
    Uv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PlannerPhase {
    SystemPrerequisites,
    ManagerBootstraps,
    AdministrativeVerification,
    OfficialAptSources,
    ThirdPartyRepositories,
    AptMetadataRefresh,
    SystemPackageStates,
    AptPurge,
    RepositoryPackages,
    AptPackages,
    FlatpakApplications,
    LanguageToolchains,
    LanguagePackages,
    BinaryPackages,
    Fonts,
    Dotfiles,
    Integrations,
    Desktop,
    Updates,
}

pub fn plan(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    let identity = resolve_platform_identity(platform)?;
    let mut phases = [
        (PlannerPhase::SystemPrerequisites, Vec::new()),
        (PlannerPhase::ManagerBootstraps, Vec::new()),
        (PlannerPhase::AdministrativeVerification, Vec::new()),
        (PlannerPhase::OfficialAptSources, Vec::new()),
        (PlannerPhase::ThirdPartyRepositories, Vec::new()),
        (PlannerPhase::AptMetadataRefresh, Vec::new()),
        (PlannerPhase::SystemPackageStates, Vec::new()),
        (PlannerPhase::AptPurge, Vec::new()),
        (PlannerPhase::RepositoryPackages, Vec::new()),
        (PlannerPhase::AptPackages, Vec::new()),
        (PlannerPhase::FlatpakApplications, Vec::new()),
        (PlannerPhase::LanguageToolchains, Vec::new()),
        (PlannerPhase::LanguagePackages, Vec::new()),
        (PlannerPhase::BinaryPackages, Vec::new()),
        (PlannerPhase::Fonts, Vec::new()),
        (PlannerPhase::Dotfiles, Vec::new()),
        (PlannerPhase::Integrations, Vec::new()),
        (PlannerPhase::Desktop, Vec::new()),
        (PlannerPhase::Updates, Vec::new()),
    ];
    let mut prerequisites = BTreeSet::new();
    let mut managers = BTreeSet::new();
    let mut needs_apt_refresh = false;

    let packages = config.packages.as_ref();
    let apt = packages.and_then(|packages| packages.apt.as_ref());

    if config
        .system
        .as_ref()
        .is_some_and(|system| system.ensure_admin == Some(true))
    {
        push_operation(
            &mut phases,
            PlannerPhase::AdministrativeVerification,
            Operation::EnsureAdmin,
        );
    }

    if let Some(sources) = config
        .system
        .as_ref()
        .and_then(|system| system.apt.as_ref())
        .and_then(|apt| apt.sources.as_ref())
    {
        if sources.mode == SourceMode::Managed {
            let managed = sources
                .resolve_managed(platform, identity)?
                .expect("managed source resolution returns an intent");
            push_operation(
                &mut phases,
                PlannerPhase::OfficialAptSources,
                Operation::ManagedAptSources(managed),
            );
        }
    }

    if let Some(repositories) = apt.and_then(|apt| apt.repositories.as_ref()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        prerequisites.insert("gnupg");
        for repository in repositories {
            let operation = plan_repository(repository, platform, identity)?;
            push_operation(
                &mut phases,
                PlannerPhase::ThirdPartyRepositories,
                Operation::AptRepository(operation),
            );
            push_operation(
                &mut phases,
                PlannerPhase::RepositoryPackages,
                Operation::AptPackages {
                    packages: repository.packages.clone(),
                },
            );
            needs_apt_refresh = true;
        }
    }

    plan_system_states(config, platform, &mut phases, &mut needs_apt_refresh);

    if let Some(remove) = apt.and_then(|apt| apt.remove.as_ref()) {
        push_operation(
            &mut phases,
            PlannerPhase::AptPurge,
            Operation::AptPurge {
                packages: remove.clone(),
            },
        );
        needs_apt_refresh = true;
    }
    if let Some(install) = apt.and_then(|apt| apt.install.as_ref()) {
        push_operation(
            &mut phases,
            PlannerPhase::AptPackages,
            Operation::AptPackages {
                packages: install.clone(),
            },
        );
        needs_apt_refresh = true;
    }

    if let Some(applications) = packages.and_then(|packages| packages.flatpak.as_ref()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Flatpak);
        push_operation(
            &mut phases,
            PlannerPhase::FlatpakApplications,
            Operation::FlatpakEnsureApps {
                refs: applications.clone(),
            },
        );
    }

    plan_tools(config, platform, &mut phases, &mut prerequisites, &mut managers)?;

    if let Some(cargo) = packages.and_then(|packages| packages.cargo.as_ref()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Rustup);
        managers.insert(ManagerBootstrap::CargoBinstall);
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::CargoPackageSet {
                packages: cargo.clone(),
                mode: CargoPackageMode::EnsurePresent,
            },
        );
    }
    if let Some(npm) = packages.and_then(|packages| packages.npm.as_ref()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::NpmPackageSet {
                packages: npm.clone(),
                mode: NpmPackageMode::EnsurePresent,
            },
        );
    }

    if let Some(binaries) = packages.and_then(|packages| packages.binaries.as_ref()) {
        for binary in binaries {
            let Some(planned) = plan_binary(binary, platform.architecture, BinaryPackageMode::EnsurePresent)? else {
                continue;
            };
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            match binary.format {
                BinaryFormat::Deb => {
                    prerequisites.insert("dpkg");
                    needs_apt_refresh = true;
                }
                BinaryFormat::Appimage => {}
            }
            push_operation(
                &mut phases,
                PlannerPhase::BinaryPackages,
                Operation::BinaryPackage(planned),
            );
        }
    }

    if let Some(fonts) = config.fonts.as_ref().and_then(|fonts| fonts.nerd.as_ref()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        prerequisites.insert("tar");
        prerequisites.insert("xz-utils");
        prerequisites.insert("fontconfig");
        push_operation(
            &mut phases,
            PlannerPhase::Fonts,
            Operation::NerdFonts {
                families: fonts.clone(),
                mode: NerdFontsMode::EnsurePresent,
            },
        );
    }

    if let Some(dotfiles) = &config.dotfiles {
        if dotfiles_root.as_os_str().is_empty() {
            anyhow::bail!("dotfiles root must not be empty");
        }
        prerequisites.insert("stow");
        push_operation(
            &mut phases,
            PlannerPhase::Dotfiles,
            Operation::Dotfiles {
                root: dotfiles_root.to_path_buf(),
                packages: dotfiles.packages.clone(),
            },
        );
    }

    plan_integrations(config, &mut phases)?;
    plan_desktop(config, platform, &mut phases, &mut prerequisites)?;
    plan_updates(config, platform, &mut phases, &mut needs_apt_refresh)?;

    if needs_apt_refresh {
        push_operation(
            &mut phases,
            PlannerPhase::AptMetadataRefresh,
            Operation::AptMetadataRefresh,
        );
    }

    if managers.contains(&ManagerBootstrap::Flatpak) {
        prerequisites.insert("flatpak");
    }
    if managers.contains(&ManagerBootstrap::Fnm) {
        prerequisites.insert("unzip");
    }
    if managers.contains(&ManagerBootstrap::CargoBinstall) {
        prerequisites.insert("tar");
    }

    if !prerequisites.is_empty() {
        push_operation(
            &mut phases,
            PlannerPhase::SystemPrerequisites,
            Operation::AptBootstrapPackages {
                packages: prerequisites.iter().map(|s| (*s).to_owned()).collect(),
            },
        );
    }

    for manager in &managers {
        let op = match manager {
            ManagerBootstrap::Flatpak => Operation::FlatpakEnsureFlathub,
            ManagerBootstrap::Rustup => Operation::RustupBootstrap,
            ManagerBootstrap::CargoBinstall => Operation::CargoBinstallBootstrap {
                architecture: platform.architecture,
            },
            ManagerBootstrap::Fnm => Operation::FnmBootstrap,
            ManagerBootstrap::Uv => Operation::UvBootstrap,
        };
        push_operation(&mut phases, PlannerPhase::ManagerBootstraps, op);
    }

    let mut final_operations = Vec::new();
    for (_phase, ops) in phases {
        final_operations.extend(ops);
    }

    Ok(final_operations)
}

fn push_operation(phases: &mut [(PlannerPhase, Vec<Operation>)], phase: PlannerPhase, op: Operation) {
    phases
        .iter_mut()
        .find(|(p, _)| *p == phase)
        .expect("phase exists")
        .1
        .push(op);
}

fn plan_repository(
    repository: &crate::config::Repository,
    platform: &Platform,
    identity: crate::config::PlatformIdentity,
) -> Result<AptRepositoryOperation> {
    let resolved = repository.resolve_for_platform(0, platform, identity)?;
    let layout = if let Some(path) = &repository.path {
        AptRepositorySourceLayout::ExactPath(AptRepositoryPath::parse(path)?)
    } else {
        let suite_token = resolved.suite.as_ref().expect("validated suite/components repository");
        AptRepositorySourceLayout::SuiteComponents {
            suite: AptRepositoryToken::parse(suite_token.as_str())?,
            components: repository
                .components
                .as_ref()
                .expect("validated suite/components repository")
                .iter()
                .map(|component| AptRepositoryToken::parse(component.as_str()))
                .collect::<Result<Vec<_>>>()?,
        }
    };
    AptRepositoryOperation::new(
        repository.name.clone(),
        repository.key.clone(),
        resolved.source_url.clone(),
        platform.architecture,
        layout,
        PathBuf::from(&repository.key_path),
    )
}

fn plan_binary(
    binary: &crate::config::BinaryPackage,
    architecture: Architecture,
    mode: BinaryPackageMode,
) -> Result<Option<BinaryPackageOperation>> {
    let Some(native) = binary.source.resolve_native(architecture) else {
        return Ok(None);
    };
    let source = match native {
        ResolvedNativeBinary::Github { repository, selector } => BinarySourceOperation::GithubLatest {
            repository: GithubRepository::parse(repository.to_owned())?,
            selector: BinaryPackageSelector::new(selector.to_owned())?,
            sha256: None,
        },
        ResolvedNativeBinary::Url { url, sha256 } => BinarySourceOperation::ChecksummedUrl {
            url: url.clone(),
            sha256: BinarySha256::parse(sha256.as_str())?,
        },
    };
    BinaryPackageOperation::new(
        binary.name.clone(),
        match binary.format {
            BinaryFormat::Deb => BinaryPackageFormat::Deb,
            BinaryFormat::Appimage => BinaryPackageFormat::AppImage,
        },
        binary.commands.clone(),
        architecture,
        source,
        mode,
    )
    .map(Some)
}

fn plan_system_states(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    needs_apt_refresh: &mut bool,
) {
    let Some(system) = &config.system else { return };
    if let Some(state) = system.apt.as_ref().and_then(|apt| apt.unattended_upgrades) {
        push_operation(
            phases,
            PlannerPhase::SystemPackageStates,
            Operation::UnattendedUpgrades {
                enabled: enabled(state),
            },
        );
        *needs_apt_refresh = true;
    }
    let Some(ubuntu) = &system.ubuntu else { return };
    let ubuntu_family = platform.upstream == "ubuntu";
    if let Some(state) = ubuntu.snap {
        if ubuntu_family {
            *needs_apt_refresh = true;
            push_operation(
                phases,
                PlannerPhase::SystemPackageStates,
                Operation::UbuntuSnap {
                    enabled: enabled(state),
                },
            );
        }
    }
    if let Some(state) = ubuntu.codecs {
        if ubuntu_family {
            *needs_apt_refresh = true;
            if state == InstalledState::Installed {
                push_operation(
                    phases,
                    PlannerPhase::SystemPackageStates,
                    Operation::AptPackages {
                        packages: vec!["ubuntu-restricted-extras".into()],
                    },
                );
            }
        }
    }
}

fn plan_tools(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
    managers: &mut BTreeSet<ManagerBootstrap>,
) -> Result<()> {
    let Some(tools) = &config.tools else { return Ok(()) };
    if let Some(selector) = tools.rust.as_deref() {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::RustToolchain {
                selector: rust_selector_main(selector),
                architecture: platform.architecture,
                mode: ToolMutationMode::EnsurePresent,
            },
        );
    }
    if let Some(selector) = tools.go.as_deref() {
        prerequisites.extend(["ca-certificates", "curl", "tar", "xz-utils"]);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::GoToolchain {
                selector: go_selector_main(selector),
                architecture: platform.architecture,
                mode: ToolMutationMode::EnsurePresent,
            },
        );
    }
    if let Some(selector) = tools.node.as_deref() {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::NodeToolchain {
                selector: node_selector_main(selector),
                architecture: platform.architecture,
                mode: ToolMutationMode::EnsurePresent,
            },
        );
    }
    if let Some(selector) = &tools.python {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Uv);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::PythonToolchain {
                version: selector.clone(),
                architecture: platform.architecture,
            },
        );
    }
    Ok(())
}

fn plan_integrations(config: &Config, phases: &mut [(PlannerPhase, Vec<Operation>)]) -> Result<()> {
    let Some(integrations) = &config.integrations else {
        return Ok(());
    };
    if let Some(docker) = &integrations.docker {
        if docker.add_user_to_group == Some(true) {
            push_operation(phases, PlannerPhase::Integrations, Operation::DockerGroup);
        }
        if let Some(logging) = &docker.logging {
            push_operation(
                phases,
                PlannerPhase::Integrations,
                Operation::DockerLocalLog {
                    max_size: logging.max_size.clone(),
                },
            );
        }
    }
    if integrations
        .virtualbox
        .as_ref()
        .is_some_and(|virtualbox| virtualbox.add_user_to_group == Some(true))
    {
        push_operation(phases, PlannerPhase::Integrations, Operation::VirtualBoxGroup);
    }
    if let Some(extensions) = integrations.vscode.as_ref().map(|vscode| vscode.extensions.clone()) {
        push_operation(
            phases,
            PlannerPhase::Integrations,
            Operation::VsCodeExtensionSet { extensions },
        );
    }
    Ok(())
}

fn plan_desktop(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
) -> Result<()> {
    let Some(desktop) = &config.desktop else {
        return Ok(());
    };
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
            Operation::DesktopSetting {
                target,
                setting: DesktopSetting::Terminal(executable.clone()),
            },
        );
    }
    if let Some(idle) = &desktop.idle {
        if let Some(timeout) = &idle.timeout {
            push_operation(
                phases,
                PlannerPhase::Desktop,
                Operation::DesktopSetting {
                    target,
                    setting: DesktopSetting::IdleTimeoutSeconds(timeout.seconds()),
                },
            );
        }
        if let Some(enabled) = idle.dim {
            push_operation(
                phases,
                PlannerPhase::Desktop,
                Operation::DesktopSetting {
                    target,
                    setting: DesktopSetting::IdleDim(enabled),
                },
            );
        }
    }
    if target == DesktopEnvironment::Gnome {
        if let Some(gnome) = &desktop.gnome {
            if let Some(extensions) = &gnome.extensions {
                prerequisites.insert("gnome-shell");
                push_operation(
                    phases,
                    PlannerPhase::Desktop,
                    Operation::GnomeExtensions {
                        extensions: extensions.clone(),
                    },
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
    Ok(())
}

fn plan_updates(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    needs_apt_refresh: &mut bool,
) -> Result<()> {
    let Some(updates) = &config.updates else {
        return Ok(());
    };
    let packages = config.packages.as_ref();
    let tools = config.tools.as_ref();
    if let Some(policy) = updates.apt {
        *needs_apt_refresh = true;
        push_operation(
            phases,
            PlannerPhase::Updates,
            Operation::AptUpgrade {
                policy: match policy {
                    AptUpdate::Standard => AptUpgradePolicy::Standard,
                    AptUpdate::Full => AptUpgradePolicy::Full,
                },
            },
        );
    }
    if updates.flatpak == Some(true) {
        push_operation(
            phases,
            PlannerPhase::Updates,
            Operation::FlatpakUpdateApps {
                refs: packages
                    .and_then(|packages| packages.flatpak.clone())
                    .expect("validated update target"),
            },
        );
    }
    if let Some(tool_updates) = &updates.tools {
        if tool_updates.rust == Some(true) {
            push_operation(
                phases,
                PlannerPhase::Updates,
                Operation::RustToolchain {
                    selector: rust_selector_main(
                        tools
                            .and_then(|tools| tools.rust.as_deref())
                            .expect("validated update target"),
                    ),
                    architecture: platform.architecture,
                    mode: ToolMutationMode::UpdateMoving,
                },
            );
        }
        if tool_updates.go == Some(true) {
            push_operation(
                phases,
                PlannerPhase::Updates,
                Operation::GoToolchain {
                    selector: go_selector_main(
                        tools
                            .and_then(|tools| tools.go.as_deref())
                            .expect("validated update target"),
                    ),
                    architecture: platform.architecture,
                    mode: ToolMutationMode::UpdateMoving,
                },
            );
        }
        if tool_updates.node == Some(true) {
            push_operation(
                phases,
                PlannerPhase::Updates,
                Operation::NodeToolchain {
                    selector: node_selector_main(
                        tools
                            .and_then(|tools| tools.node.as_deref())
                            .expect("validated update target"),
                    ),
                    architecture: platform.architecture,
                    mode: ToolMutationMode::UpdateMoving,
                },
            );
        }
    }
    if let Some(package_updates) = &updates.packages {
        if package_updates.cargo == Some(true) {
            push_operation(
                phases,
                PlannerPhase::Updates,
                Operation::CargoPackageSet {
                    packages: packages
                        .and_then(|packages| packages.cargo.clone())
                        .expect("validated update target"),
                    mode: CargoPackageMode::UpdateCurrent,
                },
            );
        }
        if package_updates.npm == Some(true) {
            push_operation(
                phases,
                PlannerPhase::Updates,
                Operation::NpmPackageSet {
                    packages: packages
                        .and_then(|packages| packages.npm.clone())
                        .expect("validated update target"),
                    mode: NpmPackageMode::UpdateCurrent,
                },
            );
        }
        if package_updates.binaries == Some(true) {
            if let Some(binaries) = packages.and_then(|packages| packages.binaries.as_ref()) {
                for binary in binaries {
                    let is_github = matches!(
                        binary.source.resolve_native(platform.architecture),
                        Some(ResolvedNativeBinary::Github { .. })
                    );
                    if is_github {
                        let planned = plan_binary(binary, platform.architecture, BinaryPackageMode::Update)?
                            .expect("native GitHub source was resolved");
                        push_operation(phases, PlannerPhase::Updates, Operation::BinaryPackage(planned));
                    }
                }
            }
        }
    }
    if updates.fonts == Some(true) {
        push_operation(
            phases,
            PlannerPhase::Updates,
            Operation::NerdFonts {
                families: config
                    .fonts
                    .as_ref()
                    .and_then(|fonts| fonts.nerd.clone())
                    .expect("validated update target"),
                mode: NerdFontsMode::Update,
            },
        );
    }
    Ok(())
}

fn rust_selector_main(value: &str) -> RustToolchainSelector {
    if value == "stable" {
        RustToolchainSelector::Stable
    } else {
        RustToolchainSelector::Version(value.to_owned())
    }
}

fn go_selector_main(value: &str) -> GoToolchainSelector {
    if value == "latest" {
        GoToolchainSelector::Latest
    } else {
        GoToolchainSelector::Version(value.to_owned())
    }
}

fn node_selector_main(value: &str) -> NodeToolchainSelector {
    match value {
        "lts" => NodeToolchainSelector::Lts,
        "latest" => NodeToolchainSelector::Latest,
        value => NodeToolchainSelector::Version(value.to_owned()),
    }
}

fn enabled(state: EnabledDisabled) -> bool {
    match state {
        EnabledDisabled::Enabled => true,
        EnabledDisabled::Disabled => false,
    }
}
