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
    CargoBinstallBootstrap,
    LanguagePackages,
    AppImageManager,
    BinaryPackages,
    Fonts,
    Dotfiles,
    Integrations,
    Desktop,
    Updates,
}

pub fn plan_apply(config: &Config, platform: &Platform, dotfiles_root: &Path) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    validate_binary_integrations(config, platform.architecture)?;
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
        (PlannerPhase::CargoBinstallBootstrap, Vec::new()),
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
    let mut needs_apt_refresh = false;

    let packages = config.packages.as_ref();
    let apt = packages.and_then(|packages| packages.apt.as_ref());

    if config.system.as_ref().is_some_and(|system| system.ensure_admin == Some(true)) {
        push_operation(&mut phases, PlannerPhase::AdministrativeVerification, Operation::EnsureAdmin);
    }

    if let Some(sources) =
        config.system.as_ref().and_then(|system| system.apt.as_ref()).and_then(|apt| apt.sources.as_ref())
        && sources.mode == SourceMode::Managed
    {
        let managed =
            sources.resolve_managed(platform, identity)?.expect("managed source resolution returns an intent");
        push_operation(&mut phases, PlannerPhase::OfficialAptSources, Operation::ManagedAptSources(managed));
    }

    if let Some(repositories) = apt.and_then(|apt| apt.repositories.as_ref()).filter(|values| !values.is_empty()) {
        for repository in repositories {
            let Some(operation) = plan_repository(repository, platform, identity) else {
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
                    Operation::AptPackages { packages: repository.packages.clone() },
                );
            }
            needs_apt_refresh = true;
        }
    }

    plan_system_states(config, platform, &mut phases, &mut needs_apt_refresh);

    if let Some(remove) = apt.and_then(|apt| apt.remove.as_ref()).filter(|values| !values.is_empty()) {
        push_operation(&mut phases, PlannerPhase::AptPurge, Operation::AptPurge { packages: remove.clone() });
        needs_apt_refresh = true;
    }
    if let Some(install) = apt.and_then(|apt| apt.install.as_ref()).filter(|values| !values.is_empty()) {
        push_operation(&mut phases, PlannerPhase::AptPackages, Operation::AptPackages { packages: install.clone() });
        needs_apt_refresh = true;
    }

    if let Some(applications) =
        packages.and_then(|packages| packages.flatpak.as_ref()).filter(|values| !values.is_empty())
    {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Flatpak);
        push_operation(
            &mut phases,
            PlannerPhase::FlatpakApplications,
            Operation::FlatpakEnsureApps { refs: applications.clone() },
        );
    }

    plan_tools(config, platform, &mut phases, &mut prerequisites, &mut managers);

    if let Some(cargo) = packages.and_then(|packages| packages.cargo.as_ref()).filter(|values| !values.is_empty()) {
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
    if let Some(npm) = packages.and_then(|packages| packages.npm.as_ref()).filter(|values| !values.is_empty()) {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Fnm);
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::NpmPackageSet { packages: npm.clone(), mode: NpmPackageMode::EnsurePresent },
        );
    }

    if let Some(binaries) = packages.and_then(|packages| packages.binaries.as_ref()).filter(|values| !values.is_empty())
    {
        for binary in binaries {
            let Some(planned) = plan_binary(binary, platform.architecture) else {
                continue;
            };
            prerequisites.insert("ca-certificates");
            prerequisites.insert("curl");
            if binary.format == BinaryFormat::Deb {
                needs_apt_refresh = true;
            }
            push_operation(&mut phases, PlannerPhase::BinaryPackages, Operation::BinaryPackage(planned));
        }
    }

    if let Some(fonts) = config.fonts.as_ref().and_then(|fonts| fonts.nerd.as_ref()).filter(|values| !values.is_empty())
    {
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

    if let Some(dotfiles) = config.dotfiles.as_ref().filter(|dotfiles| !dotfiles.packages.is_empty()) {
        if dotfiles_root.as_os_str().is_empty() {
            anyhow::bail!("dotfiles root must not be empty");
        }
        prerequisites.insert("stow");
        push_operation(
            &mut phases,
            PlannerPhase::Dotfiles,
            Operation::Dotfiles { root: dotfiles_root.to_path_buf(), packages: dotfiles.packages.clone() },
        );
    }

    plan_integrations(config, platform, &mut phases, &mut prerequisites);
    plan_desktop(config, platform, &mut phases, &mut prerequisites);
    if needs_apt_refresh {
        push_operation(&mut phases, PlannerPhase::AptMetadataRefresh, Operation::AptMetadataRefresh);
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
            ManagerBootstrap::CargoBinstall => {
                (PlannerPhase::CargoBinstallBootstrap, Operation::CargoBinstallBootstrap)
            }
            ManagerBootstrap::Fnm => (PlannerPhase::ManagerBootstraps, Operation::FnmBootstrap),
            ManagerBootstrap::Uv => (PlannerPhase::ManagerBootstraps, Operation::UvBootstrap),
        };
        push_operation(&mut phases, phase, operation);
    }

    Ok(phases.into_iter().flat_map(|(_, operations)| operations).collect())
}

