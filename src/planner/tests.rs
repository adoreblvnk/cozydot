use super::*;
use crate::config::Config;

fn macos_platform() -> Platform {
    Platform::from_release_parts("macos".into(), "macos".into(), String::new(), String::new(), "none".into(), "aarch64")
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

fn assert_no_empty_collection_operations(operations: &[Operation]) {
    for operation in operations {
        let populated = match operation {
            Operation::AptPackages { packages }
            | Operation::AptBootstrapPackages { packages }
            | Operation::CargoPackageSet { packages }
            | Operation::NpmPackageSet { packages } => !packages.is_empty(),
            Operation::AptRepositoryPackages { packages, .. } => !packages.is_empty(),
            Operation::FlatpakEnsureApps { refs } => !refs.is_empty(),
            Operation::NerdFonts { families, .. } | Operation::UserNerdFonts { families, .. } => !families.is_empty(),
            Operation::Dotfiles { packages, .. } => !packages.is_empty(),
            Operation::VsCodeExtensionSet { extensions } | Operation::GnomeExtensions { extensions } => {
                !extensions.is_empty()
            }
            Operation::HomebrewPackages { formulae, casks } => !formulae.is_empty() || !casks.is_empty(),
            Operation::MacDefaults { settings } => !settings.is_empty(),
            _ => true,
        };
        assert!(populated, "empty synthetic operation: {operation:?}");
    }
}

fn apply_order_config() -> Config {
    Config::parse(
        r#"
version: 1.0.0
shared:
  tools:
    rust: stable
    go: latest
    node: lts
    python: "3.13"
  packages:
    cargo: [cargo-first, cargo-second]
    npm: [npm-first, npm-second]
  fonts:
    nerd: [OrderFont]
  dotfiles:
    packages: [shared-dotfiles]
  integrations:
    vscode:
      extensions: [publisher.first, publisher.second]
  updates:
    tools: {}
    packages: {}
os:
  linux:
    system:
      ensure_admin: true
      apt:
        unattended_upgrades: disabled
    packages:
      apt:
        install: [apt-first, apt-second]
        repositories:
          - name: repo-first
            key: https://example.com/first.asc
            key_path: /etc/apt/keyrings/first.asc
            urls:
              default: https://example.com/first
            suite: stable
            components: [main]
            conflicts:
              default: [old-first]
            packages: [repo-package-first]
          - name: repo-second
            key: https://example.com/second.asc
            key_path: /etc/apt/keyrings/second.asc
            urls:
              default: https://example.com/second
            path: ./
            packages: [repo-package-second]
      flatpak: [flatpak.first, flatpak.second]
      binaries:
        - name: appimage-first
          format: appimage
          source:
            provider: url
            urls:
              amd64: https://example.com/appimage-first.AppImage
        - name: deb-first
          format: deb
          source:
            provider: url
            urls:
              amd64: https://example.com/deb-first.deb
        - name: appimage-second
          format: appimage
          source:
            provider: url
            urls:
              amd64: https://example.com/appimage-second.AppImage
        - name: deb-second
          format: deb
          source:
            provider: url
            urls:
              amd64: https://example.com/deb-second.deb
    dotfiles:
      packages: [linux-dotfiles]
    integrations:
      docker:
        add_user_to_group: true
        logging:
          driver: local
          max_size: 5m
      virtualbox:
        add_user_to_group: true
    desktop:
      theme: dark
      terminal: workflow-terminal
      idle:
        timeout: 5m
        dim: false
      gnome:
        extensions: [extension-first, extension-second]
        dock: true
        rounded_corners: true
  macos:
    system:
      ensure_admin: true
      xcode:
        command_line_tools: true
      rosetta: true
    homebrew:
      formulae: [formula-first, formula-second]
      casks: [cask-first, cask-second]
    dotfiles:
      packages: [macos-dotfiles]
    desktop:
      appearance: dark
      dock:
        autohide: true
        show_recent_applications: false
      finder:
        show_filename_extensions: true
        show_hidden_files: false
      keyboard:
        key_repeat: 2
        initial_key_repeat: 15
      trackpad:
        tap_to_click: true
    updates:
      homebrew: {}
"#,
    )
    .unwrap()
}

#[test]
fn full_example_parses_macos_configuration() {
    let config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    assert_eq!(config.macos().homebrew.formulae[0], "cmake");
    assert_eq!(config.macos().desktop.appearance, Some(Theme::Dark));
}

#[test]
fn macos_planner_emits_native_operations() {
    let mut config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
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

    let dotfiles = plan_standalone_dotfiles(&config, &macos_platform(), Path::new("/tmp/dotfiles"), true).unwrap();
    assert!(matches!(dotfiles.as_slice(), [Operation::Dotfiles { replace: true, .. }]));
}

#[test]
fn apply_and_standalone_commands_share_dotfiles_planning() {
    let config = apply_order_config();
    for platform in [debian_platform(), macos_platform()] {
        let apply = plan_apply(&config, &platform, Path::new("/tmp/dotfiles")).unwrap();
        let apply_dotfiles =
            apply.into_iter().find(|operation| matches!(operation, Operation::Dotfiles { .. })).unwrap();

        assert_eq!(
            plan_standalone_dotfiles(&config, &platform, Path::new("/tmp/dotfiles"), false).unwrap(),
            vec![apply_dotfiles]
        );
    }
}

#[test]
fn macos_apply_workflow_preserves_exact_typed_capability_order() {
    let config = apply_order_config();
    let operations = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();

    assert_eq!(
        operations,
        vec![
            Operation::MacEnsureAdmin,
            Operation::XcodeCommandLineTools,
            Operation::Rosetta,
            Operation::HomebrewBootstrap,
            Operation::HomebrewPackages {
                formulae: vec!["formula-first".into(), "formula-second".into(), "stow".into()],
                casks: vec!["cask-first".into(), "cask-second".into()],
            },
            Operation::RustupBootstrap,
            Operation::RustToolchain { selector: Some("stable".into()), mode: ToolchainMode::EnsurePresent },
            Operation::GoToolchain {
                selector: GoToolchainSelector::Latest,
                architecture: Architecture::DarwinArm64,
                mode: ToolchainMode::EnsurePresent,
            },
            Operation::FnmBootstrap,
            Operation::NodeToolchain { selector: "lts".into(), mode: ToolchainMode::EnsurePresent },
            Operation::UvBootstrap,
            Operation::PythonToolchain { version: "3.13".into(), mode: ToolchainMode::EnsurePresent },
            Operation::CargoBinstallBootstrap,
            Operation::CargoPackageSet { packages: vec!["cargo-first".into(), "cargo-second".into()] },
            Operation::NpmPackageSet { packages: vec!["npm-first".into(), "npm-second".into()] },
            Operation::UserNerdFonts { families: vec!["OrderFont".into()], mode: NerdFontsMode::EnsurePresent },
            Operation::Dotfiles {
                root: PathBuf::from("/tmp/dotfiles"),
                packages: vec!["shared-dotfiles".into(), "macos-dotfiles".into()],
                replace: false,
            },
            Operation::VsCodeExtensionSet { extensions: vec!["publisher.first".into(), "publisher.second".into()] },
            Operation::MacDefaults {
                settings: vec![
                    crate::operations::macos::MacDefault::Appearance(true),
                    crate::operations::macos::MacDefault::DockAutohide(true),
                    crate::operations::macos::MacDefault::DockRecentApplications(false),
                    crate::operations::macos::MacDefault::FinderExtensions(true),
                    crate::operations::macos::MacDefault::FinderHiddenFiles(false),
                    crate::operations::macos::MacDefault::KeyRepeat(2),
                    crate::operations::macos::MacDefault::InitialKeyRepeat(15),
                    crate::operations::macos::MacDefault::TrackpadTapToClick(true),
                ],
            },
        ]
    );
}

#[test]
fn linux_apply_workflow_preserves_exact_typed_capability_order() {
    let config = apply_order_config();
    let operations = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();

    assert_eq!(
        operations,
        vec![
            Operation::EnsureAdmin,
            Operation::EnsureDebianAptComponents { release: "bookworm".into() },
            Operation::AptMetadataRefresh,
            Operation::UnattendedUpgrades { enabled: false },
            Operation::AptBootstrapPackages {
                packages: vec![
                    "ca-certificates".into(),
                    "curl".into(),
                    "dconf-cli".into(),
                    "flatpak".into(),
                    "fontconfig".into(),
                    "gnome-shell".into(),
                    "gnupg".into(),
                    "libglib2.0-bin".into(),
                    "stow".into(),
                    "tar".into(),
                    "unzip".into(),
                    "xz-utils".into(),
                ],
            },
            Operation::AptPackages { packages: vec!["apt-first".into(), "apt-second".into()] },
            Operation::AptRepository(Box::new(
                AptRepositoryOperation::new(
                    "repo-first",
                    "https://example.com/first.asc".into(),
                    "https://example.com/first".into(),
                    Architecture::Amd64,
                    Some("stable".into()),
                    vec!["main".into()],
                    None,
                    PathBuf::from("/etc/apt/keyrings/first.asc"),
                )
                .unwrap(),
            )),
            Operation::AptRepository(Box::new(
                AptRepositoryOperation::new(
                    "repo-second",
                    "https://example.com/second.asc".into(),
                    "https://example.com/second".into(),
                    Architecture::Amd64,
                    None,
                    Vec::new(),
                    Some("./".into()),
                    PathBuf::from("/etc/apt/keyrings/second.asc"),
                )
                .unwrap(),
            )),
            Operation::AptMetadataRefresh,
            Operation::AptRepositoryPackages {
                conflicts: vec!["old-first".into()],
                packages: vec!["repo-package-first".into()],
            },
            Operation::AptRepositoryPackages { conflicts: Vec::new(), packages: vec!["repo-package-second".into()] },
            Operation::FlatpakEnsureFlathub,
            Operation::FlatpakEnsureApps { refs: vec!["flatpak.first".into(), "flatpak.second".into()] },
            Operation::RustupBootstrap,
            Operation::RustToolchain { selector: Some("stable".into()), mode: ToolchainMode::EnsurePresent },
            Operation::GoToolchain {
                selector: GoToolchainSelector::Latest,
                architecture: Architecture::Amd64,
                mode: ToolchainMode::EnsurePresent,
            },
            Operation::FnmBootstrap,
            Operation::NodeToolchain { selector: "lts".into(), mode: ToolchainMode::EnsurePresent },
            Operation::UvBootstrap,
            Operation::PythonToolchain { version: "3.13".into(), mode: ToolchainMode::EnsurePresent },
            Operation::CargoBinstallBootstrap,
            Operation::CargoPackageSet { packages: vec!["cargo-first".into(), "cargo-second".into()] },
            Operation::NpmPackageSet { packages: vec!["npm-first".into(), "npm-second".into()] },
            Operation::BinaryPackage(BinaryPackageOperation::new(
                "deb-first".into(),
                BinaryFormat::Deb,
                Architecture::Amd64,
                BinarySourceOperation::Url { url: "https://example.com/deb-first.deb".into() },
            )),
            Operation::BinaryPackage(BinaryPackageOperation::new(
                "deb-second".into(),
                BinaryFormat::Deb,
                Architecture::Amd64,
                BinarySourceOperation::Url { url: "https://example.com/deb-second.deb".into() },
            )),
            Operation::Appimaged { architecture: Architecture::Amd64 },
            Operation::BinaryPackage(BinaryPackageOperation::new(
                "appimage-first".into(),
                BinaryFormat::Appimage,
                Architecture::Amd64,
                BinarySourceOperation::Url { url: "https://example.com/appimage-first.AppImage".into() },
            )),
            Operation::BinaryPackage(BinaryPackageOperation::new(
                "appimage-second".into(),
                BinaryFormat::Appimage,
                Architecture::Amd64,
                BinarySourceOperation::Url { url: "https://example.com/appimage-second.AppImage".into() },
            )),
            Operation::NerdFonts { families: vec!["OrderFont".into()], mode: NerdFontsMode::EnsurePresent },
            Operation::Dotfiles {
                root: PathBuf::from("/tmp/dotfiles"),
                packages: vec!["shared-dotfiles".into(), "linux-dotfiles".into()],
                replace: false,
            },
            Operation::DockerGroup,
            Operation::DockerLocalLog { max_size: Some("5m".into()) },
            Operation::VirtualBoxGroup,
            Operation::VsCodeExtensionSet { extensions: vec!["publisher.first".into(), "publisher.second".into()] },
            Operation::DesktopSetting {
                target: DesktopEnvironment::Gnome,
                setting: DesktopSetting::Theme(DesktopTheme::Dark),
            },
            Operation::DesktopSetting {
                target: DesktopEnvironment::Gnome,
                setting: DesktopSetting::Terminal("workflow-terminal".into()),
            },
            Operation::DesktopSetting {
                target: DesktopEnvironment::Gnome,
                setting: DesktopSetting::IdleTimeoutSeconds(300),
            },
            Operation::DesktopSetting { target: DesktopEnvironment::Gnome, setting: DesktopSetting::IdleDim(false) },
            Operation::GnomeExtensions { extensions: vec!["extension-first".into(), "extension-second".into()] },
            Operation::GnomeDock,
            Operation::GnomeRoundedCorners,
        ]
    );
}

#[test]
fn apply_derives_prerequisites_and_deduplicates_bootstraps() {
    let config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    let linux = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();
    let macos = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();
    let prerequisites = linux
        .iter()
        .find_map(|operation| match operation {
            Operation::AptBootstrapPackages { packages } => Some(packages),
            _ => None,
        })
        .unwrap();

    for package in ["ca-certificates", "curl", "flatpak", "fontconfig", "gnupg", "stow", "tar", "unzip", "xz-utils"] {
        assert!(prerequisites.iter().any(|candidate| candidate == package));
    }
    for operations in [&linux, &macos] {
        assert_eq!(operations.iter().filter(|operation| matches!(operation, Operation::RustupBootstrap)).count(), 1);
        assert_eq!(operations.iter().filter(|operation| matches!(operation, Operation::FnmBootstrap)).count(), 1);
        assert_eq!(
            operations.iter().filter(|operation| matches!(operation, Operation::CargoBinstallBootstrap)).count(),
            1
        );
        assert_no_empty_collection_operations(operations);
    }
}

#[test]
fn yaml_mapping_order_does_not_change_apply_or_update_order() {
    let source = include_str!("../../configs/full.yaml");
    let shared = source.find("\nshared:").unwrap();
    let os = source.find("\nos:\n").unwrap();
    let reordered = format!("{}{}{}", &source[..shared], &source[os..], &source[shared..os]);
    let original = Config::parse(source).unwrap();
    let reordered = Config::parse(&reordered).unwrap();

    assert_eq!(
        plan_apply(&original, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap(),
        plan_apply(&reordered, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap()
    );
    assert_eq!(
        plan_apply(&original, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap(),
        plan_apply(&reordered, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap()
    );
    assert_eq!(
        plan_update(&original, &debian_platform()).unwrap(),
        plan_update(&reordered, &debian_platform()).unwrap()
    );
    assert_eq!(plan_update(&original, &macos_platform()).unwrap(), plan_update(&reordered, &macos_platform()).unwrap());
}

#[test]
fn update_workflows_deduplicate_non_apt_manager_bootstraps() {
    let config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    for operations in
        [plan_update(&config, &debian_platform()).unwrap(), plan_update(&config, &macos_platform()).unwrap()]
    {
        assert_eq!(operations.iter().filter(|operation| **operation == Operation::RustupBootstrap).count(), 1);
        assert_eq!(operations.iter().filter(|operation| **operation == Operation::FnmBootstrap).count(), 1);
        assert_eq!(operations.iter().filter(|operation| **operation == Operation::UvBootstrap).count(), 1);
    }
}

#[test]
fn debian_apply_always_ensures_required_apt_components() {
    let config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    let operations = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();
    assert!(operations.contains(&Operation::EnsureDebianAptComponents { release: "bookworm".into() }));
}

#[test]
fn cli_preset_plans_on_a_headless_host() {
    let config = Config::parse(include_str!("../../configs/cli.yaml")).unwrap();
    let operations = plan_apply(&config, &headless_ubuntu_platform(), Path::new("/tmp/dotfiles")).unwrap();
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::VsCodeExtensionSet { .. })));
}

#[test]
fn macos_planner_skips_empty_portable_package_and_font_sets() {
    let mut config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    config.shared.packages.cargo = Some(Vec::new());
    config.shared.packages.npm = Some(Vec::new());
    config.shared.fonts.nerd = Some(Vec::new());
    config.os.macos.system.rosetta = Some(false);

    let apply = plan_apply(&config, &macos_platform(), Path::new("/tmp/dotfiles")).unwrap();
    assert!(!apply.iter().any(|operation| matches!(operation, Operation::CargoPackageSet { .. })));
    assert!(!apply.iter().any(|operation| matches!(operation, Operation::NpmPackageSet { .. })));
    assert!(!apply.iter().any(|operation| matches!(operation, Operation::Rosetta)));
    assert_no_empty_collection_operations(&apply);

    let update = plan_update(&config, &macos_platform()).unwrap();
    assert!(!update.iter().any(|operation| matches!(operation, Operation::UserNerdFonts { .. })));
}

#[test]
fn linux_planner_skips_empty_package_binary_and_font_sets() {
    let mut config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    let packages = &mut config.os.linux.packages;
    packages.apt.as_mut().unwrap().install = Some(Vec::new());
    packages.apt.as_mut().unwrap().repositories = Some(Vec::new());
    packages.flatpak = Some(Vec::new());
    packages.binaries = Some(Vec::new());
    config.shared.packages.cargo = Some(Vec::new());
    config.shared.packages.npm = Some(Vec::new());
    config.shared.fonts.nerd = Some(Vec::new());
    config.os.linux.system.ensure_admin = Some(false);
    config.os.linux.integrations.docker.as_mut().unwrap().add_user_to_group = Some(false);
    config.os.linux.integrations.virtualbox.as_mut().unwrap().add_user_to_group = Some(false);

    let operations = plan_apply(&config, &debian_platform(), Path::new("/tmp/dotfiles")).unwrap();
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::AptRepository(_))));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::AptRepositoryPackages { .. })));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::FlatpakEnsureApps { .. })));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::BinaryPackage(_))));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::CargoPackageSet { .. })));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::NpmPackageSet { .. })));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::NerdFonts { .. })));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::EnsureAdmin)));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::DockerGroup)));
    assert!(!operations.iter().any(|operation| matches!(operation, Operation::VirtualBoxGroup)));
    assert_no_empty_collection_operations(&operations);
}

