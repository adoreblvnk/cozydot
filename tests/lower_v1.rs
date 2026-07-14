use cozydot::{
    config::v1::ConfigV1,
    planner::{lower_v1::lower, v1::plan},
    platform::Platform,
    runner::Step,
};
use std::path::Path;

const FULL: &str = include_str!("fixtures/config-v1-full.yaml");

fn ubuntu(desktop: &str) -> Platform {
    Platform::from_parts(
        "ubuntu".into(),
        "ubuntu".into(),
        "noble".into(),
        desktop.into(),
        "x86_64",
    )
    .unwrap()
}

fn lowered(yaml: &str, desktop: &str) -> Vec<Step> {
    let config = ConfigV1::parse(yaml).unwrap();
    let plan = plan(&config, &ubuntu(desktop), Path::new("/dotfiles")).unwrap();
    lower(&plan).unwrap()
}

#[test]
fn complete_plan_lowers_every_action_to_fixed_workflows() {
    let steps = lowered(FULL, "gnome");
    assert!(steps.iter().all(|step| matches!(step, Step::Workflow(_))));
    let text = steps
        .iter()
        .map(Step::display)
        .collect::<Vec<_>>()
        .join("\n");

    for expected in [
        "workflow apt-bootstrap-packages",
        "workflow rustup-bootstrap",
        "workflow fnm-bootstrap",
        "workflow uv-bootstrap",
        "workflow flatpak-ensure-flathub",
        "workflow ensure-admin",
        "workflow managed-apt-sources ubuntu noble amd64",
        "workflow unattended-upgrades false",
        "workflow ubuntu-snap false",
        "workflow repository-key /etc/apt/keyrings/cozydot-github-cli.gpg",
        "workflow apt-source /etc/apt/sources.list.d/cozydot-github-cli.list",
        "workflow apt-metadata-refresh",
        "workflow rust-toolchain ensure-present stable x86_64-unknown-linux-gnu",
        "workflow go-toolchain ensure-present latest amd64",
        "workflow node-toolchain ensure-present lts",
        "workflow python-toolchain 3.13",
        "workflow cargo-package-set ensure-present bat starship",
        "workflow npm-package-set ensure-present opencode-ai",
        "workflow direct-package obsidian ensure-present",
        "workflow nerd-fonts GeistMono",
        "workflow dotfiles-backup-stow bash starship",
        "workflow docker-group",
        "workflow docker-local-log 10m",
        "workflow virtualbox-group",
        "workflow vscode-extension-set rust-lang.rust-analyzer",
        "workflow desktop-setting gnome theme dark",
        "workflow desktop-setting gnome terminal wezterm",
        "workflow desktop-setting gnome idle-timeout-seconds 900",
        "workflow desktop-setting gnome idle-dim false",
        "workflow gnome-extensions blur-my-shell@aunetx",
        "workflow gnome-dock",
        "workflow gnome-rounded-corners",
        "workflow apt-upgrade standard",
        "workflow flatpak-update-apps com.bitwarden.desktop",
        "workflow cargo-package-set update-current bat starship",
        "workflow npm-package-set update-current opencode-ai",
        "workflow direct-package obsidian update",
    ] {
        assert!(text.contains(expected), "missing {expected:?}:\n{text}");
    }
    assert_eq!(text.matches("workflow apt-metadata-refresh").count(), 1);
    assert_eq!(text.matches("workflow flatpak-ensure-flathub").count(), 1);
    for legacy in [
        "workflow go-install",
        "workflow node-install",
        "workflow uv-install",
        "workflow cargo-packages",
        "workflow npm-packages",
        "workflow nerdfont ",
    ] {
        assert!(!text.contains(legacy), "legacy lowering remained: {legacy}");
    }
}

#[test]
fn bootstrap_precedes_sources_and_consumers() {
    let displays = lowered(FULL, "gnome")
        .iter()
        .map(Step::display)
        .collect::<Vec<_>>();
    let position = |needle: &str| {
        displays
            .iter()
            .position(|step| step.contains(needle))
            .unwrap_or_else(|| panic!("missing {needle}"))
    };
    assert!(position("apt-bootstrap-packages") < position("managed-apt-sources"));
    assert!(position("managed-apt-sources") < position("apt-metadata-refresh"));
    assert!(position("apt-metadata-refresh") < position("apt-packages gh"));
    assert!(position("fnm-bootstrap") < position("node-toolchain"));
    assert!(position("node-toolchain") < position("npm-package-set ensure-present"));
}

#[test]
fn desktop_mismatch_remains_a_planner_skip_during_lowering() {
    let text = lowered(FULL, "none")
        .iter()
        .map(Step::display)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("desktop-setting"));
    assert!(!text.contains("gnome-extensions"));
    assert!(!text.contains("gnome-dock"));
    assert!(!text.contains("gnome-rounded-corners"));
}

#[test]
fn minimal_config_lowers_to_no_steps() {
    assert!(lowered("schema: 1\n", "none").is_empty());
}
