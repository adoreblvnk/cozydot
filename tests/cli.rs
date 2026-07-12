use assert_cmd::Command;
use predicates::prelude::*;
#[test]
fn help_has_contract() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("check").and(predicate::str::contains("--list-configs")));
}
#[test]
fn version_works() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains("cozydot 0.1.0"));
}
#[test]
fn aliases_and_dry_run_work() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .args(["-c", "cli", "i"])
        .env("COZYDOT_DRY_RUN", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("npm install --global opencode-ai"));
}
#[test]
fn lists_configs() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("--list-configs")
        .assert()
        .success()
        .stdout(predicate::str::contains("default:").and(predicate::str::contains("vm:")));
}