#[test]
fn linux_update_workflow_preserves_exact_capability_order() {
    let config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    let operations = plan_update(&config, &debian_platform()).unwrap();

    assert_eq!(
        operations,
        vec![
            Operation::AptMetadataRefresh,
            Operation::AptUpgrade { policy: AptUpgradePolicy::Full },
            Operation::AptBootstrapPackages {
                packages: vec![
                    "ca-certificates".into(),
                    "curl".into(),
                    "flatpak".into(),
                    "fontconfig".into(),
                    "tar".into(),
                    "unzip".into(),
                    "xz-utils".into(),
                ],
            },
            Operation::FlatpakUpdateApps,
            Operation::RustupBootstrap,
            Operation::RustToolchain { selector: Some("stable".into()), mode: ToolchainMode::ConvergeLatest },
            Operation::GoToolchain {
                selector: GoToolchainSelector::Latest,
                architecture: Architecture::Amd64,
                mode: ToolchainMode::ConvergeLatest,
            },
            Operation::FnmBootstrap,
            Operation::NodeToolchain { selector: "lts".into(), mode: ToolchainMode::ConvergeLatest },
            Operation::UvBootstrap,
            Operation::PythonToolchain { version: "3.13".into(), mode: ToolchainMode::ConvergeLatest },
            Operation::CargoPackageUpdate,
            Operation::NpmPackageUpdate,
            Operation::NerdFonts { families: vec!["GeistMono".into()], mode: NerdFontsMode::Update },
        ]
    );
}

