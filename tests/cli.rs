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
fn canonical_init_and_dry_run_apply_succeeds() {
    let temp = tempfile::tempdir().unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .arg("init")
        .assert()
        .success();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_DRY_RUN", "1")
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::contains("summary:"));
}
