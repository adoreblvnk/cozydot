use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

#[test]
fn help_and_version() {
    for args in [Vec::<&str>::new(), vec!["--help"]] {
        Command::cargo_bin("cozydot")
            .unwrap()
            .args(args)
            .assert()
            .success()
            .stdout(predicate::str::contains("init").and(predicate::str::contains("apply")));
    }

    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn missing_config_fails_apply() {
    let temp = tempfile::tempdir().unwrap();
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("active config is missing or invalid"));
}

#[test]
fn unknown_preset_rejected() {
    let temp = tempfile::tempdir().unwrap();
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .args(["init", "--preset", "invalid_preset_name"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'invalid_preset_name'"));
}

#[test]
fn init_materializes_four_presets() {
    let presets = ["cozydot", "full", "cli", "vm"];
    for preset in presets {
        let temp = tempfile::tempdir().unwrap();
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .args(["init", "--preset", preset])
            .assert()
            .success();

        let config_path = temp.path().join("cozydot/cozydot.yaml");
        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        let expected = fs::read_to_string(format!("configs/{preset}.yaml")).unwrap();
        assert_eq!(content, expected);
    }
}

#[test]
fn init_updates_unmodified_files_and_preserves_modified_files() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("cozydot/cozydot.yaml");
    let dotfile_path = temp.path().join("cozydot/dotfiles/bash/.bashrc");

    for preset in ["cli", "vm"] {
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .args(["init", "--preset", preset])
            .assert()
            .success();
    }
    assert_eq!(fs::read_to_string(&config_path).unwrap(), fs::read_to_string("configs/vm.yaml").unwrap());

    fs::write(&config_path, "user-owned config\n").unwrap();
    fs::write(&dotfile_path, "user-owned dotfile\n").unwrap();
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .args(["init", "--preset", "full"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(config_path).unwrap(), "user-owned config\n");
    assert_eq!(fs::read_to_string(dotfile_path).unwrap(), "user-owned dotfile\n");
}

#[test]
fn init_preserves_unmanaged_existing_config_and_dotfile() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("cozydot");
    let config_path = root.join("cozydot.yaml");
    let dotfile_path = root.join("dotfiles/bash/.bashrc");
    fs::create_dir_all(dotfile_path.parent().unwrap()).unwrap();
    fs::write(&config_path, "existing config\n").unwrap();
    fs::write(&dotfile_path, "existing dotfile\n").unwrap();

    Command::cargo_bin("cozydot").unwrap().env("XDG_CONFIG_HOME", temp.path()).arg("init").assert().success();

    assert_eq!(fs::read_to_string(config_path).unwrap(), "existing config\n");
    assert_eq!(fs::read_to_string(dotfile_path).unwrap(), "existing dotfile\n");
}

#[test]
fn init_ignores_removed_failure_injection_environment_variables() {
    let temp = tempfile::tempdir().unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .env("COZYDOT_TEST_FAIL_AFTER_INSTALLS", "1")
        .env("COZYDOT_TEST_FAIL_AFTER_RELATIVE", "cozydot.yaml")
        .env("COZYDOT_TEST_FAIL_MANAGED_FILE_AT", "cp")
        .arg("init")
        .assert()
        .success();
}

#[test]
fn empty_config_apply_has_no_synthetic_report_output() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("cozydot");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("cozydot.yaml"), "version: 1.0.0\n").unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn standard_yaml_null_is_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("cozydot");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("cozydot.yaml"), "version: 1.0.0\nsystem: null\n").unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn invalid_yaml_fails_apply() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("cozydot");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("cozydot.yaml"), "version: [\n").unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("active config is missing or invalid"));
}

#[test]
fn unsupported_architecture_selector_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("cozydot");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        r#"version: "1.0.0"
packages:
  binaries:
    - name: unsupported
      format: appimage
      commands: [unsupported]
      source:
        provider: github
        repository: example/unsupported
        assets:
          riscv64: ^unsupported$
"#,
    )
    .unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown field `riscv64`"));
}

#[test]
fn empty_sections_and_false_enable_flags_are_noops() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("cozydot");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        r#"version: 1.0.0
system:
  require:
    distros: []
    desktops: []
  ensure_admin: false
  apt: {}
  ubuntu: {}
packages:
  apt:
    remove: []
    install: []
    repositories: []
  flatpak: []
  cargo: []
  npm: []
  binaries: []
tools: {}
fonts:
  nerd: []
dotfiles:
  packages: []
integrations:
  docker:
    add_user_to_group: false
  virtualbox:
    add_user_to_group: false
  vscode:
    extensions: []
desktop:
  idle: {}
  gnome:
    extensions: []
    dock: false
    rounded_corners: false
updates:
  flatpak: false
  fonts: false
  tools:
    rust: false
    go: false
    node: false
  packages:
    cargo: false
    npm: false
    binaries: false
"#,
    )
    .unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn true_updates_require_nonempty_targets_and_rendered_values_stay_valid() {
    for (config, message) in [
        (
            "version: 1.0.0\npackages:\n  flatpak: []\nupdates:\n  flatpak: true\n",
            "updates.flatpak: requires configured packages.flatpak targets",
        ),
        (
            "version: 1.0.0\nfonts:\n  nerd: []\nupdates:\n  fonts: true\n",
            "updates.fonts: requires configured fonts.nerd targets",
        ),
        (
            "version: 1.0.0\npackages:\n  cargo: []\nupdates:\n  packages:\n    cargo: true\n",
            "updates.packages.cargo: requires configured packages.cargo targets",
        ),
        (
            "version: 1.0.0\npackages:\n  npm: []\nupdates:\n  packages:\n    npm: true\n",
            "updates.packages.npm: requires configured packages.npm targets",
        ),
        (
            "version: 1.0.0\nintegrations:\n  docker:\n    logging:\n      driver: local\n      max_size: invalid\n",
            "invalid Docker size",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("cozydot");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("cozydot.yaml"), config).unwrap();

        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .arg("apply")
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}
