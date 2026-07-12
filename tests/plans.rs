use cozydot::{config::Config, planner, platform::Platform};
use std::{fs, path::Path};
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
    let cargo = text.find("workflow cargo-packages").unwrap();
    let node = text.find("workflow node-install").unwrap();
    assert!(cargo < node);
    assert!(text.contains("workflow node-install latest"));
    assert!(text.contains("latest opencode-ai"));
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

#[test]
fn apply_has_one_shared_check_and_no_internal_check_duplication() {
    let c = Config::load(Path::new("configs/default.yaml")).unwrap();
    let text = planner::plan_apply(&c, &platform(), Path::new("."))
        .unwrap()
        .iter()
        .map(|step| step.display())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(text.matches("[ -L ").count(), 1, "{text}");
    assert_eq!(text.matches("apt-get update -qq").count(), 3, "{text}");
}

#[test]
fn repository_architecture_is_resolved_before_stdin_write() {
    let c = Config::load(Path::new("configs/full.yaml")).unwrap();
    let steps = planner::plan("install", &c, &platform(), Path::new(".")).unwrap();
    let repo_writes = steps
        .iter()
        .filter(|s| s.display().contains("/etc/apt/sources.list.d/"))
        .filter_map(|s| s.command().and_then(|command| command.stdin.as_deref()))
        .collect::<Vec<_>>();
    assert!(repo_writes.iter().any(|s| s.contains("arch=amd64")));
    assert!(repo_writes.iter().all(|s| !s.contains("$(")));
}

#[test]
fn pinning_block_is_written_as_exact_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pin.yaml");
    fs::write(
        &path,
        config_with_repo_pinning("Package: foo\nPin: origin example\nPin-Priority: 1001\n"),
    )
    .unwrap();
    let c = Config::load(&path).unwrap();
    let steps = planner::plan("install", &c, &platform(), Path::new(".")).unwrap();
    let pin = steps
        .iter()
        .find(|s| s.display().contains("/etc/apt/preferences.d/example"))
        .unwrap();
    assert_eq!(
        pin.command().and_then(|command| command.stdin.as_deref()),
        Some("Package: foo\nPin: origin example\nPin-Priority: 1001\n")
    );
}

#[test]
fn binary_and_language_steps_are_state_aware() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pyenv.yaml");
    let yaml = fs::read_to_string("configs/full.yaml")
        .unwrap()
        .replace("pyenv: !disabled", "pyenv: !enabled");
    fs::write(&path, yaml).unwrap();
    let c = Config::load(&path).unwrap();
    let text = planner::plan("install", &c, &platform(), Path::new("."))
        .unwrap()
        .iter()
        .map(|x| x.display())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("workflow download-binary"));
    assert!(text.contains("workflow go-install"));
    assert!(text.contains("workflow uv-install"));
    assert!(text.contains("workflow pyenv-install"));
}

#[test]
fn configure_plan_contains_stateful_app_and_gnome_behavior() {
    let c = Config::load(Path::new("configs/full.yaml")).unwrap();
    let text = planner::plan("configure", &c, &platform(), Path::new("/repo"))
        .unwrap()
        .iter()
        .map(|x| x.display())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("workflow docker-config"));
    assert!(text.contains("workflow vscode-extension"));
    assert!(text.contains("workflow gnome-terminal"));
    assert!(text.contains("idle-dim"));
    assert!(text.contains("workflow gnome-dependencies"));
    assert!(text.contains("workflow gnome-dock-settings"));
    assert!(text.contains("workflow gnome-rounded-corners-settings"));
}

#[test]
fn malformed_config_values_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.yaml");
    let mut yaml = config_with_repo_pinning("false\n");
    yaml = yaml.replace(
        "nerdfont: !enabled GeistMono",
        "nerdfont: !enabled \"bad'; touch /tmp/pwn\"",
    );
    fs::write(&path, yaml).unwrap();
    let err = Config::load(&path).unwrap_err().to_string();
    assert!(err.contains("validate"));
}

#[test]
fn unsupported_binary_suffix_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad-bin.yaml");
    let yaml = config_with_repo_pinning("false\n").replace("name: tool.deb", "name: tool.tar.gz");
    fs::write(&path, yaml).unwrap();
    let err = Config::load(&path).unwrap_err().to_string();
    assert!(err.contains("validate"));
}

fn config_with_repo_pinning(pinning: &str) -> String {
    format!(
        r#"metadata:
  description: test
  distro: ubuntu
  DE: gnome
check:
  distroCfg: false
  purge: !disabled []
  deps: !disabled []
  rustupCheck: false
  appimaged: false
  nerdfont: !enabled GeistMono
install:
  check: false
  apt: !disabled []
  addRepos: !enabled
    - sourceName: example
      remoteKey: https://example.test/key.asc
      keyPath: /etc/apt/keyrings/example.asc
      repo: deb [arch=$(dpkg --print-architecture)] https://example.test stable main
      pinning: |-
{pinning_indented}
      packages:
        - example
  flatpak: !disabled []
  cargo: !disabled []
  npm: !disabled []
  binaries: !enabled
    - name: tool.deb
      url: https://example.test/tool.deb
  languages:
    goVersion: !disabled latest
    nodeVersion: !disabled latest
    pyenv: !disabled
      update: false
      version: 3
      pip: false
    uv: !disabled
      version: !disabled 3.13
update:
  check: false
  apt: !disabled
    aptFull: false
  flatpak: false
  cargo: false
  other:
    go: false
    node: false
configure:
  check: false
  dotfiles: !disabled
    stowMode: backup
    packages: []
  apps:
    docker: false
    virtualbox: false
    vscodeExtensions: !disabled []
  desktopEnvironment: !disabled
    common: !disabled
      defaultTerm: !enabled wezterm
    gnome: !disabled
      settings: false
      extensions: !disabled []
      MacOSDock: false
      smoothRoundedCorners: false
    cinnamon: !disabled
"#,
        pinning_indented = pinning
            .lines()
            .map(|l| format!("        {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}
