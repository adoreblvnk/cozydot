use cozydot::{config::Config, planner, platform::Platform};
use std::path::Path;
fn platform() -> Platform {
    Platform::from_parts(
        "ubuntu".into(),
        "ubuntu".into(),
        "noble".into(),
        "gnome".into(),
        "x86_64",
    )
    .unwrap()
}
#[test]
fn parses_every_preset() {
    for n in ["default", "cli", "full", "vm"] {
        let c = Config::load(Path::new(&format!("configs/{n}.yaml"))).unwrap();
        assert!(!c.strings("install.cargo").is_empty());
    }
}
#[test]
fn install_order_and_integrations() {
    let c = Config::load(Path::new("configs/cli.yaml")).unwrap();
    let s = planner::plan("install", &c, &platform(), Path::new(".")).unwrap();
    let text = s.iter().map(|x| x.display()).collect::<Vec<_>>().join("\n");
    let boot = text.find("cargo install cargo-binstall").unwrap();
    let bins = text.find("cargo binstall").unwrap();
    assert!(boot < bins);
    assert!(text.contains("fnm install --lts"));
    assert!(text.contains("npm install --global opencode-ai"));
    assert!(!text.contains("flatpak install"));
}
#[test]
fn update_uses_binstall_force() {
    let c = Config::load(Path::new("configs/cli.yaml")).unwrap();
    let text = planner::plan("update", &c, &platform(), Path::new("."))
        .unwrap()
        .iter()
        .map(|x| x.display())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("binstall --no-confirm --force"));
}
#[test]
fn configure_stow_precedes_desktop() {
    let c = Config::load(Path::new("configs/full.yaml")).unwrap();
    let s = planner::plan("configure", &c, &platform(), Path::new("/repo")).unwrap();
    let text = s.iter().map(|x| x.display()).collect::<Vec<_>>().join("\n");
    assert!(text.find("stow").unwrap() < text.find("gsettings").unwrap());
}
