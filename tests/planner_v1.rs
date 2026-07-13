use cozydot::{
    config::v1::ConfigV1,
    planner::v1::{
        plan, AptSourcesIntent, AptUpdatePolicy, AptUpdateTarget, DesktopAction,
        DirectPackageIntent, DotfilesConflictPolicy, FlatpakUpdateScope, GoSelector,
        IntegrationAction, NodeSelector, PlannedAction, Prerequisite, RustSelector, SystemAction,
        ToolInstall, ToolUpdate, UpdateAction,
    },
    platform::Platform,
};
use std::path::{Path, PathBuf};

const MINIMAL: &str = include_str!("fixtures/config-v1-minimal.yaml");
const FULL: &str = include_str!("fixtures/config-v1-full.yaml");

fn platform(distro: &str, upstream: &str, codename: &str, desktop: &str, arch: &str) -> Platform {
    Platform::from_parts(
        distro.into(),
        upstream.into(),
        codename.into(),
        desktop.into(),
        arch,
    )
    .unwrap()
}

fn ubuntu(arch: &str) -> Platform {
    platform("ubuntu", "ubuntu", "noble", "gnome", arch)
}

fn update_actions(actions: &[PlannedAction]) -> Vec<&UpdateAction> {
    actions
        .iter()
        .filter_map(|action| match action {
            PlannedAction::Update(update) => Some(update),
            _ => None,
        })
        .collect()
}

fn action_domain(action: &PlannedAction) -> &'static str {
    match action {
        PlannedAction::Prepare(_) => "prepare",
        PlannedAction::System(_) => "system",
        PlannedAction::RemovePackages(_) => "remove",
        PlannedAction::Repository(_) => "repository",
        PlannedAction::AptPackages(_) => "apt",
        PlannedAction::Flatpak(_) => "flatpak",
        PlannedAction::Tool(_) => "tool",
        PlannedAction::CargoPackages(_) => "cargo",
        PlannedAction::NpmPackages(_) => "npm",
        PlannedAction::DirectPackage(_) => "direct",
        PlannedAction::NerdFonts(_) => "fonts",
        PlannedAction::Dotfiles(_) => "dotfiles",
        PlannedAction::Integration(_) => "integration",
        PlannedAction::Desktop(_) => "desktop",
        PlannedAction::Update(_) => "update",
    }
}

#[test]
fn minimal_config_is_an_empty_typed_plan() {
    let config = ConfigV1::parse(MINIMAL).unwrap();
    assert!(plan(&config, &ubuntu("amd64"), Path::new("/dotfiles"))
        .unwrap()
        .actions
        .is_empty());
}

#[test]
fn canonical_full_fixture_has_exact_domains_and_one_shared_preparation() {
    let config = ConfigV1::parse(FULL).unwrap();
    let actions = plan(&config, &ubuntu("amd64"), Path::new("/repo/dotfiles"))
        .unwrap()
        .actions;
    assert_eq!(
        actions.iter().map(action_domain).collect::<Vec<_>>(),
        [
            "prepare",
            "system",
            "system",
            "system",
            "system",
            "system",
            "remove",
            "repository",
            "apt",
            "flatpak",
            "tool",
            "tool",
            "tool",
            "tool",
            "cargo",
            "npm",
            "direct",
            "fonts",
            "dotfiles",
            "integration",
            "integration",
            "integration",
            "integration",
            "desktop",
            "desktop",
            "desktop",
            "desktop",
            "desktop",
            "desktop",
            "desktop",
            "update",
            "update",
            "update",
            "update",
            "update",
            "update",
            "update",
            "update",
        ]
    );
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(action, PlannedAction::Prepare(_)))
            .count(),
        1
    );
    let PlannedAction::Prepare(preparation) = &actions[0] else {
        panic!("preparation must be first")
    };
    assert!(preparation.apt_metadata);
    assert_eq!(preparation.prerequisites.len(), 14);
    for prerequisite in [
        Prerequisite::NetworkDownload,
        Prerequisite::AptRepositorySupport,
        Prerequisite::FlatpakFlathub,
        Prerequisite::RustupCargoBinstall,
        Prerequisite::GoArchives,
        Prerequisite::FnmNpm,
        Prerequisite::Uv,
        Prerequisite::Stow,
        Prerequisite::DirectAppImage,
        Prerequisite::NerdFonts,
        Prerequisite::DockerIntegration,
        Prerequisite::VirtualBoxIntegration,
        Prerequisite::VsCodeIntegration,
        Prerequisite::GnomeTools,
    ] {
        assert!(preparation.prerequisites.contains(&prerequisite));
    }
}