fn push_operation(phases: &mut [(PlannerPhase, Vec<Operation>)], phase: PlannerPhase, op: Operation) {
    phases.iter_mut().find(|(p, _)| *p == phase).expect("phase exists").1.push(op);
}

fn plan_repository(
    repository: &crate::config::Repository,
    platform: &Platform,
    identity: crate::config::PlatformIdentity,
) -> Option<AptRepositoryOperation> {
    let (key, source_url) = select_distro_map(&repository.urls, identity.distro, identity.upstream)?;
    let suite = repository.suite.as_ref().map(|suite| {
        if suite == "system" {
            selected_repository_codename(key, platform, identity.distro).to_owned()
        } else {
            suite.clone()
        }
    });
    Some(AptRepositoryOperation::new(
        repository.name.clone(),
        repository.key.clone(),
        source_url.clone(),
        platform.architecture,
        suite,
        repository.components.clone().unwrap_or_default(),
        repository.path.clone(),
        PathBuf::from(&repository.key_path),
    ))
}

fn plan_binary(binary: &crate::config::BinaryPackage, architecture: Architecture) -> Option<BinaryPackageOperation> {
    let source = match &binary.source {
        BinarySource::Github { repository, assets } => {
            let selector = assets.get(architecture)?;
            BinarySourceOperation::GithubLatest { repository: repository.clone(), selector: selector.to_owned() }
        }
        BinarySource::Url { urls } => BinarySourceOperation::Url { url: urls.get(architecture)?.clone() },
    };
    Some(BinaryPackageOperation::new(binary.name.clone(), binary.format, architecture, source))
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

fn plan_tools(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
    managers: &mut BTreeSet<ManagerBootstrap>,
) {
    let Some(tools) = &config.tools else { return };
    if let Some(selector) = tools.rust.as_deref() {
        prerequisites.insert("ca-certificates");
        prerequisites.insert("curl");
        managers.insert(ManagerBootstrap::Rustup);
        push_operation(
            phases,
            PlannerPhase::LanguageToolchains,
            Operation::RustToolchain { selector: selector.to_owned(), mode: ToolchainMode::EnsurePresent },
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

fn plan_integrations(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
) {
    let Some(integrations) = &config.integrations else { return };
    if integrations.appimaged == Some(true) {
        prerequisites.extend(["ca-certificates", "curl"]);
        push_operation(
            phases,
            PlannerPhase::AppImageManager,
            Operation::Appimaged { architecture: platform.architecture },
        );
    }
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
    if let Some(extensions) =
        integrations.vscode.as_ref().map(|vscode| vscode.extensions.clone()).filter(|values| !values.is_empty())
    {
        push_operation(phases, PlannerPhase::Integrations, Operation::VsCodeExtensionSet { extensions });
    }
}

fn plan_desktop(
    config: &Config,
    platform: &Platform,
    phases: &mut [(PlannerPhase, Vec<Operation>)],
    prerequisites: &mut BTreeSet<&'static str>,
) {
    let Some(desktop) = config.desktop.as_ref().filter(|desktop| desktop.has_intent()) else { return };
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

pub fn plan_update(config: &Config, platform: &Platform) -> Result<Vec<Operation>> {
    config.validate_for_platform(platform)?;
    validate_binary_integrations(config, platform.architecture)?;
    let Some(updates) = &config.updates else {
        return Ok(Vec::new());
    };
    let mut phases = [
        (PlannerPhase::SystemPrerequisites, Vec::new()),
        (PlannerPhase::ManagerBootstraps, Vec::new()),
        (PlannerPhase::ThirdPartyRepositories, Vec::new()),
        (PlannerPhase::AptMetadataRefresh, Vec::new()),
        (PlannerPhase::FlatpakApplications, Vec::new()),
        (PlannerPhase::LanguageToolchains, Vec::new()),
        (PlannerPhase::LanguagePackages, Vec::new()),
        (PlannerPhase::Fonts, Vec::new()),
        (PlannerPhase::Updates, Vec::new()),
    ];
    let packages = config.packages.as_ref();
    let tools = config.tools.as_ref();
    let mut prerequisites = BTreeSet::new();
    let mut managers = BTreeSet::new();

    if let Some(policy) = updates.apt {
        let identity = resolve_platform_identity(platform)?;
        let apt = packages.and_then(|packages| packages.apt.as_ref());
        let mut configured = BTreeSet::new();
        if let Some(install) = apt.and_then(|apt| apt.install.as_ref()) {
            configured.extend(install.iter().cloned());
        }
        if let Some(repositories) = apt.and_then(|apt| apt.repositories.as_ref()) {
            for repository in repositories {
                let Some(operation) = plan_repository(repository, platform, identity) else { continue };
                if repository.packages.is_empty() {
                    continue;
                }
                prerequisites.extend(["ca-certificates", "curl", "gnupg"]);
                configured.extend(repository.packages.iter().cloned());
                push_operation(
                    &mut phases,
                    PlannerPhase::ThirdPartyRepositories,
                    Operation::AptRepository(Box::new(operation)),
                );
            }
        }
        if config.system.as_ref().and_then(|system| system.ubuntu.as_ref()).is_some_and(|ubuntu| ubuntu.codecs)
            && platform.upstream == "ubuntu"
        {
            configured.insert("ubuntu-restricted-extras".into());
        }

        push_operation(&mut phases, PlannerPhase::AptMetadataRefresh, Operation::AptMetadataRefresh);
        if !configured.is_empty() {
            push_operation(
                &mut phases,
                PlannerPhase::Updates,
                Operation::AptPackages { packages: configured.into_iter().collect() },
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
    if updates.flatpak == Some(true) {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Flatpak);
        let refs = packages.and_then(|packages| packages.flatpak.clone()).expect("validated Flatpak update targets");
        push_operation(&mut phases, PlannerPhase::FlatpakApplications, Operation::FlatpakUpdateApps { refs });
    }

    let tool_updates = updates.tools.as_ref();
    let rust_update = tool_updates.is_some_and(|updates| updates.rust == Some(true));
    let go_update = tool_updates.is_some_and(|updates| updates.go == Some(true));
    let node_update = tool_updates.is_some_and(|updates| updates.node == Some(true));
    let python_update = tool_updates.is_some_and(|updates| updates.python == Some(true));
    let package_updates = updates.packages.as_ref();
    let cargo_update = package_updates.is_some_and(|updates| updates.cargo == Some(true));
    let npm_update = package_updates.is_some_and(|updates| updates.npm == Some(true));

    if rust_update || cargo_update {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Rustup);
        let selector = tools.and_then(|tools| tools.rust.clone()).expect("validated Rust selector");
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::RustToolchain {
                selector,
                mode: if rust_update { ToolchainMode::ConvergeLatest } else { ToolchainMode::EnsurePresent },
            },
        );
    }
    if go_update {
        prerequisites.extend(["ca-certificates", "curl", "tar"]);
        let selector = tools.and_then(|tools| tools.go.as_deref()).expect("validated Go selector");
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
    if node_update || npm_update {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Fnm);
        let selector = tools.and_then(|tools| tools.node.clone()).expect("validated Node selector");
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::NodeToolchain {
                selector,
                mode: if node_update { ToolchainMode::ConvergeLatest } else { ToolchainMode::EnsurePresent },
            },
        );
    }
    if python_update {
        prerequisites.extend(["ca-certificates", "curl"]);
        managers.insert(ManagerBootstrap::Uv);
        let version = tools.and_then(|tools| tools.python.clone()).expect("validated Python selector");
        push_operation(
            &mut phases,
            PlannerPhase::LanguageToolchains,
            Operation::PythonToolchain { version, mode: ToolchainMode::ConvergeLatest },
        );
    }
    if cargo_update {
        let configured = packages.and_then(|packages| packages.cargo.clone()).expect("validated Cargo update targets");
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::CargoPackageSet { packages: configured, mode: CargoPackageMode::UpdateCurrent },
        );
    }
    if npm_update {
        let configured = packages.and_then(|packages| packages.npm.clone()).expect("validated npm update targets");
        push_operation(
            &mut phases,
            PlannerPhase::LanguagePackages,
            Operation::NpmPackageSet { packages: configured, mode: NpmPackageMode::UpdateCurrent },
        );
    }
    if updates.fonts == Some(true) {
        prerequisites.extend(["ca-certificates", "curl", "tar", "xz-utils", "fontconfig"]);
        let families =
            config.fonts.as_ref().and_then(|fonts| fonts.nerd.clone()).expect("validated Nerd Font update targets");
        push_operation(
            &mut phases,
            PlannerPhase::Fonts,
            Operation::NerdFonts { families, mode: NerdFontsMode::Update },
        );
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

fn validate_binary_integrations(config: &Config, architecture: Architecture) -> Result<()> {
    let has_appimage =
        config.packages.as_ref().and_then(|packages| packages.binaries.as_ref()).is_some_and(|binaries| {
            binaries
                .iter()
                .any(|binary| binary.format == BinaryFormat::Appimage && plan_binary(binary, architecture).is_some())
        });
    if has_appimage && config.integrations.as_ref().and_then(|integrations| integrations.appimaged) != Some(true) {
        anyhow::bail!("packages.binaries: AppImages require integrations.appimaged: true");
    }
    Ok(())
}

fn go_selector_main(value: &str) -> GoToolchainSelector {
    if value == "latest" { GoToolchainSelector::Latest } else { GoToolchainSelector::Version(value.to_owned()) }
}

fn enabled(state: EnabledDisabled) -> bool {
    state == EnabledDisabled::Enabled
}
