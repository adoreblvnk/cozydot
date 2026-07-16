use crate::{
    config::Config,
    planner::{lower_neutral::lower, plan},
    platform::Platform,
    runner::{Step, StepKind},
};
use std::path::Path;

fn platform(distro: &str, upstream: &str, desktop: &str) -> Platform {
    Platform::from_parts(
        distro.into(),
        upstream.into(),
        if distro == "debian" {
            "trixie"
        } else {
            "noble"
        }
        .into(),
        desktop.into(),
        "x86_64",
    )
    .unwrap()
}

fn preset(name: &str, platform: &Platform) -> Vec<Step> {
    let path = if name == "full" {
        "docs/examples/full.yaml".to_owned()
    } else {
        format!("configs/{name}.yaml")
    };
    let config = Config::load(Path::new(&path)).unwrap();
    let plan = plan(&config, platform, Path::new("/dotfiles")).unwrap();
    lower(&plan).unwrap()
}

fn text(steps: &[Step]) -> String {
    steps
        .iter()
        .filter(|step| matches!(step.kind(), StepKind::Operation { .. }))
        .map(Step::display)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_preset_uses_version_1_0_0_and_lowers_on_ubuntu_and_debian() {
    for name in ["default", "cli", "full", "vm"] {
        for resolved in [
            platform("ubuntu", "ubuntu", "gnome"),
            platform("debian", "debian", "gnome"),
        ] {
            let steps = preset(name, &resolved);
            assert!(!steps.is_empty(), "{name}");
        }
    }
}

#[test]
fn only_full_explicitly_configures_existing_docker_and_virtualbox_products() {
    let resolved = platform("ubuntu", "ubuntu", "gnome");
    for name in ["default", "cli", "vm"] {
        let plan = text(&preset(name, &resolved));
        for product in ["docker", "virtualbox"] {
            assert!(!plan.contains(product), "{name}: {plan}");
        }
    }

    let full = text(&preset("full", &resolved));
    for expected in ["docker-group", "docker-local-log 10m", "virtualbox-group"] {
        assert!(full.contains(expected), "full: missing {expected}: {full}");
    }
}

#[test]
fn cli_preset_has_no_desktop_flatpak_or_vscode_actions() {
    let plan = text(&preset("cli", &platform("ubuntu", "ubuntu", "gnome")));
    for excluded in ["desktop-setting", "gnome-", "flatpak-", "vscode-"] {
        assert!(!plan.contains(excluded), "{plan}");
    }
    assert!(plan.contains("cargo-package-set update-current"));
    assert!(plan.contains("npm-package-set update-current"));
}

#[test]
fn vm_preserves_full_desktop_controls() {
    let plan = text(&preset("vm", &platform("ubuntu", "ubuntu", "gnome")));
    assert!(plan.contains(" terminal wezterm"), "{plan}");
    assert!(plan.contains("gnome-dock"), "{plan}");
    assert!(plan.contains("desktop-setting gnome theme dark"), "{plan}");
    assert!(plan.contains("gnome-rounded-corners"), "{plan}");
}

#[test]
fn dotfiles_precede_desktop_and_full_uses_full_apt_update() {
    let plan = text(&preset("full", &platform("ubuntu", "ubuntu", "gnome")));
    assert!(plan.find("dotfiles-backup-stow").unwrap() < plan.find("desktop-setting").unwrap());
    assert!(plan.contains("apt-upgrade full"), "{plan}");
    assert_eq!(plan.matches("apt-metadata-refresh").count(), 1, "{plan}");
}