#[test]
fn canonical_fixture_selects_native_direct_intent_on_amd64_and_arm64() {
    let config = ConfigV1::parse(FULL).unwrap();
    for (arch, include, debian_arch) in [
        ("amd64", "Obsidian-*.AppImage", "amd64"),
        ("arm64", "Obsidian-*-arm64.AppImage", "arm64"),
    ] {
        let actions = plan(&config, &ubuntu(arch), Path::new("."))
            .unwrap()
            .actions;
        let direct = actions
            .iter()
            .find_map(|action| match action {
                PlannedAction::DirectPackage(package) => Some(package),
                _ => None,
            })
            .unwrap();
        assert_eq!(direct.name, "obsidian");
        assert_eq!(direct.provides, ["obsidian"]);
        assert_eq!(direct.source.selector.include, include);
        assert_eq!(direct.source.repository, "obsidianmd/obsidian-releases");
        let repository = actions
            .iter()
            .find_map(|action| match action {
                PlannedAction::Repository(repository) => Some(repository),
                _ => None,
            })
            .unwrap();
        assert_eq!(repository.architecture, debian_arch);
        assert_eq!(repository.suite, "stable");
    }
    for arch in ["arm32", "riscv64"] {
        let error = plan(&config, &ubuntu(arch), Path::new("."))
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("packages.direct[0].source.assets.{arch}")));
    }
}

#[test]
fn system_controls_map_all_distros_and_skip_ubuntu_controls_on_debian_family() {
    let config = ConfigV1::parse(
        "schema: 1\nsystem:\n  apt:\n    sources: managed\n    components: [main]\n    unattended_upgrades: false\n  ensure_admin: false\n  ubuntu:\n    snap: false\n    codecs: true",
    )
    .unwrap();
    for (distro, upstream) in [
        ("ubuntu", "ubuntu"),
        ("linuxmint", "ubuntu"),
        ("pop", "ubuntu"),
        ("zorin", "ubuntu"),
        ("deepin", "ubuntu"),
        ("debian", "debian"),
        ("kali", "debian"),
        ("tails", "debian"),
    ] {
        let actions = plan(
            &config,
            &platform(distro, upstream, "release", "none", "amd64"),
            Path::new("."),
        )
        .unwrap()
        .actions;
        let system = actions
            .iter()
            .filter_map(|action| match action {
                PlannedAction::System(action) => Some(action),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            system[0],
            SystemAction::AptSources(AptSourcesIntent::Managed { distro: planned, upstream: family, codename, .. })
                if planned == distro && family == upstream && codename == "release"
        ));
        assert!(system.contains(&&SystemAction::EnsureAdmin { enabled: false }));
        assert!(system.contains(&&SystemAction::UnattendedUpgrades { enabled: false }));
        assert_eq!(
            system
                .iter()
                .filter(|action| matches!(
                    action,
                    SystemAction::UbuntuSnap { .. } | SystemAction::UbuntuCodecs
                ))
                .count(),
            if upstream == "ubuntu" { 2 } else { 0 }
        );
    }
}

#[test]
fn preserve_and_managed_sources_are_distinct_and_managed_requires_codename() {
    let preserve = ConfigV1::parse("schema: 1\nsystem:\n  apt:\n    sources: preserve").unwrap();
    assert!(plan(&preserve, &ubuntu("amd64"), Path::new("."))
        .unwrap()
        .actions
        .contains(&PlannedAction::System(SystemAction::AptSources(
            AptSourcesIntent::Preserve
        ))));

    let managed = ConfigV1::parse(
        "schema: 1\nsystem:\n  apt:\n    sources: managed\n    components: [main, universe]",
    )
    .unwrap();
    let actions = plan(&managed, &ubuntu("amd64"), Path::new("."))
        .unwrap()
        .actions;
    assert!(actions.iter().any(|action| matches!(
        action,
        PlannedAction::System(SystemAction::AptSources(AptSourcesIntent::Managed {
            components: Some(components), ..
        })) if components.len() == 2
    )));
    let error = plan(
        &managed,
        &platform("ubuntu", "ubuntu", "", "none", "amd64"),
        Path::new("."),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("system.apt.sources"));
    assert!(error.contains("codename"));
}

