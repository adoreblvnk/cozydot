use cozydot::{
    config::Config,
    planner::{
        plan, BinarySourceIntent, DesktopIntent, DesktopTarget, ExistingProductRequirement,
        ManagerBootstrap, Plan, PlanPhaseKind, PlannedAction, PreconditionTiming,
        ProviderConvergence, RepositoryLayout, SkipReason, SkippedIntent, SystemPrerequisite,
        ToolchainIntent, UpdateIntent,
    },
    platform::{Architecture, Platform},
};
use std::path::{Path, PathBuf};

const FULL: &str = include_str!("../docs/examples/config-v1-full.yaml");
const EXHAUSTIVE: &str = include_str!("../docs/examples/config-v1-exhaustive.yaml");

fn platform(
    distro: &str,
    upstream: &str,
    distro_codename: &str,
    base_codename: &str,
    desktop: &str,
    arch: &str,
) -> Platform {
    Platform::from_release_parts(
        distro.into(),
        upstream.into(),
        distro_codename.into(),
        base_codename.into(),
        desktop.into(),
        arch,
    )
    .unwrap()
}

fn ubuntu(desktop: &str, arch: &str) -> Platform {
    platform("ubuntu", "ubuntu", "noble", "noble", desktop, arch)
}

fn debian(desktop: &str) -> Platform {
    platform("debian", "debian", "trixie", "trixie", desktop, "amd64")
}

fn planned(yaml: &str, platform: &Platform) -> Plan {
    let config = Config::parse(yaml).unwrap();
    plan(&config, platform, Path::new("/dotfiles")).unwrap()
}

fn assert_empty_manager_phase(plan: &Plan) {
    assert!(plan
        .phase(PlanPhaseKind::ManagerBootstraps)
        .actions()
        .is_empty());
}

fn apt_refresh_count(plan: &Plan) -> usize {
    plan.actions()
        .filter(|action| matches!(action, PlannedAction::AptMetadataRefresh))
        .count()
}

#[test]
fn public_neutral_api_exposes_read_only_fixed_phase_traversal() {
    fn accepts_public_plan(_: &Plan) {}
    let plan = planned("version: 1.0.0", &ubuntu("none", "amd64"));
    accepts_public_plan(&plan);
    assert_eq!(
        plan.phases()
            .iter()
            .map(|phase| phase.kind())
            .collect::<Vec<_>>(),
        [
            PlanPhaseKind::SystemPrerequisites,
            PlanPhaseKind::ManagerBootstraps,
            PlanPhaseKind::AdministrativeVerification,
            PlanPhaseKind::OfficialAptSources,
            PlanPhaseKind::ThirdPartyRepositories,
            PlanPhaseKind::AptMetadataRefresh,
            PlanPhaseKind::SystemPackageStates,
            PlanPhaseKind::AptPurge,
            PlanPhaseKind::RepositoryPackages,
            PlanPhaseKind::AptPackages,
            PlanPhaseKind::FlatpakApplications,
            PlanPhaseKind::LanguageToolchains,
            PlanPhaseKind::LanguagePackages,
            PlanPhaseKind::BinaryPackages,
            PlanPhaseKind::Fonts,
            PlanPhaseKind::Dotfiles,
            PlanPhaseKind::Integrations,
            PlanPhaseKind::Desktop,
            PlanPhaseKind::Updates,
            PlanPhaseKind::FinalVerification,
        ]
    );
    assert!(plan.phases().iter().all(|phase| phase.actions().is_empty()));
    assert!(plan.is_empty());
    assert!(plan
        .phase(PlanPhaseKind::FinalVerification)
        .actions()
        .is_empty());
}

#[test]
fn full_fixture_plans_on_both_mandatory_reference_hosts() {
    let config = Config::parse(FULL).unwrap();
    for target in [ubuntu("gnome", "amd64"), debian("gnome")] {
        let plan = plan(&config, &target, Path::new("/dotfiles")).unwrap();
        assert_eq!(plan.phases().len(), 20);
        assert_eq!(
            plan.actions()
                .filter(|action| matches!(action, PlannedAction::AptMetadataRefresh))
                .count(),
            1
        );
    }
}

