use cozydot::{
    config::v1::{AptSourceToken, ConfigV1, Theme},
    planner::v1::{
        plan, AptSourcesIntent, AptUpdatePolicy, AptUpdateTarget, DesktopAction, DesktopTarget,
        DotfilesConflictPolicy, ExistingProduct, FlatpakUpdateScope, GoSelector, IntegrationAction,
        NodeSelector, PlannedAction, Prerequisite, RepositorySuite, RustSelector, SystemAction,
        ToolInstall, ToolUpdate, UpdateAction,
    },
    platform::{Architecture, Platform},
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

fn planned(yaml: &str, platform: &Platform) -> Vec<PlannedAction> {
    let config = ConfigV1::parse(yaml).unwrap();
    plan(&config, platform, Path::new("/dotfiles"))
        .unwrap()
        .actions
}

fn prerequisites(actions: &[PlannedAction]) -> Vec<Prerequisite> {
    actions
        .iter()
        .find_map(|action| match action {
            PlannedAction::Bootstrap(bootstrap) => {
                Some(bootstrap.prerequisites.iter().copied().collect())
            }
            _ => None,
        })
        .unwrap_or_default()
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

#[test]
fn minimal_config_is_an_empty_typed_plan() {
    let config = ConfigV1::parse(MINIMAL).unwrap();
    assert!(plan(&config, &ubuntu("amd64"), Path::new("/dotfiles"))
        .unwrap()
        .actions
        .is_empty());
}

#[test]
fn apt_phases_are_dependency_safe_with_one_refresh_before_every_consumer() {
    let actions = planned(
        "schema: 1
system:
  ensure_admin: true
  apt:
    sources: managed
    components: [main]
    unattended_upgrades: false
  ubuntu: { snap: true, codecs: true }
packages:
  remove: [old]
  repositories:
    - name: vendor
      key: https://example.com/key
      source: { urls: { default: https://example.com/repo }, suite: system, components: [main] }
      packages: [vendor]
  apt: [native]
updates:
  apt: full",
        &ubuntu("amd64"),
    );

    assert!(matches!(actions[0], PlannedAction::Bootstrap(_)));
    assert_eq!(actions[1], PlannedAction::System(SystemAction::EnsureAdmin));
    assert!(matches!(
        actions[2],
        PlannedAction::System(SystemAction::AptSources(AptSourcesIntent::Managed(_)))
    ));
    assert!(matches!(actions[3], PlannedAction::Repository(_)));
    assert_eq!(actions[4], PlannedAction::AptMetadataRefresh);
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(action, PlannedAction::AptMetadataRefresh))
            .count(),
        1
    );
    assert_eq!(
        &actions[5..10],
        [
            PlannedAction::System(SystemAction::UnattendedUpgrades { enabled: false }),
            PlannedAction::System(SystemAction::UbuntuSnap { enabled: true }),
            PlannedAction::System(SystemAction::UbuntuCodecs),
            PlannedAction::RemovePackages(vec!["old".into()]),
            PlannedAction::RepositoryPackages(cozydot::planner::v1::AptRepositoryPackages {
                repository: "vendor".into(),
                packages: vec!["vendor".into()],
            }),
        ]
    );
    assert_eq!(
        actions[10],
        PlannedAction::AptPackages(vec!["native".into()])
    );
    assert_eq!(
        actions.last(),
        Some(&PlannedAction::Update(UpdateAction::Apt {
            policy: AptUpdatePolicy::Full,
            target: AptUpdateTarget::SystemPackages,
        }))
    );
}

#[test]
fn source_only_preserve_and_disabled_enable_only_controls_are_exact_no_ops() {
    let source_only = planned(
        "schema: 1
system:
  ensure_admin: false
  apt:
    sources: managed
    components: [main]",
        &ubuntu("amd64"),
    );
    assert_eq!(source_only.len(), 1);
    assert!(matches!(
        source_only[0],
        PlannedAction::System(SystemAction::AptSources(AptSourcesIntent::Managed(_)))
    ));
    assert!(!source_only.contains(&PlannedAction::AptMetadataRefresh));

    for sources in ["", "    sources: null\n", "    sources: preserve\n"] {
        let yaml = format!(
            "schema: 1\nsystem:\n  ensure_admin: false\n  apt:\n{sources}    unattended_upgrades: null\n"
        );
        assert!(planned(&yaml, &ubuntu("amd64")).is_empty());
    }
}

#[test]
fn managed_sources_require_a_codename_and_unattended_both_states_are_consumers() {
    let managed =
        ConfigV1::parse("schema: 1\nsystem:\n  apt:\n    sources: managed\n    components: [main]")
            .unwrap();
    let error = plan(
        &managed,
        &platform("ubuntu", "ubuntu", "", "none", "amd64"),
        Path::new("."),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("system.apt.sources"));
    assert!(error.contains("codename"));

    let error = plan(
        &managed,
        &platform("ubuntu", "ubuntu", "Noble Main", "none", "amd64"),
        Path::new("."),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("system.apt.sources"));
    assert!(error.contains("valid platform codename"));

    for enabled in [true, false] {
        let actions = planned(
            &format!("schema: 1\nsystem:\n  apt:\n    unattended_upgrades: {enabled}"),
            &ubuntu("amd64"),
        );
        assert_eq!(
            actions,
            [
                PlannedAction::AptMetadataRefresh,
                PlannedAction::System(SystemAction::UnattendedUpgrades { enabled }),
            ]
        );
    }
}

#[test]
fn managed_source_policy_is_release_aware_and_kali_never_uses_debian_sources() {
    let yaml = "schema: 1\nsystem:\n  apt:\n    sources: managed\n    components: [main]";
    let noble = planned(
        yaml,
        &platform("ubuntu", "ubuntu", "noble", "none", "arm64"),
    );
    let PlannedAction::System(SystemAction::AptSources(AptSourcesIntent::Managed(noble))) =
        &noble[0]
    else {
        panic!("missing noble managed source intent")
    };
    assert_eq!(noble.stanzas.len(), 1);
    assert_eq!(
        noble.stanzas[0].uri,
        "https://ports.ubuntu.com/ubuntu-ports"
    );

    let resolute = planned(
        yaml,
        &platform("ubuntu", "ubuntu", "resolute", "none", "arm64"),
    );
    let PlannedAction::System(SystemAction::AptSources(AptSourcesIntent::Managed(resolute))) =
        &resolute[0]
    else {
        panic!("missing resolute managed source intent")
    };
    assert_eq!(resolute.stanzas.len(), 2);
    assert_eq!(resolute.stanzas[0].uri, "https://archive.ubuntu.com/ubuntu");

    let kali = planned(
        yaml,
        &platform("kali", "debian", "kali-rolling", "none", "arm64"),
    );
    let PlannedAction::System(SystemAction::AptSources(AptSourcesIntent::Managed(kali))) = &kali[0]
    else {
        panic!("missing Kali managed source intent")
    };
    let rendered = kali.render_deb822();
    assert!(rendered.contains("https://http.kali.org/kali"));
    assert!(rendered.contains("Suites: kali-rolling"));
    assert!(!rendered.contains("debian.org"));
}

#[test]
fn system_repository_suite_uses_distribution_not_base_codename() {
    let mint = Platform::from_release_parts(
        "linuxmint".into(),
        "ubuntu".into(),
        "wilma".into(),
        "noble".into(),
        "none".into(),
        "amd64",
    )
    .unwrap();
    let actions = planned(
        "schema: 1\npackages:\n  repositories:\n    - name: vendor\n      key: https://example.com/key\n      source: { urls: { default: https://example.com/repo }, suite: system, components: [main] }\n      packages: [vendor]",
        &mint,
    );
    let repository = actions
        .iter()
        .find_map(|action| match action {
            PlannedAction::Repository(repository) => Some(repository),
            _ => None,
        })
        .unwrap();
    assert_eq!(repository.suite.value(), "wilma");
}

#[test]
fn integrations_require_existing_products_without_installable_prerequisites_or_apt_refresh() {
    let actions = planned(
        "schema: 1
integrations:
  docker: { add_user_to_group: true, local_log_driver: true }
  virtualbox: { add_user_to_group: true }
  vscode: { extensions: [rust-lang.rust-analyzer] }",
        &ubuntu("amd64"),
    );
    assert_eq!(prerequisites(&actions), []);
    assert!(!actions
        .iter()
        .any(|action| matches!(action, PlannedAction::AptMetadataRefresh)));
    assert_eq!(
        actions,
        [
            PlannedAction::Integration(IntegrationAction::DockerGroup),
            PlannedAction::Integration(IntegrationAction::DockerLocalLog { max_size: None }),
            PlannedAction::Integration(IntegrationAction::VirtualBoxGroup),
            PlannedAction::Integration(IntegrationAction::VsCodeExtensions(vec![
                "rust-lang.rust-analyzer".into()
            ])),
        ]
    );
    let required = actions
        .iter()
        .filter_map(|action| match action {
            PlannedAction::Integration(integration) => Some(integration.required_product()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        required,
        [
            ExistingProduct::Docker,
            ExistingProduct::Docker,
            ExistingProduct::VirtualBox,
            ExistingProduct::VsCode,
        ]
    );
}

#[test]
fn native_apt_install_and_update_do_not_require_https_downloader() {
    for yaml in [
        "schema: 1\npackages:\n  apt: [curl]",
        "schema: 1\nupdates:\n  apt: standard",
    ] {
        let actions = planned(yaml, &ubuntu("amd64"));
        assert_eq!(prerequisites(&actions), []);
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(action, PlannedAction::AptMetadataRefresh))
                .count(),
            1
        );
    }
}

#[test]
fn rustup_and_cargo_binstall_are_inferred_separately() {
    let rust = planned("schema: 1\ntools:\n  rust: stable", &ubuntu("amd64"));
    assert_eq!(
        prerequisites(&rust),
        [Prerequisite::HttpsDownloader, Prerequisite::Rustup]
    );

    let cargo = planned("schema: 1\npackages:\n  cargo: [bat]", &ubuntu("amd64"));
    assert_eq!(
        prerequisites(&cargo),
        [
            Prerequisite::HttpsDownloader,
            Prerequisite::Rustup,
            Prerequisite::CargoBinstall,
        ]
    );
}

#[test]
fn repository_publication_retains_typed_values_and_has_a_separate_package_group() {
    let yaml = "schema: 1
packages:
  repositories:
    - name: 'Vendor Repo!'
      key: https://münchen.example/key.asc
      source:
        urls: { default: https://default.example/apt, ubuntu: https://ubuntu.example/apt }
        suite: system
        components: [stable, main]
      packages: [vendor, helper]";
    let actions = planned(yaml, &ubuntu("arm64"));
    let repository = actions
        .iter()
        .find_map(|action| match action {
            PlannedAction::Repository(repository) => Some(repository),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        repository.key_url.as_str(),
        "https://xn--mnchen-3ya.example/key.asc"
    );
    assert_eq!(repository.source_url.as_str(), "https://ubuntu.example/apt");
    assert_eq!(
        repository.suite,
        RepositorySuite::ResolvedSystem(AptSourceToken::parse("noble").unwrap())
    );
    assert_eq!(repository.suite.value(), "noble");
    assert_eq!(repository.architecture, Architecture::Arm64);
    assert_eq!(
        repository
            .components
            .iter()
            .map(AptSourceToken::as_str)
            .collect::<Vec<_>>(),
        ["stable", "main"]
    );
    assert_eq!(
        repository.keyring_path,
        PathBuf::from("/etc/apt/keyrings/cozydot-vendor-repo.gpg")
    );
    assert_eq!(
        repository.source_list_path,
        PathBuf::from("/etc/apt/sources.list.d/cozydot-vendor-repo.list")
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        PlannedAction::RepositoryPackages(group)
            if group.repository == "Vendor Repo!" && group.packages == ["vendor", "helper"]
    )));

    let fixed = planned(
        &yaml.replace("suite: system", "suite: stable"),
        &platform("debian", "debian", "trixie", "none", "amd64"),
    );
    let repository = fixed
        .iter()
        .find_map(|action| match action {
            PlannedAction::Repository(repository) => Some(repository),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        repository.source_url.as_str(),
        "https://default.example/apt"
    );
    assert_eq!(
        repository.suite,
        RepositorySuite::Fixed(AptSourceToken::parse("stable").unwrap())
    );
    assert_eq!(repository.architecture, Architecture::Amd64);
}

#[test]
fn system_repository_suite_validates_consumed_codename_only() {
    let system = ConfigV1::parse(
        "schema: 1
packages:
  repositories:
    - name: vendor
      key: https://example.com/key
      source: { urls: { default: https://example.com/repo }, suite: system, components: [main] }
      packages: [vendor]",
    )
    .unwrap();
    let malformed = platform("ubuntu", "ubuntu", "noble/", "none", "amd64");
    let error = plan(&system, &malformed, Path::new("."))
        .unwrap_err()
        .to_string();
    assert!(error.contains("packages.repositories[0].source.suite"));
    assert!(error.contains("system platform codename"));

    let fixed = ConfigV1::parse(
        "schema: 1
packages:
  repositories:
    - name: vendor
      key: https://example.com/key
      source: { urls: { default: https://example.com/repo }, suite: stable, components: [main] }
      packages: [vendor]",
    )
    .unwrap();
    plan(&fixed, &malformed, Path::new(".")).unwrap();
    plan(
        &ConfigV1::parse(MINIMAL).unwrap(),
        &platform("ubuntu", "ubuntu", "arbitrary codename", "none", "amd64"),
        Path::new("."),
    )
    .unwrap();
}

#[test]
fn desktop_actions_retain_exact_gnome_and_cinnamon_targets_and_zero_timeout() {
    let yaml = "schema: 1
desktop:
  theme: dark
  terminal: wezterm
  idle: { timeout: 0s, dim: false }
  gnome: { extensions: [example@test], dock: true, rounded_corners: true }";
    for (desktop, target, count) in [
        ("gnome", DesktopTarget::Gnome, 7),
        ("cinnamon", DesktopTarget::Cinnamon, 4),
    ] {
        let actions = planned(
            yaml,
            &platform("ubuntu", "ubuntu", "noble", desktop, "amd64"),
        );
        let desktop_actions = actions
            .iter()
            .filter_map(|action| match action {
                PlannedAction::Desktop(action) => Some(action),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(desktop_actions.len(), count);
        assert!(desktop_actions.contains(&&DesktopAction::Theme {
            target,
            theme: Theme::Dark,
        }));
        assert!(desktop_actions.contains(&&DesktopAction::Terminal {
            target,
            executable: "wezterm".into(),
        }));
        assert!(desktop_actions.contains(&&DesktopAction::IdleTimeout {
            target,
            timeout: "0s".into(),
        }));
        assert!(desktop_actions.contains(&&DesktopAction::IdleDim {
            target,
            enabled: false,
        }));
    }

    let none = planned(
        yaml,
        &platform("ubuntu", "ubuntu", "noble", "none", "amd64"),
    );
    assert!(!none
        .iter()
        .any(|action| matches!(action, PlannedAction::Desktop(_))));

    for desktop in ["KDE", "plasma", "arbitrary text", ""] {
        let resolved = platform("ubuntu", "ubuntu", "noble", desktop, "amd64");
        assert_eq!(resolved.desktop, "none");
        assert!(!planned(yaml, &resolved)
            .iter()
            .any(|action| matches!(action, PlannedAction::Desktop(_))));
    }

    let mixed = platform(
        "ubuntu",
        "ubuntu",
        "noble",
        "plasma:X-Cinnamon:GNOME",
        "amd64",
    );
    assert_eq!(mixed.desktop, "cinnamon");
    assert!(planned(yaml, &mixed).iter().any(|action| matches!(
        action,
        PlannedAction::Desktop(DesktopAction::Theme {
            target: DesktopTarget::Cinnamon,
            ..
        })
    )));
}

#[test]
fn ubuntu_controls_apply_only_to_resolved_ubuntu_family() {
    for enabled in [true, false] {
        let yaml = format!("schema: 1\nsystem:\n  ubuntu:\n    snap: {enabled}\n    codecs: true");
        let actions = planned(&yaml, &ubuntu("amd64"));
        assert!(actions.contains(&PlannedAction::System(SystemAction::UbuntuSnap { enabled })));
        assert!(actions.contains(&PlannedAction::System(SystemAction::UbuntuCodecs)));

        for distro in ["linuxmint", "pop", "zorin"] {
            let derivative = planned(
                &yaml,
                &platform(distro, "ubuntu", "release", "none", "amd64"),
            );
            assert!(
                derivative.contains(&PlannedAction::System(SystemAction::UbuntuSnap { enabled }))
            );
            assert!(derivative.contains(&PlannedAction::System(SystemAction::UbuntuCodecs)));
        }

        let debian = planned(
            &yaml,
            &platform("debian", "debian", "trixie", "none", "amd64"),
        );
        assert!(debian.is_empty());
        for distro in ["linuxmint", "deepin"] {
            let debian_family = planned(
                &yaml,
                &platform(distro, "debian", "release", "none", "amd64"),
            );
            assert!(debian_family.is_empty());
        }
    }

    for codecs in ["false", "null"] {
        let yaml = format!("schema: 1\nsystem:\n  ubuntu:\n    codecs: {codecs}");
        assert!(planned(&yaml, &ubuntu("amd64")).is_empty());
    }
}

#[test]
fn direct_packages_select_all_architectures_and_preserve_ordered_excludes() {
    let yaml = "schema: 1
packages:
  direct:
    - name: app
      format: deb
      provides: [app]
      source:
        type: github
        repository: owner/repo
        assets:
          amd64: { include: 'app-amd64-*.deb', exclude: ['app-amd64-debug-*.deb', 'app-amd64-old-*.deb'] }
          arm64: { include: 'app-arm64-*.deb', exclude: ['app-arm64-debug-*.deb', 'app-arm64-old-*.deb'] }
          arm32: { include: 'app-arm32-*.deb', exclude: ['app-arm32-debug-*.deb', 'app-arm32-old-*.deb'] }
          riscv64: { include: 'app-riscv64-*.deb', exclude: ['app-riscv64-debug-*.deb', 'app-riscv64-old-*.deb'] }";
    for (arch, architecture) in [
        ("amd64", Architecture::Amd64),
        ("arm64", Architecture::Arm64),
        ("arm32", Architecture::Arm32),
        ("riscv64", Architecture::Riscv64),
    ] {
        let actions = planned(yaml, &ubuntu(arch));
        let direct = actions
            .iter()
            .find_map(|action| match action {
                PlannedAction::DirectPackage(package) => Some(package),
                _ => None,
            })
            .unwrap();
        assert_eq!(direct.source.architecture, architecture);
        assert_eq!(direct.source.selector.include, format!("app-{arch}-*.deb"));
        assert_eq!(
            direct.source.selector.exclude,
            [
                format!("app-{arch}-debug-*.deb"),
                format!("app-{arch}-old-*.deb")
            ]
        );
    }
}

#[test]
fn exact_tool_selectors_and_typed_architectures_are_retained() {
    for (value, expected) in [
        ("stable", RustSelector::Stable),
        ("beta", RustSelector::Beta),
        ("nightly", RustSelector::Nightly),
        (
            "nightly-2026-07-14",
            RustSelector::DatedNightly("nightly-2026-07-14".into()),
        ),
        ("1.85.0", RustSelector::Version("1.85.0".into())),
    ] {
        let actions = planned(
            &format!("schema: 1\ntools:\n  rust: {value}"),
            &ubuntu("riscv64"),
        );
        assert!(actions.contains(&PlannedAction::Tool(ToolInstall::Rust {
            selector: expected,
            architecture: Architecture::Riscv64,
        })));
    }
    for (value, expected) in [
        ("latest", GoSelector::Latest),
        ("1.24.5", GoSelector::Version("1.24.5".into())),
    ] {
        let actions = planned(
            &format!("schema: 1\ntools:\n  go: {value:?}"),
            &ubuntu("arm32"),
        );
        assert!(actions.contains(&PlannedAction::Tool(ToolInstall::Go {
            selector: expected,
            architecture: Architecture::Arm32,
        })));
    }
    for (value, expected) in [
        ("lts", NodeSelector::Lts),
        ("latest", NodeSelector::Latest),
        ("22", NodeSelector::Version("22".into())),
    ] {
        let actions = planned(
            &format!("schema: 1\ntools:\n  node: {value:?}"),
            &ubuntu("amd64"),
        );
        assert!(actions.contains(&PlannedAction::Tool(ToolInstall::Node {
            selector: expected,
        })));
    }
}

#[test]
fn updates_do_not_cross_enable_and_apt_has_one_explicit_refresh() {
    for apt in ["", "  apt: null\n", "  apt: off\n"] {
        let yaml = format!("schema: 1\nupdates:\n{apt}  flatpak: false");
        assert!(planned(&yaml, &ubuntu("amd64")).is_empty());
    }

    let apt = planned("schema: 1\nupdates:\n  apt: standard", &ubuntu("amd64"));
    assert_eq!(
        apt,
        [
            PlannedAction::AptMetadataRefresh,
            PlannedAction::Update(UpdateAction::Apt {
                policy: AptUpdatePolicy::Standard,
                target: AptUpdateTarget::SystemPackages,
            }),
        ]
    );

    let rust = planned(
        "schema: 1\ntools: { rust: stable, go: latest, node: lts }\nupdates:\n  tools: { rust: true, go: false, node: false }",
        &ubuntu("amd64"),
    );
    assert_eq!(
        update_actions(&rust),
        [&UpdateAction::Tool(ToolUpdate::Rust {
            selector: RustSelector::Stable,
            architecture: Architecture::Amd64,
        })]
    );
    assert!(!rust
        .iter()
        .any(|action| matches!(action, PlannedAction::AptMetadataRefresh)));
}

#[test]
fn granular_updates_retain_only_configured_targets_and_typed_architectures() {
    let actions = planned(
        "schema: 1
packages:
  flatpak: [com.example.App]
  cargo: [bat]
  npm: [package]
  direct:
    - name: app
      format: deb
      provides: [app]
      source:
        type: github
        repository: owner/repo
        assets:
          amd64: { include: 'app-*.deb', exclude: [] }
tools: { rust: '1.85.0', go: latest, node: lts }
updates:
  apt: full
  flatpak: true
  tools: { rust: true, go: true, node: true }
  packages: { cargo: true, npm: true, direct: true }",
        &ubuntu("amd64"),
    );
    let updates = update_actions(&actions);
    assert_eq!(updates.len(), 8);
    assert!(updates.contains(&&UpdateAction::Apt {
        policy: AptUpdatePolicy::Full,
        target: AptUpdateTarget::SystemPackages,
    }));
    assert!(updates.contains(&&UpdateAction::Flatpak {
        refs: vec!["com.example.App".into()],
        scope: FlatpakUpdateScope::ConfiguredAppsRequiredRuntimesRelatedRefsAndEolReplacements,
    }));
    assert!(updates.contains(&&UpdateAction::Tool(ToolUpdate::Rust {
        selector: RustSelector::Version("1.85.0".into()),
        architecture: Architecture::Amd64,
    })));
    assert!(updates.contains(&&UpdateAction::Tool(ToolUpdate::Go {
        selector: GoSelector::Latest,
        architecture: Architecture::Amd64,
    })));
    assert!(updates.contains(&&UpdateAction::Tool(ToolUpdate::Node {
        selector: NodeSelector::Lts,
    })));
    assert!(updates.contains(&&UpdateAction::Cargo {
        packages: vec!["bat".into()],
    }));
    assert!(updates.contains(&&UpdateAction::Npm {
        packages: vec!["package".into()],
    }));
    assert!(matches!(
        updates.last(),
        Some(UpdateAction::Direct { packages })
            if packages.len() == 1 && packages[0].name == "app"
    ));
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(action, PlannedAction::AptMetadataRefresh))
            .count(),
        1
    );
    assert!(!RustSelector::Version("1.85.0".into()).is_moving());
    assert!(GoSelector::Latest.is_moving());
    assert!(NodeSelector::Lts.is_moving());
}

#[test]
fn dotfiles_keep_the_only_conflict_policy() {
    let actions = planned(
        "schema: 1\ndotfiles:\n  packages: [bash, starship]",
        &ubuntu("amd64"),
    );
    let dotfiles = actions
        .iter()
        .find_map(|action| match action {
            PlannedAction::Dotfiles(dotfiles) => Some(dotfiles),
            _ => None,
        })
        .unwrap();
    assert_eq!(dotfiles.root, PathBuf::from("/dotfiles"));
    assert_eq!(dotfiles.packages, ["bash", "starship"]);
    assert_eq!(
        dotfiles.conflict_policy,
        DotfilesConflictPolicy::BackupBeforeStow
    );
}

#[test]
fn canonical_full_fixture_keeps_one_bootstrap_and_one_apt_refresh() {
    let config = ConfigV1::parse(FULL).unwrap();
    let actions = plan(&config, &ubuntu("amd64"), Path::new("/dotfiles"))
        .unwrap()
        .actions;
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(action, PlannedAction::Bootstrap(_)))
            .count(),
        1
    );
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(action, PlannedAction::AptMetadataRefresh))
            .count(),
        1
    );
    let refresh = actions
        .iter()
        .position(|action| matches!(action, PlannedAction::AptMetadataRefresh))
        .unwrap();
    let repository = actions
        .iter()
        .position(|action| matches!(action, PlannedAction::Repository(_)))
        .unwrap();
    let repository_packages = actions
        .iter()
        .position(|action| matches!(action, PlannedAction::RepositoryPackages(_)))
        .unwrap();
    assert!(repository < refresh && refresh < repository_packages);
}

#[test]
fn typed_intents_contain_no_commands_or_interpolation() {
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
}