#[test]
fn repository_intent_uses_precedence_suite_paths_and_native_architecture() {
    let config = ConfigV1::parse(
        "schema: 1\npackages:\n  repositories:\n    - name: 'Vendor Repo!'\n      key: https://keys.example/key.asc\n      source:\n        urls:\n          default: https://default.example/apt\n          ubuntu: https://ubuntu.example/apt\n        suite: system\n        components: [stable, main]\n      packages: [vendor, helper]",
    )
    .unwrap();
    let actions = plan(&config, &ubuntu("arm64"), Path::new("."))
        .unwrap()
        .actions;
    let repository = actions
        .iter()
        .find_map(|action| match action {
            PlannedAction::Repository(repository) => Some(repository),
            _ => None,
        })
        .unwrap();
    assert_eq!(repository.key_url, "https://keys.example/key.asc");
    assert_eq!(repository.source_url, "https://ubuntu.example/apt");
    assert_eq!(repository.suite, "noble");
    assert_eq!(repository.components, ["stable", "main"]);
    assert_eq!(repository.packages, ["vendor", "helper"]);
    assert_eq!(repository.architecture, "arm64");
    assert_eq!(
        repository.keyring_path,
        PathBuf::from("/etc/apt/keyrings/cozydot-vendor-repo.gpg")
    );
    assert_eq!(
        repository.source_list_path,
        PathBuf::from("/etc/apt/sources.list.d/cozydot-vendor-repo.list")
    );

    let debian = platform("debian", "debian", "trixie", "none", "amd64");
    let repository = plan(&config, &debian, Path::new("."))
        .unwrap()
        .actions
        .into_iter()
        .find_map(|action| match action {
            PlannedAction::Repository(repository) => Some(repository),
            _ => None,
        })
        .unwrap();
    assert_eq!(repository.source_url, "https://default.example/apt");
    assert_eq!(repository.suite, "trixie");

    let no_codename = platform("ubuntu", "ubuntu", "", "none", "amd64");
    let error = plan(&config, &no_codename, Path::new("."))
        .unwrap_err()
        .to_string();
    assert!(error.contains("packages.repositories[0].source.suite"));
}

#[test]
fn npm_requires_node_and_dotfiles_always_back_up_before_stow() {
    let npm = ConfigV1::parse("schema: 1\npackages:\n  npm: [package]").unwrap();
    assert_eq!(
        plan(&npm, &ubuntu("amd64"), Path::new("."))
            .unwrap_err()
            .to_string(),
        "packages.npm: requires tools.node"
    );

    let dotfiles = ConfigV1::parse("schema: 1\ndotfiles:\n  packages: [bash, starship]").unwrap();
    let action = plan(&dotfiles, &ubuntu("amd64"), Path::new("/active/dotfiles"))
        .unwrap()
        .actions
        .into_iter()
        .find_map(|action| match action {
            PlannedAction::Dotfiles(dotfiles) => Some(dotfiles),
            _ => None,
        })
        .unwrap();
    assert_eq!(action.root, PathBuf::from("/active/dotfiles"));
    assert_eq!(action.packages, ["bash", "starship"]);
    assert_eq!(
        action.conflict_policy,
        DotfilesConflictPolicy::BackupBeforeStow
    );
}

#[test]
fn desktop_mismatch_omits_only_desktop_actions() {
    let config = ConfigV1::parse(
        "schema: 1\nintegrations:\n  vscode:\n    extensions: [rust-lang.rust-analyzer]\ndesktop:\n  theme: dark\n  terminal: wezterm\n  idle: { timeout: 0s, dim: false }\n  gnome:\n    extensions: [example@test]\n    dock: true\n    rounded_corners: true",
    )
    .unwrap();
    let none = plan(
        &config,
        &platform("ubuntu", "ubuntu", "noble", "none", "amd64"),
        Path::new("."),
    )
    .unwrap()
    .actions;
    assert!(!none
        .iter()
        .any(|action| matches!(action, PlannedAction::Desktop(_))));
    assert!(none.iter().any(|action| matches!(
        action,
        PlannedAction::Integration(IntegrationAction::VsCodeExtensions(_))
    )));

    let cinnamon = plan(
        &config,
        &platform("ubuntu", "ubuntu", "noble", "cinnamon", "amd64"),
        Path::new("."),
    )
    .unwrap()
    .actions;
    assert!(cinnamon
        .iter()
        .any(|action| matches!(action, PlannedAction::Desktop(DesktopAction::Theme(_)))));
    assert!(!cinnamon.iter().any(|action| matches!(
        action,
        PlannedAction::Desktop(
            DesktopAction::GnomeExtensions(_)
                | DesktopAction::GnomeDock
                | DesktopAction::GnomeRoundedCorners
        )
    )));
}