#[test]
fn full_fixture_observes_every_execution_phase_boundary_in_order() {
    let plan = planned(FULL, &ubuntu("gnome", "amd64"));
    assert_eq!(
        plan.phases()
            .iter()
            .map(|phase| phase.kind())
            .collect::<Vec<_>>(),
        PlanPhaseKind::ORDERED
    );
    for phase in &plan.phases()[..19] {
        assert!(
            !phase.actions().is_empty(),
            "empty required phase: {:?}",
            phase.kind()
        );
    }
    assert!(plan
        .phase(PlanPhaseKind::FinalVerification)
        .actions()
        .is_empty());
}

#[test]
fn repositories_publish_before_one_refresh_and_keep_later_groups_in_declaration_order() {
    let plan = planned(FULL, &ubuntu("gnome", "amd64"));
    let repositories = plan.phase(PlanPhaseKind::ThirdPartyRepositories).actions();
    let groups = plan.phase(PlanPhaseKind::RepositoryPackages).actions();
    assert_eq!(repositories.len(), 8);
    assert_eq!(groups.len(), 8);
    assert_eq!(
        plan.phase(PlanPhaseKind::AptMetadataRefresh)
            .actions()
            .len(),
        1
    );
    let names = repositories
        .iter()
        .map(|action| match action {
            PlannedAction::Repository(repository) => repository.name.as_str(),
            other => panic!("unexpected repository phase action: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "docker",
            "debian-griffo",
            "github-cli",
            "helium",
            "onlyoffice",
            "virtualbox",
            "vscode",
            "wezterm"
        ]
    );
}

#[test]
fn repository_resolution_keeps_upstream_codename_star_exact_path_and_deterministic_paths() {
    let yaml = "version: 1.0.0
packages:
  apt:
    repositories:
      - name: Vendor__Repo
        key: https://example.com/key
        urls: {ubuntu: https://ubuntu.example/repo, default: https://default.example/repo}
        suite: system
        components: [main]
        packages: [vendor]
      - name: literal-star
        key: https://example.com/star-key
        urls: {default: https://example.com/star}
        suite: '*'
        components: ['*']
        packages: [star]
      - name: exact-path
        key: https://example.com/path-key
        urls: {default: https://example.com/path}
        path: pool/vendor/
        packages: [path-package]";
    let mint = platform("linuxmint", "ubuntu", "wilma", "noble", "none", "arm64");
    let plan = planned(yaml, &mint);
    let repositories = plan.phase(PlanPhaseKind::ThirdPartyRepositories).actions();
    let PlannedAction::Repository(first) = &repositories[0] else {
        panic!("missing first repository")
    };
    assert_eq!(first.source_url.as_str(), "https://ubuntu.example/repo");
    assert_eq!(first.filename_stem, "vendor-repo");
    assert_eq!(first.architecture, Architecture::Arm64);
    assert_eq!(
        first.keyring_path,
        PathBuf::from("/etc/apt/keyrings/cozydot-vendor-repo.gpg")
    );
    assert_eq!(
        first.source_list_path,
        PathBuf::from("/etc/apt/sources.list.d/cozydot-vendor-repo.list")
    );
    match &first.layout {
        RepositoryLayout::SuiteComponents { suite, .. } => assert_eq!(suite.value(), "noble"),
        other => panic!("unexpected layout: {other:?}"),
    }
    let PlannedAction::Repository(star) = &repositories[1] else {
        panic!("missing star repository")
    };
    match &star.layout {
        RepositoryLayout::SuiteComponents { suite, components } => {
            assert_eq!(suite.value(), "*");
            assert_eq!(components[0].as_str(), "*");
        }
        other => panic!("unexpected layout: {other:?}"),
    }
    let PlannedAction::Repository(path) = &repositories[2] else {
        panic!("missing exact-path repository")
    };
    assert_eq!(
        path.layout,
        RepositoryLayout::ExactPath("pool/vendor/".into())
    );
}

#[test]
fn exhaustive_fixture_keeps_fixed_and_github_source_identity_and_github_only_updates() {
    let plan = planned(EXHAUSTIVE, &ubuntu("gnome", "amd64"));
    let binaries = plan.phase(PlanPhaseKind::BinaryPackages).actions();
    assert_eq!(binaries.len(), 2);
    let sources = binaries
        .iter()
        .map(|action| match action {
            PlannedAction::Binary(binary) => &binary.source,
            other => panic!("unexpected binary action: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert!(matches!(sources[0], BinarySourceIntent::Github { .. }));
    assert!(matches!(sources[1], BinarySourceIntent::FixedUrl { .. }));
    let update = plan
        .phase(PlanPhaseKind::Updates)
        .actions()
        .iter()
        .find_map(|action| match action {
            PlannedAction::Update(UpdateIntent::GithubBinaries(binaries)) => Some(binaries),
            _ => None,
        })
        .unwrap();
    assert_eq!(update.len(), 1);
    assert_eq!(update[0].name, "github-example");
}

#[test]
fn native_binary_selector_is_authoritative_for_every_architecture() {
    let yaml = "version: 1.0.0
packages:
  binaries:
    - name: app
      format: appimage
      commands: [app]
      source:
        provider: github
        repository: owner/app
        assets:
          amd64: {include: 'app-amd64-*.AppImage'}
          arm64: {include: 'app-arm64-*.AppImage'}
          arm32: {include: 'app-arm32-*.AppImage'}
          riscv64: {include: 'app-riscv64-*.AppImage'}";
    for (arch, expected) in [
        ("amd64", "app-amd64-*.AppImage"),
        ("arm64", "app-arm64-*.AppImage"),
        ("arm32", "app-arm32-*.AppImage"),
        ("riscv64", "app-riscv64-*.AppImage"),
    ] {
        let plan = planned(yaml, &ubuntu("none", arch));
        let PlannedAction::Binary(binary) = &plan.phase(PlanPhaseKind::BinaryPackages).actions()[0]
        else {
            panic!("missing binary")
        };
        let BinarySourceIntent::Github { selector, .. } = &binary.source else {
            panic!("wrong source")
        };
        assert_eq!(selector.include, expected);
    }
}

#[test]
fn dotfiles_only_uses_stow_prerequisite_and_owns_phase_sixteen() {
    let plan = planned(
        "version: 1.0.0\ndotfiles: {packages: [bash]}",
        &ubuntu("none", "amd64"),
    );
    assert_empty_manager_phase(&plan);
    let PlannedAction::SystemPrerequisites(prerequisites) =
        &plan.phase(PlanPhaseKind::SystemPrerequisites).actions()[0]
    else {
        panic!("missing prerequisites")
    };
    assert_eq!(prerequisites.len(), 1);
    assert!(prerequisites.contains(&SystemPrerequisite::Stow));
    assert!(matches!(
        plan.phase(PlanPhaseKind::Dotfiles).actions(),
        [PlannedAction::Dotfiles(intent)] if intent.packages == ["bash"]
    ));
}

#[test]
fn deb_only_uses_inspection_and_one_shared_apt_refresh_without_a_manager() {
    let plan = planned(
        "version: 1.0.0
packages:
  binaries:
    - name: deb-example
      format: deb
      commands: [deb-example]
      source:
        provider: url
        urls: {amd64: https://example.com/deb-example.deb}
        sha256: {amd64: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}",
        &ubuntu("none", "amd64"),
    );
    assert_empty_manager_phase(&plan);
    let PlannedAction::SystemPrerequisites(prerequisites) =
        &plan.phase(PlanPhaseKind::SystemPrerequisites).actions()[0]
    else {
        panic!("missing prerequisites")
    };
    assert!(prerequisites.contains(&SystemPrerequisite::HttpsCertificates));
    assert!(prerequisites.contains(&SystemPrerequisite::DebInspection));
    assert_eq!(plan.phase(PlanPhaseKind::BinaryPackages).actions().len(), 1);
    assert_eq!(apt_refresh_count(&plan), 1);
}

#[test]
fn appimage_only_uses_elf_inspection_without_a_manager_or_apt_refresh() {
    let plan = planned(
        "version: 1.0.0
packages:
  binaries:
    - name: app-example
      format: appimage
      commands: [app-example]
      source:
        provider: url
        urls: {amd64: https://example.com/app-example.AppImage}
        sha256: {amd64: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}",
        &ubuntu("none", "amd64"),
    );
    assert_empty_manager_phase(&plan);
    let PlannedAction::SystemPrerequisites(prerequisites) =
        &plan.phase(PlanPhaseKind::SystemPrerequisites).actions()[0]
    else {
        panic!("missing prerequisites")
    };
    assert!(prerequisites.contains(&SystemPrerequisite::HttpsCertificates));
    assert!(prerequisites.contains(&SystemPrerequisite::ElfInspection));
    assert_eq!(plan.phase(PlanPhaseKind::BinaryPackages).actions().len(), 1);
    assert_eq!(apt_refresh_count(&plan), 0);
}

#[test]
fn mixed_deb_and_appimage_binaries_share_exactly_one_apt_refresh() {
    let plan = planned(
        "version: 1.0.0
packages:
  binaries:
    - name: deb-example
      format: deb
      commands: [deb-example]
      source:
        provider: url
        urls: {amd64: https://example.com/deb-example.deb}
        sha256: {amd64: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}
    - name: app-example
      format: appimage
      commands: [app-example]
      source:
        provider: url
        urls: {amd64: https://example.com/app-example.AppImage}
        sha256: {amd64: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}",
        &ubuntu("none", "amd64"),
    );
    assert_empty_manager_phase(&plan);
    assert_eq!(plan.phase(PlanPhaseKind::BinaryPackages).actions().len(), 2);
    assert_eq!(apt_refresh_count(&plan), 1);
}

#[test]
fn nerd_fonts_only_use_phase_one_capabilities_and_phase_fifteen_intent() {
    let plan = planned(
        "version: 1.0.0\nfonts: {nerd: [GeistMono]}",
        &ubuntu("none", "amd64"),
    );
    assert_empty_manager_phase(&plan);
    let PlannedAction::SystemPrerequisites(prerequisites) =
        &plan.phase(PlanPhaseKind::SystemPrerequisites).actions()[0]
    else {
        panic!("missing prerequisites")
    };
    assert!(prerequisites.contains(&SystemPrerequisite::HttpsCertificates));
    assert!(prerequisites.contains(&SystemPrerequisite::ArchiveExtraction));
    assert!(prerequisites.contains(&SystemPrerequisite::FontCache));
    assert!(matches!(
        plan.phase(PlanPhaseKind::Fonts).actions(),
        [PlannedAction::NerdFonts(fonts)] if fonts == &["GeistMono"]
    ));
}

#[test]
fn gnome_desktop_only_uses_phase_one_tools_and_keeps_provider_convergence_in_phase_eighteen() {
    let plan = planned(
        "version: 1.0.0
desktop:
  theme: dark
  gnome:
    extensions: [example@example.com]
    dock: true
    rounded_corners: true",
        &ubuntu("gnome", "amd64"),
    );
    assert_empty_manager_phase(&plan);
    let PlannedAction::SystemPrerequisites(prerequisites) =
        &plan.phase(PlanPhaseKind::SystemPrerequisites).actions()[0]
    else {
        panic!("missing prerequisites")
    };
    assert!(prerequisites.contains(&SystemPrerequisite::DesktopSettings));
    assert!(prerequisites.contains(&SystemPrerequisite::GnomeExtensionManagement));

    let provider_convergence_count = plan
        .actions()
        .filter(|action| {
            matches!(
                action,
                PlannedAction::Desktop(
                    DesktopIntent::GnomeDock(
                        ProviderConvergence::EnsureFixedProviderThenConfigureAndVerify
                    ) | DesktopIntent::GnomeRoundedCorners(
                        ProviderConvergence::EnsureFixedProviderThenConfigureAndVerify
                    )
                )
            )
        })
        .count();
    assert_eq!(provider_convergence_count, 2);
    assert!(plan
        .phases()
        .iter()
        .filter(|phase| phase.kind() != PlanPhaseKind::Desktop)
        .flat_map(|phase| phase.actions())
        .all(|action| !matches!(action, PlannedAction::Desktop(_))));
}

#[test]
fn system_prerequisites_and_fixed_manager_bootstraps_are_separate() {
    let plan = planned(
        "version: 1.0.0
packages:
  flatpak: [org.example.App]
  cargo: [bat]
  npm: [typescript]
tools: {rust: stable, go: latest, node: lts, python: '3.13'}
fonts: {nerd: [GeistMono]}",
        &ubuntu("none", "amd64"),
    );
    let PlannedAction::SystemPrerequisites(prerequisites) =
        &plan.phase(PlanPhaseKind::SystemPrerequisites).actions()[0]
    else {
        panic!("missing prerequisites")
    };
    let PlannedAction::ManagerBootstraps(managers) =
        &plan.phase(PlanPhaseKind::ManagerBootstraps).actions()[0]
    else {
        panic!("missing managers")
    };
    assert!(prerequisites.contains(&SystemPrerequisite::HttpsCertificates));
    assert!(prerequisites.contains(&SystemPrerequisite::FontCache));
    assert_eq!(managers.len(), 5);
    assert!(managers.contains(&ManagerBootstrap::Flatpak));
    assert!(managers.contains(&ManagerBootstrap::Rustup));
    assert!(managers.contains(&ManagerBootstrap::CargoBinstall));
    assert!(managers.contains(&ManagerBootstrap::Fnm));
    assert!(managers.contains(&ManagerBootstrap::Uv));
}

#[test]
fn tools_packages_fonts_and_updates_retain_moving_selectors_and_targets() {
    let plan = planned(EXHAUSTIVE, &ubuntu("gnome", "amd64"));
    assert!(plan
        .phase(PlanPhaseKind::LanguageToolchains)
        .actions()
        .iter()
        .any(|action| matches!(
            action,
            PlannedAction::Toolchain(ToolchainIntent::Rust { .. })
        )));
    let updates = plan.phase(PlanPhaseKind::Updates).actions();
    assert!(updates
        .iter()
        .any(|action| matches!(action, PlannedAction::Update(UpdateIntent::Rust { .. }))));
    assert!(updates.iter().any(|action| matches!(
        action,
        PlannedAction::Update(UpdateIntent::Cargo(packages))
            if packages.iter().map(String::as_str).eq(["cargo-edit"])
    )));
    assert!(updates.iter().any(|action| matches!(
        action,
        PlannedAction::Update(UpdateIntent::Npm(packages))
            if packages.iter().map(String::as_str).eq(["example-package"])
    )));
    assert!(updates.iter().any(|action| matches!(
        action,
        PlannedAction::Update(UpdateIntent::NerdFonts(fonts))
            if fonts.iter().map(String::as_str).eq(["GeistMono"])
    )));
}

#[test]
fn debian_retains_explicit_ubuntu_only_skips_without_apt_side_effects() {
    let plan = planned(
        "version: 1.0.0
system:
  ubuntu: {snap: disabled, codecs: installed}",
        &debian("none"),
    );
    let actions = plan.phase(PlanPhaseKind::SystemPackageStates).actions();
    assert_eq!(actions.len(), 2);
    assert!(
        actions.contains(&PlannedAction::Skip(cozydot::planner::PlatformSkip {
            intent: SkippedIntent::UbuntuSnap,
            reason: SkipReason::RequiresUbuntuFamily,
        }))
    );
    assert!(
        actions.contains(&PlannedAction::Skip(cozydot::planner::PlatformSkip {
            intent: SkippedIntent::UbuntuCodecs,
            reason: SkipReason::RequiresUbuntuFamily,
        }))
    );
    assert!(plan
        .phase(PlanPhaseKind::AptMetadataRefresh)
        .actions()
        .is_empty());
}

#[test]
fn desktop_ownership_terminal_precondition_and_fixed_gnome_convergence_are_explicit() {
    let gnome = planned(
        "version: 1.0.0
desktop:
  theme: dark
  terminal: wezterm
  idle: {timeout: 0s, dim: false}
  gnome: {dock: true, rounded_corners: true}",
        &ubuntu("gnome", "amd64"),
    );
    let actions = gnome.phase(PlanPhaseKind::Desktop).actions();
    assert!(actions.iter().any(|action| matches!(
        action,
        PlannedAction::Desktop(DesktopIntent::Terminal { target: DesktopTarget::Gnome, executable })
            if executable.exact_basename == "wezterm" && executable.timing == PreconditionTiming::AfterInstallPhases
    )));
    assert!(
        actions.contains(&PlannedAction::Desktop(DesktopIntent::GnomeDock(
            ProviderConvergence::EnsureFixedProviderThenConfigureAndVerify,
        )))
    );
    assert!(
        actions.contains(&PlannedAction::Desktop(DesktopIntent::GnomeRoundedCorners(
            ProviderConvergence::EnsureFixedProviderThenConfigureAndVerify,
        ),))
    );

    let cinnamon = planned(
        "version: 1.0.0
desktop:
  theme: dark
  terminal: wezterm
  idle: {timeout: 0s, dim: false}",
        &ubuntu("cinnamon", "amd64"),
    );
    assert!(cinnamon
        .phase(PlanPhaseKind::Desktop)
        .actions()
        .iter()
        .all(|action| matches!(
            action,
            PlannedAction::Desktop(
                DesktopIntent::Theme {
                    target: DesktopTarget::Cinnamon,
                    ..
                } | DesktopIntent::Terminal {
                    target: DesktopTarget::Cinnamon,
                    ..
                } | DesktopIntent::IdleTimeout {
                    target: DesktopTarget::Cinnamon,
                    ..
                } | DesktopIntent::IdleDim {
                    target: DesktopTarget::Cinnamon,
                    ..
                }
            )
        )));
}

#[test]
fn integrations_are_preflighted_existing_products_not_install_prerequisites() {
    let plan = planned(
        "version: 1.0.0
integrations:
  docker: {add_user_to_group: true, logging: {driver: local, max_size: 10m}}
  virtualbox: {add_user_to_group: true}
  vscode: {extensions: [rust-lang.rust-analyzer]}",
        &ubuntu("none", "amd64"),
    );
    assert!(plan
        .phase(PlanPhaseKind::SystemPrerequisites)
        .actions()
        .is_empty());
    assert!(plan
        .phase(PlanPhaseKind::ManagerBootstraps)
        .actions()
        .is_empty());
    let products = plan
        .phase(PlanPhaseKind::Integrations)
        .actions()
        .iter()
        .map(|action| match action {
            PlannedAction::Integration(integration) => integration.required_product,
            other => panic!("unexpected integration action: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        products,
        [
            ExistingProductRequirement::Docker,
            ExistingProductRequirement::Docker,
            ExistingProductRequirement::VirtualBox,
            ExistingProductRequirement::VsCode,
        ]
    );
}

#[test]
fn declared_order_is_preserved_and_intents_contain_no_generic_commands_or_interpolation() {
    let plan = planned(EXHAUSTIVE, &ubuntu("gnome", "amd64"));
    let debug = format!("{plan:#?}");
    for forbidden in ["CommandAction", "Shell(", "sudo ", "apt-get ", "${", "$("] {
        assert!(!debug.contains(forbidden), "found {forbidden:?} in {debug}");
    }
    let dotfiles = &plan.phase(PlanPhaseKind::Dotfiles).actions()[0];
    assert!(matches!(dotfiles, PlannedAction::Dotfiles(intent) if intent.packages == ["bash"]));
}
