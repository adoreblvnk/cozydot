use super::*;

const ROOT: &str = "/tmp/dotfiles";

fn linux() -> Platform {
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

fn macos() -> Platform {
    Platform::from_release_parts("macos".into(), "macos".into(), String::new(), String::new(), "none".into(), "aarch64")
        .unwrap()
}

fn full() -> Config {
    Config::parse(include_str!("../../configs/full.yaml")).unwrap()
}

fn names(operations: &[Operation]) -> Vec<&str> {
    operations
        .iter()
        .map(|operation| match operation {
            Operation::EnsureAdmin => "admin",
            Operation::EnsureDebianAptComponents { .. } => "apt-components",
            Operation::AptMetadataRefresh => "apt-refresh",
            Operation::UnattendedUpgrades { .. } => "unattended",
            Operation::UbuntuSnap { .. } => "snap",
            Operation::AptBootstrapPackages { .. } => "apt-prerequisites",
            Operation::AptPackages { .. } => "apt-packages",
            Operation::AptRepository(_) => "apt-repository",
            Operation::AptRepositoryPackages { .. } => "repository-packages",
            Operation::FlatpakEnsureFlathub => "flathub",
            Operation::FlatpakEnsureApps { .. } => "flatpak-apps",
            Operation::RustupBootstrap => "rustup",
            Operation::RustToolchain { .. } => "rust",
            Operation::GoToolchain { .. } => "go",
            Operation::FnmBootstrap => "fnm",
            Operation::NodeToolchain { .. } => "node",
            Operation::UvBootstrap => "uv",
            Operation::PythonToolchain { .. } => "python",
            Operation::CargoBinstallBootstrap => "cargo-binstall",
            Operation::CargoPackageSet { .. } => "cargo-packages",
            Operation::NpmPackageSet { .. } => "npm-packages",
            Operation::BinaryPackage(_) => "binary",
            Operation::Appimaged { .. } => "appimaged",
            Operation::NerdFonts { .. } => "fonts",
            Operation::UserNerdFonts { .. } => "user-fonts",
            Operation::Dotfiles { .. } => "dotfiles",
            Operation::DockerGroup => "docker-group",
            Operation::DockerLocalLog { .. } => "docker-log",
            Operation::VirtualBoxGroup => "virtualbox-group",
            Operation::VsCodeExtensionSet { .. } => "vscode",
            Operation::DesktopSetting { .. } => "desktop-setting",
            Operation::GnomeExtensions { .. } => "gnome-extensions",
            Operation::GnomeDock => "gnome-dock",
            Operation::GnomeRoundedCorners => "gnome-corners",
            Operation::MacEnsureAdmin => "mac-admin",
            Operation::XcodeCommandLineTools => "xcode",
            Operation::Rosetta => "rosetta",
            Operation::HomebrewBootstrap => "homebrew",
            Operation::HomebrewPackages { .. } => "homebrew-packages",
            Operation::MacDefaults { .. } => "mac-defaults",
            Operation::AptUpgrade { .. } => "apt-upgrade",
            Operation::FlatpakUpdateApps => "flatpak-update",
            Operation::CargoPackageUpdate => "cargo-update",
            Operation::NpmPackageUpdate => "npm-update",
            Operation::HomebrewUpdate { .. } => "homebrew-update",
        })
        .collect()
}

#[test]
fn apply_operation_order_is_stable() {
    let config = full();
    assert_eq!(
        names(&plan_apply(&config, &linux(), Path::new(ROOT)).unwrap()),
        [
            "admin",
            "apt-components",
            "apt-refresh",
            "unattended",
            "apt-prerequisites",
            "apt-packages",
            "apt-repository",
            "apt-repository",
            "apt-repository",
            "apt-repository",
            "apt-repository",
            "apt-repository",
            "apt-repository",
            "apt-refresh",
            "repository-packages",
            "repository-packages",
            "repository-packages",
            "repository-packages",
            "repository-packages",
            "repository-packages",
            "repository-packages",
            "flathub",
            "flatpak-apps",
            "rustup",
            "rust",
            "go",
            "fnm",
            "node",
            "uv",
            "python",
            "cargo-binstall",
            "cargo-packages",
            "npm-packages",
            "binary",
            "binary",
            "binary",
            "appimaged",
            "binary",
            "binary",
            "fonts",
            "dotfiles",
            "docker-group",
            "docker-log",
            "virtualbox-group",
            "vscode",
            "desktop-setting",
            "desktop-setting",
            "desktop-setting",
            "desktop-setting",
            "gnome-extensions",
            "gnome-dock",
            "gnome-corners",
        ]
    );
    assert_eq!(
        names(&plan_apply(&config, &macos(), Path::new(ROOT)).unwrap()),
        [
            "mac-admin",
            "xcode",
            "homebrew",
            "homebrew-packages",
            "rustup",
            "rust",
            "go",
            "fnm",
            "node",
            "uv",
            "python",
            "cargo-binstall",
            "cargo-packages",
            "npm-packages",
            "user-fonts",
            "dotfiles",
            "vscode",
            "mac-defaults",
        ]
    );
}

#[test]
fn update_operation_order_is_stable() {
    let config = full();
    assert_eq!(
        names(&plan_update(&config, &linux()).unwrap()),
        [
            "apt-refresh",
            "apt-upgrade",
            "apt-prerequisites",
            "flatpak-update",
            "rustup",
            "rust",
            "go",
            "fnm",
            "node",
            "uv",
            "python",
            "cargo-update",
            "npm-update",
            "fonts",
        ]
    );
    assert_eq!(
        names(&plan_update(&config, &macos()).unwrap()),
        [
            "homebrew-update",
            "rustup",
            "rust",
            "go",
            "fnm",
            "node",
            "uv",
            "python",
            "cargo-update",
            "npm-update",
            "user-fonts",
        ]
    );
}

#[test]
fn prerequisites_bootstraps_and_dotfiles_are_deduplicated_and_ordered() {
    let shared =
        ["bat", "bin", "bottom", "fastfetch", "git", "gnupg", "opencode", "starship", "vscode", "wezterm", "yazi"];
    for platform in [linux(), macos()] {
        let operations = plan_apply(&full(), &platform, Path::new(ROOT)).unwrap();
        let operation_names = names(&operations);
        for bootstrap in ["rustup", "fnm", "uv", "cargo-binstall"] {
            assert_eq!(operation_names.iter().filter(|name| **name == bootstrap).count(), 1);
        }
        let packages = operations
            .iter()
            .find_map(|operation| match operation {
                Operation::Dotfiles { packages, .. } => Some(packages),
                _ => None,
            })
            .unwrap();
        let expected = shared.into_iter().chain([if platform.is_macos() { "zsh" } else { "bash" }]).collect::<Vec<_>>();
        assert_eq!(packages, &expected);
        assert_eq!(packages.iter().collect::<BTreeSet<_>>().len(), packages.len());
    }
    let operations = plan_apply(&full(), &linux(), Path::new(ROOT)).unwrap();
    let prerequisites = operations.iter().find_map(|operation| match operation {
        Operation::AptBootstrapPackages { packages } => Some(packages),
        _ => None,
    });
    let prerequisites = prerequisites.unwrap();
    assert_eq!(prerequisites.iter().collect::<BTreeSet<_>>().len(), prerequisites.len());
}

#[test]
fn shared_capability_payloads_are_stable() {
    let config = full();
    for platform in [linux(), macos()] {
        let apply = plan_apply(&config, &platform, Path::new(ROOT)).unwrap();
        assert!(apply.contains(&Operation::RustToolchain {
            selector: Some("stable".into()),
            mode: ToolchainMode::EnsurePresent,
        }));
        assert!(apply.contains(&Operation::GoToolchain {
            selector: GoToolchainSelector::Latest,
            architecture: platform.architecture,
            mode: ToolchainMode::EnsurePresent,
        }));
        assert!(
            apply.contains(&Operation::NodeToolchain { selector: "lts".into(), mode: ToolchainMode::EnsurePresent })
        );
        assert!(
            apply.contains(&Operation::PythonToolchain { version: "3.13".into(), mode: ToolchainMode::EnsurePresent })
        );
        assert!(
            apply.contains(&Operation::CargoPackageSet { packages: config.shared.packages.cargo.clone().unwrap() })
        );
        assert!(apply.contains(&Operation::NpmPackageSet { packages: config.shared.packages.npm.clone().unwrap() }));
        assert!(apply.contains(&Operation::VsCodeExtensionSet {
            extensions: config.shared.integrations.vscode.extensions.clone(),
        }));
        let font = if platform.is_macos() {
            Operation::UserNerdFonts { families: vec!["GeistMono".into()], mode: NerdFontsMode::EnsurePresent }
        } else {
            Operation::NerdFonts { families: vec!["GeistMono".into()], mode: NerdFontsMode::EnsurePresent }
        };
        assert!(apply.contains(&font));

        let update = plan_update(&config, &platform).unwrap();
        assert!(update.contains(&Operation::RustToolchain {
            selector: Some("stable".into()),
            mode: ToolchainMode::ConvergeLatest,
        }));
        assert!(update.contains(&Operation::GoToolchain {
            selector: GoToolchainSelector::Latest,
            architecture: platform.architecture,
            mode: ToolchainMode::ConvergeLatest,
        }));
        assert!(
            update.contains(&Operation::NodeToolchain { selector: "lts".into(), mode: ToolchainMode::ConvergeLatest })
        );
        assert!(
            update
                .contains(&Operation::PythonToolchain { version: "3.13".into(), mode: ToolchainMode::ConvergeLatest })
        );
    }
}

#[test]
fn absent_and_false_updates_are_no_ops_and_python_defaults_are_platform_specific() {
    let mut config = full();
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
    assert!(plan_update(&config, &linux()).unwrap().is_empty());
    assert!(plan_update(&config, &macos()).unwrap().is_empty());

    config.shared.updates.tools.python = Some(true);
    config.shared.tools.python = None;
    for (platform, expected) in [(linux(), "3"), (macos(), "latest")] {
        assert!(
            plan_update(&config, &platform).unwrap().contains(&Operation::PythonToolchain {
                version: expected.into(),
                mode: ToolchainMode::ConvergeLatest,
            })
        );
    }
}

#[test]
fn yaml_mapping_order_does_not_change_plans() {
    let source = include_str!("../../configs/full.yaml");
    let shared = source.find("\nshared:").unwrap();
    let os = source.find("\nos:\n").unwrap();
    let reordered = Config::parse(&format!("{}{}{}", &source[..shared], &source[os..], &source[shared..os])).unwrap();
    let original = full();
    for platform in [linux(), macos()] {
        assert_eq!(
            plan_apply(&original, &platform, Path::new(ROOT)).unwrap(),
            plan_apply(&reordered, &platform, Path::new(ROOT)).unwrap()
        );
        assert_eq!(plan_update(&original, &platform).unwrap(), plan_update(&reordered, &platform).unwrap());
    }
}
