use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
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
        .stdout(predicate::str::contains(format!(
            "cozydot {}",
            env!("CARGO_PKG_VERSION")
        )));
}
#[test]
fn aliases_and_dry_run_work() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .args(["-c", "cli", "i"])
        .env("COZYDOT_DRY_RUN", "1")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("npm install --global")
                .and(predicate::str::contains("latest opencode-ai")),
        );
}
#[test]
fn no_color_and_multiple_commands_work() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .args(["--no-color", "-c", "cli", "check", "update"])
        .env("COZYDOT_DRY_RUN", "1")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Finished cozydot check")
                .and(predicate::str::contains("Finished cozydot update")),
        );
}
#[test]
fn config_must_be_named_preset() {
    Command::cargo_bin("cozydot")
        .unwrap()
        .args(["--config", "configs/cli.yaml", "check"])
        .env("COZYDOT_DRY_RUN", "1")
        .assert()
        .failure()
        .stderr(predicate::str::contains("configs/"));
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

#[test]
fn extracted_bundle_layout_smoke() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("bundle")).unwrap();
    copy_dir("configs", &dir.path().join("bundle/configs"));
    copy_dir("dotfiles", &dir.path().join("bundle/dotfiles"));
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("--list-configs")
        .env("COZYDOT_ROOT", dir.path().join("bundle"))
        .assert()
        .success()
        .stdout(predicate::str::contains("default:"));
}

fn copy_dir(src: &str, dst: &std::path::Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir(path.to_str().unwrap(), &target);
        } else {
            fs::copy(path, target).unwrap();
        }
    }
}