#[test]
fn macos_update_workflow_preserves_exact_capability_order() {
    let config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    let operations = plan_update(&config, &macos_platform()).unwrap();

    assert_eq!(
        operations,
        vec![
            Operation::HomebrewUpdate { formulae: true, casks: true },
            Operation::RustupBootstrap,
            Operation::RustToolchain { selector: Some("stable".into()), mode: ToolchainMode::ConvergeLatest },
            Operation::GoToolchain {
                selector: GoToolchainSelector::Latest,
                architecture: Architecture::DarwinArm64,
                mode: ToolchainMode::ConvergeLatest,
            },
            Operation::FnmBootstrap,
            Operation::NodeToolchain { selector: "lts".into(), mode: ToolchainMode::ConvergeLatest },
            Operation::UvBootstrap,
            Operation::PythonToolchain { version: "3.13".into(), mode: ToolchainMode::ConvergeLatest },
            Operation::CargoPackageUpdate,
            Operation::NpmPackageUpdate,
            Operation::UserNerdFonts { families: vec!["GeistMono".into()], mode: NerdFontsMode::Update },
        ]
    );
}

#[test]
fn apt_update_plan_contains_only_refresh_and_selected_upgrade() {
    let mut config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    config.os.linux.updates.as_mut().unwrap().apt = Some(AptUpdate::Full);
    config.os.linux.updates.as_mut().unwrap().flatpak = Some(false);
    config.shared.updates.tools.rust = Some(false);
    config.shared.updates.tools.go = Some(false);
    config.shared.updates.tools.node = Some(false);
    config.shared.updates.tools.python = Some(false);
    config.shared.updates.packages.cargo = Some(false);
    config.shared.updates.packages.npm = Some(false);
    config.shared.updates.fonts = Some(false);

    assert_eq!(
        plan_update(&config, &debian_platform()).unwrap(),
        [Operation::AptMetadataRefresh, Operation::AptUpgrade { policy: AptUpgradePolicy::Full },]
    );
}

#[test]
fn absent_and_false_update_controls_are_no_ops() {
    let mut config = Config::parse(include_str!("../../configs/full.yaml")).unwrap();
    config.os.linux.updates = None;
    config.os.macos.updates.homebrew.formulae = Some(false);
    config.os.macos.updates.homebrew.casks = Some(false);
    config.shared.updates.tools.rust = Some(false);
    config.shared.updates.tools.go = Some(false);
    config.shared.updates.tools.node = Some(false);
    config.shared.updates.tools.python = Some(false);
    config.shared.updates.packages.cargo = Some(false);
    config.shared.updates.packages.npm = Some(false);
    config.shared.updates.fonts = Some(false);

    assert!(plan_update(&config, &debian_platform()).unwrap().is_empty());
    assert!(plan_update(&config, &macos_platform()).unwrap().is_empty());
}
