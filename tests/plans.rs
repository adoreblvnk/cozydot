use cozydot::{
    config::v1::ConfigV1,
    planner::{lower_v1::lower, v1::plan},
    platform::Platform,
    runner::Step,
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
    let path = format!("configs/{name}.yaml");
    let config = ConfigV1::load(Path::new(&path)).unwrap();
    let plan = plan(&config, platform, Path::new("/dotfiles")).unwrap();
    lower(&plan).unwrap()
}

fn text(steps: &[Step]) -> String {
    steps
        .iter()
        .map(Step::display)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_preset_is_schema_v1_and_lowers_on_ubuntu_and_debian() {
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
fn presets_do_not_implicitly_install_or_configure_docker_or_virtualbox() {
    for name in ["default", "cli", "full", "vm"] {
        let plan = text(&preset(name, &platform("ubuntu", "ubuntu", "gnome")));
        for product in ["docker", "virtualbox"] {
            assert!(!plan.contains(product), "{name}: {plan}");
        }
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
fn vm_omits_terminal_and_dock_but_keeps_declared_desktop_controls() {
    let plan = text(&preset("vm", &platform("ubuntu", "ubuntu", "gnome")));
    assert!(!plan.contains(" terminal "), "{plan}");
    assert!(!plan.contains("gnome-dock"), "{plan}");
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
