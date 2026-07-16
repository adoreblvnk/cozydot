use crate::{
    config::Config,
    operations::{BinaryPackageMode, BinarySourceOperation, Operation},
    planner::{lower_neutral::lower, plan},
    platform::Platform,
    runner::{ExecutionPhase, SkipReason, SkippedAction, Step, StepKind},
};
use std::path::Path;

const FULL: &str = include_str!("../../configs/full.yaml");

fn platform(distro: &str) -> Platform {
    let codename = if distro == "ubuntu" {
        "noble"
    } else {
        "trixie"
    };
    Platform::from_release_parts(
        distro.into(),
        distro.into(),
        codename.into(),
        codename.into(),
        "gnome".into(),
        "amd64",
    )
    .unwrap()
}

fn lowered(yaml: &str, distro: &str) -> Vec<Step> {
    let config = Config::parse(yaml).unwrap();
    let plan = plan(&config, &platform(distro), Path::new("/dotfiles")).unwrap();
    lower(&plan).unwrap()
}

fn operation(step: &Step) -> Option<&Operation> {
    match step.kind() {
        StepKind::Operation { operation, .. } => Some(operation),
        _ => None,
    }
}

#[test]
fn minimal_plan_retains_all_phase_boundaries_and_final_summary() {
    let steps = lowered("version: 1.0.0", "ubuntu");
    let phases = steps
        .iter()
        .filter_map(|step| match step.kind() {
            StepKind::Phase(phase) => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(phases, ExecutionPhase::ORDERED);
    assert!(matches!(steps.last().unwrap().kind(), StepKind::Summary));
    assert_eq!(steps.len(), 21);
}

#[test]
fn full_fixture_lowers_on_both_mandatory_hosts() {
    for distro in ["ubuntu", "debian"] {
        let steps = lowered(FULL, distro);
        assert_eq!(
            steps
                .iter()
                .filter(|step| matches!(step.kind(), StepKind::Phase(_)))
                .count(),
            20,
            "full on {distro}"
        );
        assert!(matches!(steps.last().unwrap().kind(), StepKind::Summary));
    }
}

#[test]
fn full_lowering_preserves_repository_refresh_and_labels() {
    let steps = lowered(FULL, "ubuntu");
    let mut sources = Vec::new();
    let mut repository_labels = Vec::new();
    for step in &steps {
        if let Some(Operation::AptRepository(repository)) = operation(step) {
            sources.push(repository.render_source());
        }
        if let StepKind::Operation {
            operation,
            label: Some(label),
        } = step.kind()
        {
            if matches!(operation.as_ref(), Operation::AptPackages { .. }) {
                repository_labels.push(label.as_str());
            }
        }
    }
    assert!(sources.iter().any(|source| source.ends_with(" * *\n")));

    assert_eq!(
        repository_labels,
        [
            "repository docker",
            "repository debian-griffo",
            "repository github-cli",
            "repository helium",
            "repository onlyoffice",
            "repository virtualbox",
            "repository vscode",
            "repository wezterm",
        ]
    );

    let refresh = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| matches!(operation(step), Some(Operation::AptMetadataRefresh)))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(refresh.len(), 1);
    let refresh = refresh[0];
    assert!(steps[..refresh]
        .iter()
        .any(|step| matches!(operation(step), Some(Operation::AptRepository(_)))));
    assert!(steps[refresh + 1..].iter().any(|step| {
        matches!(
            operation(step),
            Some(Operation::AptPurge { .. } | Operation::AptPackages { .. })
        )
    }));
}

#[test]
fn full_binary_sources_and_update_modes_survive_exactly() {
    let steps = lowered(FULL, "ubuntu");
    let binaries = steps
        .iter()
        .filter_map(|step| match operation(step) {
            Some(Operation::BinaryPackage(binary)) => Some(binary),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(binaries.len(), 10, "five ensures and five GitHub updates");

    let mut github_modes = Vec::new();
    for binary in binaries {
        assert!(matches!(
            binary.source(),
            BinarySourceOperation::GithubLatest { sha256: None, .. }
        ));
        github_modes.push(binary.mode());
    }
    assert_eq!(
        github_modes
            .iter()
            .filter(|mode| **mode == BinaryPackageMode::EnsurePresent)
            .count(),
        5
    );
    assert_eq!(
        github_modes
            .iter()
            .filter(|mode| **mode == BinaryPackageMode::Update)
            .count(),
        5
    );
}

#[test]
fn selectors_modes_terminal_and_provider_operations_remain_typed() {
    let partial = lowered(
        "version: 1.0.0\ntools: {rust: '1.85', go: '1.22', node: '22', python: '3.13'}",
        "ubuntu",
    )
    .iter()
    .map(Step::display)
    .collect::<Vec<_>>()
    .join("\n");
    for expected in [
        "rust-toolchain ensure-present 1.85 x86_64-unknown-linux-gnu",
        "go-toolchain ensure-present 1.22 amd64",
        "node-toolchain ensure-present 22 amd64",
        "python-toolchain 3.13 amd64",
    ] {
        assert!(partial.contains(expected), "{partial}");
    }

    let steps = lowered(FULL, "ubuntu");
    let displays = steps.iter().map(Step::display).collect::<Vec<_>>();
    for expected in [
        "workflow binary-package drawio ensure-present",
        "workflow binary-package drawio update",
        "workflow nerd-fonts update GeistMono",
        "workflow cargo-package-set update-current bat bottom du-dust fd-find presenterm starship tealdeer",
        "workflow npm-package-set update-current opencode-ai",
        "workflow gnome-dock",
        "workflow gnome-rounded-corners",
    ] {
        assert!(
            displays.iter().any(|display| display == expected),
            "{expected}"
        );
    }
    let terminal = displays
        .iter()
        .position(|display| display == "workflow desktop-setting gnome terminal wezterm")
        .unwrap();
    for install in [
        "workflow rust-toolchain ensure-present",
        "workflow go-toolchain ensure-present",
        "workflow node-toolchain ensure-present",
        "workflow python-toolchain",
        "workflow cargo-package-set ensure-present",
        "workflow npm-package-set ensure-present",
        "workflow binary-package drawio ensure-present",
    ] {
        assert!(
            displays
                .iter()
                .position(|display| display.starts_with(install))
                .unwrap()
                < terminal,
            "{install} must precede terminal preflight/configuration"
        );
    }

    let bootstrap = displays
        .iter()
        .find(|display| display.starts_with("workflow apt-bootstrap-packages"))
        .unwrap();
    for package in ["flatpak", "tar", "unzip"] {
        assert!(
            bootstrap.split_whitespace().any(|part| part == package),
            "{bootstrap}"
        );
    }
    for existing_product in ["docker", "virtualbox", "code"] {
        assert!(!bootstrap
            .split_whitespace()
            .any(|part| part == existing_product));
    }
}

#[test]
fn debian_ubuntu_controls_are_visible_skips_without_control_side_effects() {
    let steps = lowered(FULL, "debian");
    assert!(steps.iter().any(|step| {
        matches!(
            step.kind(),
            StepKind::Skip(skip)
                if skip.action == SkippedAction::UbuntuSnap
                    && skip.reason == SkipReason::RequiresUbuntuFamily
        )
    }));
    assert!(steps.iter().any(|step| {
        matches!(
            step.kind(),
            StepKind::Skip(skip)
                if skip.action == SkippedAction::UbuntuCodecs
                    && skip.reason == SkipReason::RequiresUbuntuFamily
        )
    }));
    let displays = steps
        .iter()
        .map(Step::display)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!displays.contains("workflow ubuntu-snap"), "{displays}");
    assert!(!displays.contains("ubuntu-restricted-extras"), "{displays}");
}