#[test]
fn updates_are_granular_scoped_and_preserve_moving_or_exact_selectors() {
    let config = ConfigV1::parse(
        "schema: 1\npackages:\n  flatpak: [com.example.App]\n  cargo: [bat]\n  npm: [package]\n  direct:\n    - name: app\n      format: deb\n      provides: [app]\n      source:\n        type: github\n        repository: owner/repo\n        assets:\n          amd64: { include: 'app-*.deb', exclude: [] }\ntools:\n  rust: '1.85.0'\n  go: latest\n  node: lts\nupdates:\n  apt: full\n  flatpak: true\n  tools: { rust: true, go: true, node: true }\n  packages: { cargo: true, npm: true, direct: true }",
    )
    .unwrap();
    let actions = plan(&config, &ubuntu("amd64"), Path::new("."))
        .unwrap()
        .actions;
    let updates = update_actions(&actions);
    assert_eq!(updates.len(), 8);
    assert!(updates.contains(&&UpdateAction::Apt {
        policy: AptUpdatePolicy::Full,
        target: AptUpdateTarget::SystemPackages,
    }));
    assert!(updates.contains(&&UpdateAction::Flatpak {
        refs: vec!["com.example.App".into()],
        scope: FlatpakUpdateScope::ConfiguredRefsAndRequiredRuntimes,
    }));
    assert!(updates.contains(&&UpdateAction::Tool(ToolUpdate::Rust {
        selector: RustSelector::Version("1.85.0".into()),
        target: "x86_64-unknown-linux-gnu".into(),
    })));
    assert!(updates.contains(&&UpdateAction::Tool(ToolUpdate::Go {
        selector: GoSelector::Latest,
        archive_architecture: "amd64".into(),
    })));
    assert!(updates.contains(&&UpdateAction::Tool(ToolUpdate::Node {
        selector: NodeSelector::Lts,
    })));
    assert!(updates.contains(&&UpdateAction::Cargo {
        packages: vec!["bat".into()]
    }));
    assert!(updates.contains(&&UpdateAction::Npm {
        packages: vec!["package".into()]
    }));
    assert!(
        matches!(updates.last(), Some(UpdateAction::Direct { packages }) if packages.len() == 1)
    );

    let installs = actions
        .iter()
        .filter_map(|action| match action {
            PlannedAction::Tool(tool) => Some(tool),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(installs.contains(&&ToolInstall::Rust {
        selector: RustSelector::Version("1.85.0".into()),
        target: "x86_64-unknown-linux-gnu".into(),
    }));
    assert!(!RustSelector::Version("1.85.0".into()).is_moving());
    assert!(GoSelector::Latest.is_moving());
    assert!(NodeSelector::Lts.is_moving());
}

#[test]
fn disabled_empty_and_missing_update_prerequisites_are_no_ops_without_cross_enablement() {
    for apt in ["", "  apt: null\n", "  apt: off\n"] {
        let yaml = format!("schema: 1\nupdates:\n{apt}  flatpak: false");
        let config = ConfigV1::parse(&yaml).unwrap();
        assert!(update_actions(
            &plan(&config, &ubuntu("amd64"), Path::new("."))
                .unwrap()
                .actions
        )
        .is_empty());
    }

    let empty = ConfigV1::parse(
        "schema: 1\npackages:\n  flatpak: []\n  cargo: []\n  npm: []\n  direct: []\nupdates:\n  flatpak: true\n  tools: { rust: true, go: true, node: true }\n  packages: { cargo: true, npm: true, direct: true }",
    )
    .unwrap();
    assert!(plan(&empty, &ubuntu("amd64"), Path::new("."))
        .unwrap()
        .actions
        .is_empty());

    let apt_only = ConfigV1::parse("schema: 1\nupdates:\n  apt: standard").unwrap();
    let actions = plan(&apt_only, &ubuntu("amd64"), Path::new("."))
        .unwrap()
        .actions;
    assert_eq!(
        update_actions(&actions),
        [&UpdateAction::Apt {
            policy: AptUpdatePolicy::Standard,
            target: AptUpdateTarget::SystemPackages,
        }]
    );
    assert_eq!(actions.len(), 2);
}

#[test]
fn typed_intents_contain_domain_values_not_commands_or_interpolation() {
    let config = ConfigV1::parse(FULL).unwrap();
    let plan = plan(&config, &ubuntu("amd64"), Path::new("/dotfiles")).unwrap();
    let debug = format!("{plan:#?}");
    for forbidden in [
        "serde_yaml",
        "CommandStep",
        "Shell(",
        "sudo ",
        "apt-get ",
        "${",
        "$(",
    ] {
        assert!(!debug.contains(forbidden), "found {forbidden:?} in {debug}");
    }
    let direct = plan.actions.iter().find_map(|action| match action {
        PlannedAction::DirectPackage(package) => Some(package),
        _ => None,
    });
    assert!(matches!(direct, Some(DirectPackageIntent { .. })));
}
