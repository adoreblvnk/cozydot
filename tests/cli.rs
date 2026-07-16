use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_has_public_contract_only() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("init")
                .and(predicate::str::contains("apply"))
                .and(predicate::str::contains("--preset"))
                .and(predicate::str::contains("--config").not())
                .and(predicate::str::contains("plan").not()),
        );
}

#[test]
fn version_works() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "cozydot {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn removed_commands_are_rejected() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("--list-configs")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown command"));
}

#[test]
fn apply_requires_active_config() {
    let home = tempfile::tempdir().unwrap();
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("apply")
        .env("HOME", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .assert()
        .failure()
        .stderr(predicate::str::contains("cozydot init"));
}

#[test]
fn init_emits_version_1_0_0_and_dry_apply_uses_the_only_runtime_path() {
    let root = tempfile::tempdir().unwrap();
    let config_home = root.path().join("config");
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("init")
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", &config_home)
        .assert()
        .success();

    let config = std::fs::read_to_string(config_home.join("cozydot/cozydot.yaml")).unwrap();
    assert_eq!(config, include_str!("../configs/cozydot.yaml"));
    cozydot::config::Config::parse(&config).unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("apply")
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "GNOME")
        .env("COZYDOT_DRY_RUN", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("summary:"));
}

#[test]
fn init_materializes_each_embedded_preset() {
    for (preset, expected) in [
        ("cozydot", include_str!("../configs/cozydot.yaml")),
        ("full", include_str!("../configs/full.yaml")),
        ("cli", include_str!("../configs/cli.yaml")),
        ("vm", include_str!("../configs/vm.yaml")),
    ] {
        let root = tempfile::tempdir().unwrap();
        let config_home = root.path().join("config");
        Command::cargo_bin("cozydot")
            .unwrap()
            .args(["init", "--preset", preset])
            .env("HOME", root.path())
            .env("XDG_CONFIG_HOME", &config_home)
            .assert()
            .success();

        assert_eq!(
            std::fs::read_to_string(config_home.join("cozydot/cozydot.yaml")).unwrap(),
            expected,
            "wrong materialized preset: {preset}"
        );
    }
}

#[test]
fn init_rejects_unknown_preset() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .args(["init", "--preset", "unknown"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown preset 'unknown'"));
}

#[test]
fn init_help_and_equals_form_work() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--preset <PRESET>"));

    let root = tempfile::tempdir().unwrap();
    let config_home = root.path().join("config");
    Command::cargo_bin("cozydot")
        .unwrap()
        .args(["init", "--preset=cli"])
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", &config_home)
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(config_home.join("cozydot/cozydot.yaml")).unwrap(),
        include_str!("../configs/cli.yaml")
    );
}

#[test]
fn init_switches_an_unchanged_active_preset() {
    let root = tempfile::tempdir().unwrap();
    let config_home = root.path().join("config");
    for preset in ["full", "cli"] {
        Command::cargo_bin("cozydot")
            .unwrap()
            .args(["init", "--preset", preset])
            .env("HOME", root.path())
            .env("XDG_CONFIG_HOME", &config_home)
            .assert()
            .success();
    }
    assert_eq!(
        std::fs::read_to_string(config_home.join("cozydot/cozydot.yaml")).unwrap(),
        include_str!("../configs/cli.yaml")
    );
}
