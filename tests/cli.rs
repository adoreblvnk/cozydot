//! Integration tests ordered from CLI surface to local state, system mutation, and installation.

mod util;

use predicates::prelude::*;
use std::{fs, process::Command as StdCommand};
use util::{MINIMAL_CONFIG, TestEnv};

// --help & --version work, parameterless invocation prints help instead of failing
#[test]
fn cli_accepts_standard_flags() {
    let env = TestEnv::new();
    let cozydot = || env.cozydot();

    cozydot().arg("--help").assert().success().stdout(predicate::str::contains("Usage:"));

    cozydot().arg("--version").assert().success().stdout(predicate::str::contains("cozydot"));

    // parameterless invocation prints help instead of failing
    cozydot().assert().success().stdout(predicate::str::contains("Usage:"));
}

// a config without `version: 1` fails check
#[test]
fn cli_rejects_unversioned_or_legacy_configs() {
    let env = TestEnv::new();

    // scaffold the current config, then strip its version field
    env.cozydot().arg("init").assert().success();
    let path = env.root().join("cozydot/cozydot.yaml");
    let config = fs::read_to_string(&path).unwrap();
    fs::write(&path, config.replace("version: 1\n", "")).unwrap();

    env.cozydot().arg("check").assert().failure().stderr(predicate::str::contains("version"));
}

// 2nd init succeeds without overwriting a user-tuned cozydot.yaml
#[test]
fn init_scaffolds_default_config_idempotently() {
    let env = TestEnv::new();
    let active_config = env.root().join("cozydot/cozydot.yaml");

    env.cozydot().arg("init").assert().success();
    assert!(active_config.exists());

    fs::write(&active_config, "user tuned edit\n").unwrap();
    env.cozydot().arg("init").assert().success();

    assert_eq!(fs::read_to_string(&active_config).unwrap(), "user tuned edit\n");
}

// re-init skips dotfiles whose content no longer matches the .managed-files ledger
#[test]
fn init_preserves_user_edits_via_manifest_hashing() {
    let env = TestEnv::new();
    let bashrc = env.root().join("cozydot/dotfiles/bash/.bashrc");

    env.cozydot().arg("init").assert().success();
    fs::write(&bashrc, "alias g=git\n").unwrap();

    // simulated upgrade run skips a whole package once any manifest hash mismatches
    env.cozydot().arg("init").assert().success();

    assert_eq!(fs::read_to_string(&bashrc).unwrap(), "alias g=git\n");
}

// stow conflict aborts & leaves ~/.bashrc untouched; --replace backs it up first
#[test]
fn dotfiles_refuse_conflicts_without_replace_flag() {
    let env = TestEnv::new();

    // --version satisfies the CLI check, --simulate reports a conflict, installs succeed
    env.mock("stow", "#!/bin/sh\ncase \"$*\" in *--simulate*) exit 2 ;; esac\nexit 0\n");

    env.write_config(&MINIMAL_CONFIG.replace("linux: []", "linux:\n      - bash"));
    fs::create_dir_all(env.root().join("cozydot/dotfiles/bash")).unwrap();
    fs::write(env.root().join("cozydot/dotfiles/bash/.bashrc"), "cozydot bashrc\n").unwrap();
    fs::write(env.home().join(".bashrc"), "user bashrc\n").unwrap();

    env.cozydot().arg("dotfiles").assert().failure().stderr(predicate::str::contains("stow"));
    assert_eq!(fs::read_to_string(env.home().join(".bashrc")).unwrap(), "user bashrc\n");

    // --replace backs up the conflicting file before Stow claims the target
    env.cozydot().args(["dotfiles", "--replace"]).assert().success();
    assert!(!env.home().join(".bashrc").exists());
    let backups = env.state_home().join("cozydot/dotfile-backups");
    let backup = fs::read_dir(backups).unwrap().next().unwrap().unwrap().path().join("bash/.bashrc");
    assert_eq!(fs::read_to_string(backup).unwrap(), "user bashrc\n");
}

// a schema error anywhere in the config means 0 commands reach the host
#[test]
#[cfg(target_os = "linux")]
fn apply_aborts_prior_to_mutation_on_invalid_config() {
    let env = TestEnv::new();

    // record any mutation attempt so the test can prove nothing ran
    let mutations = env.root().join("mutations");
    env.mock("sudo", &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\n", mutations.display()));

    env.write_config(&format!("{MINIMAL_CONFIG}\nbogus_field: true\n"));

    env.cozydot().arg("apply").assert().failure().stderr(predicate::str::contains("bogus_field"));

    assert!(!mutations.exists(), "host was mutated despite an invalid config");
}

// macOS brew packages are ignored entirely when applying on a Linux host
#[test]
#[cfg(target_os = "linux")]
fn apply_respects_platform_target_boundaries() {
    let env = TestEnv::new();

    let brew_calls = env.root().join("brew-called");
    env.mock("sudo", "#!/bin/sh\nexit 0\n");
    env.mock("brew", &format!("#!/bin/sh\ntouch {}\nexit 1\n", brew_calls.display()));

    let config = MINIMAL_CONFIG.replace("formulae: []", "formulae:\n        - should-never-be-installed-on-linux");
    env.write_config(&config);

    env.cozydot().arg("apply").assert().success();

    assert!(!brew_calls.exists(), "macOS payload executed on a Linux host");
}

// prereqs install, then repo key, then source list, then apt update, then repo package
#[test]
#[cfg(target_os = "linux")]
fn apply_enforces_strict_dependency_ordering() {
    let env = TestEnv::new();
    let log = env.root().join("calls.log");

    env.mock("sudo", &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\n", log.display()));
    env.mock(
        "curl",
        "#!/bin/sh\nnext=0\nfor arg do\n  if [ \"$next\" = 1 ]; then printf test-key > \"$arg\"; exit 0; fi\n  [ \"$arg\" = --output ] && next=1\ndone\nexit 1\n",
    );
    // dearmor writes a fake keyring; list-keys must report a public key or validation fails
    env.mock(
        "gpg",
        "#!/bin/sh\nnext=0\nfor arg do\n  if [ \"$next\" = 1 ]; then printf test-key > \"$arg\"; fi\n  [ \"$arg\" = --output ] && next=1\ndone\ncase \"$*\" in *--list-keys*) printf 'pub:test\\n' ;; esac\nexit 0\n",
    );

    let config = MINIMAL_CONFIG.replace(
        "packages:\n  linux: {}",
        "packages:\n  linux:\n    apt:\n      install:\n        - hello\n      repos:\n        - name: example\n          key_url: https://example.com/key.gpg\n          key_path: /etc/apt/keyrings/example.gpg\n          uris:\n            default: https://example.com/apt\n          suite: stable\n          components:\n            - main\n          packages:\n            - example-package",
    );
    env.write_config(&config);

    env.cozydot().env("COZYDOT_LOG", &log).arg("apply").assert().success();

    let calls = fs::read_to_string(&log).unwrap();
    let position = |needle: &str| {
        calls
            .lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no call matched {needle:?} in {calls}"))
    };

    let mut updates = Vec::new();
    for (index, line) in calls.lines().enumerate() {
        if line.contains("apt-get update") {
            updates.push(index);
        }
    }
    assert_eq!(updates.len(), 2, "expected metadata refresh before repos & again after: {calls}");

    // staged writes end in `mv -fT -- <staged> <destination>`; compare destinations directly
    let destination = |target: &str| {
        calls
            .lines()
            .position(|line| line.starts_with("mv -fT") && line.ends_with(target))
            .unwrap_or_else(|| panic!("no call moved a file to {target:?} in {calls}"))
    };
    let key_write = destination("/etc/apt/keyrings/example.gpg");
    let source_write = destination("/etc/apt/sources.list.d/example.list");

    assert!(key_write < source_write, "repo key must be installed before its source list: {calls}");
    assert!(position("apt-get install") < key_write, "prerequisites come before repositories: {calls}");
    assert!(source_write < updates[1], "apt update runs after repositories are added: {calls}");
    assert!(updates[1] < position("example-package+"), "repo packages install last: {calls}");
}

// unsupported uname exits before curl ever runs
#[test]
fn installer_rejects_unsupported_platforms_before_download() {
    let env = TestEnv::new();

    let downloads = env.root().join("downloaded");
    env.mock("uname", "#!/bin/sh\ncase \"$1\" in -s) echo FreeBSD ;; -m) echo amd64 ;; esac\n");
    env.mock("curl", &format!("#!/bin/sh\ntouch {}\nexit 0\n", downloads.display()));

    let output = StdCommand::new("bash")
        .arg(env!("CARGO_MANIFEST_DIR").to_owned() + "/install.sh")
        .args(["-v", "1"])
        .env("PATH", env.mocked_path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unsupported platform"), "unexpected stderr: {stderr}");
    assert!(!downloads.exists(), "installer reached the network despite an unsupported platform");
}

// a bad checksum aborts the install, cleans temp files & leaves the old binary intact
#[test]
fn installer_checksum_failure_preserves_existing_binary() {
    let env = TestEnv::new();
    let downloads = env.root().join("downloads");
    fs::create_dir_all(&downloads).unwrap();
    let installed = env.home().join(".local/bin/cozydot");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::write(&installed, "existing binary\n").unwrap();

    env.mock("uname", "#!/bin/sh\ncase \"$1\" in -s) echo Linux ;; -m) echo x86_64 ;; esac\n");
    // serve fake archive & checksum bytes to whichever -o destination the installer picks
    env.mock(
        "curl",
        "#!/bin/sh\nnext=0\nfor arg do\n  if [ \"$next\" = 1 ]; then printf fake-archive > \"$arg\"; exit 0; fi\n  [ \"$arg\" = -o ] && next=1\ndone\nexit 1\n",
    );
    env.mock("sha256sum", "#!/bin/sh\nexit 1\n");

    let output = StdCommand::new("bash")
        .arg(env!("CARGO_MANIFEST_DIR").to_owned() + "/install.sh")
        .args(["-v", "1"])
        .env("PATH", env.mocked_path())
        .env("HOME", env.home())
        .env("TMPDIR", &downloads)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("checksum verification failed"), "unexpected stderr: {stderr}");

    assert_eq!(fs::read_to_string(&installed).unwrap(), "existing binary\n");
    assert!(fs::read_dir(&downloads).unwrap().count() == 0, "installer left temp files behind in {:?}", downloads);
}
