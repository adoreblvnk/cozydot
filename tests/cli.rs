use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn config(shared: &str, linux: &str) -> String {
    let mut value = json!({
        "version": "1.0.0",
        "shared": {
            "tools": {},
            "packages": {},
            "fonts": {},
            "dotfiles": {"packages": []},
            "integrations": {"vscode": {"extensions": []}},
            "updates": {"tools": {}, "packages": {}, "fonts": null}
        },
        "os": {
            "linux": {
                "system": {},
                "packages": {},
                "dotfiles": {"packages": []},
                "integrations": {},
                "desktop": null,
                "updates": null
            },
            "macos": {
                "system": {"xcode": {}},
                "homebrew": {"formulae": [], "casks": []},
                "dotfiles": {"packages": []},
                "desktop": {},
                "updates": {"homebrew": {}}
            }
        }
    });
    let shared: Value = yaml_serde::from_str(shared).unwrap();
    let linux: Value = yaml_serde::from_str(linux).unwrap();
    value["shared"].as_object_mut().unwrap().extend(shared.as_object().unwrap().clone());
    value["os"]["linux"].as_object_mut().unwrap().extend(linux.as_object().unwrap().clone());
    serde_json::to_string(&value).unwrap()
}

fn config_root(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let root = temp.path().join("config/cozydot");
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_config(root: &Path, shared: &str, linux: &str) {
    fs::write(root.join("cozydot.yaml"), config(shared, linux)).unwrap();
}

fn write_executable(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn os_release_value(key: &str) -> String {
    fs::read_to_string("/etc/os-release")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap_or_default()
        .trim_matches('"')
        .to_owned()
}

#[test]
fn cli_contracts() {
    for args in [Vec::<&str>::new(), vec!["--help"]] {
        Command::cargo_bin("cozydot").unwrap().args(args).assert().success().stdout(
            predicate::str::contains("init")
                .and(predicate::str::contains("apply"))
                .and(predicate::str::contains("check"))
                .and(predicate::str::contains("dotfiles"))
                .and(predicate::str::contains("update")),
        );
    }
    Command::cargo_bin("cozydot")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));

    let temp = tempfile::tempdir().unwrap();
    for command in ["apply", "check", "dotfiles", "update"] {
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .arg(command)
            .assert()
            .failure()
            .stderr(predicate::str::contains("active configuration is missing or invalid"));
    }
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path())
        .args(["init", "--preset", "unknown"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'unknown'"));
}

#[test]
fn installer_rejects_unsupported_platform_before_download() {
    let temp = tempfile::tempdir().unwrap();
    let fake_bin = temp.path().join("bin");
    let download = temp.path().join("download");
    write_executable(
        &fake_bin.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -s) printf 'Darwin\\n' ;; -m) printf 'x86_64\\n' ;; esac\n",
    );
    write_executable(&fake_bin.join("curl"), "#!/bin/sh\n: > \"$COZYDOT_TEST_DOWNLOAD\"\nexit 99\n");

    Command::new("bash")
        .arg("install.sh")
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .env("COZYDOT_TEST_DOWNLOAD", &download)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cozydot: unsupported platform"));
    assert!(!download.exists());
}

#[test]
fn init_materializes_presets_and_preserves_user_edits() {
    for preset in ["cozydot", "full", "cli", "vm"] {
        let temp = tempfile::tempdir().unwrap();
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .args(["init", "--preset", preset])
            .assert()
            .success();
        let root = temp.path().join("cozydot");
        assert_eq!(fs::read(root.join("cozydot.yaml")).unwrap(), fs::read(format!("configs/{preset}.yaml")).unwrap());
        assert!(root.join(".managed-files").is_file());
        assert!(root.join("dotfiles/bash/.bashrc").is_file());
    }

    let temp = tempfile::tempdir().unwrap();
    Command::cargo_bin("cozydot").unwrap().env("XDG_CONFIG_HOME", temp.path()).arg("init").assert().success();
    let active = temp.path().join("cozydot/cozydot.yaml");
    fs::write(&active, "user edit\n").unwrap();
    Command::cargo_bin("cozydot").unwrap().env("XDG_CONFIG_HOME", temp.path()).arg("init").assert().success();
    assert_eq!(fs::read_to_string(active).unwrap(), "user edit\n");
}

#[test]
fn validation_happens_before_platform_detection_or_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_root(&temp);
    let fake_bin = temp.path().join("bin");
    let probe = temp.path().join("platform-probe");
    fs::write(root.join("cozydot.yaml"), "version: [\n").unwrap();
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\n: > \"$COZYDOT_TEST_PROBE\"\nprintf 'x86_64\\n'\n");

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("COZYDOT_TEST_PROBE", &probe)
        .env("PATH", &fake_bin)
        .arg("apply")
        .assert()
        .failure();
    assert!(!probe.exists());

    let repository = |extra: &str| {
        format!(
            "packages:\n  apt:\n    repositories:\n      - name: vendor\n        key: https://example.com/key\n        key_path: /etc/apt/keyrings/vendor.gpg\n        urls: {{default: https://example.com/repo}}\n        suite: stable\n        components: [main]\n{extra}"
        )
    };
    for (linux, error) in [
        (repository("        path: /\n"), "unknown field `path`"),
        (repository("        arch: [arm32]\n"), "unknown variant `arm32`"),
        (repository("        arch: []\n"), "arch: must not be empty"),
        (repository("").replace("/etc/apt/keyrings/vendor.gpg", "/tmp/vendor.gpg"), "direct child"),
        (
            "packages:\n  apt:\n    repositories:\n      - name: vendor\n        key: key\n        key_path: /etc/apt/keyrings/vendor.gpg\n        urls: {default: source}\n        components: [main]\n".to_owned(),
            "missing field `suite`",
        ),
        (
            "packages:\n  apt:\n    repositories:\n      - name: vendor\n        key: key\n        key_path: /etc/apt/keyrings/vendor.gpg\n        urls: {default: source}\n        suite: stable\n".to_owned(),
            "missing field `components`",
        ),
    ] {
        write_config(&root, "{}", &linux);
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .arg("check")
            .assert()
            .failure()
            .stderr(predicate::str::contains(error));
    }
}

#[test]
fn empty_apply_and_update_are_silent_noops() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_root(&temp);
    let fake_bin = temp.path().join("bin");
    let mutation = temp.path().join("mutation");
    write_config(&root, "{}", "{}");
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    for command in ["sudo", "curl", "gpg", "stow", "flatpak", "rustup"] {
        write_executable(&fake_bin.join(command), "#!/bin/sh\n: > \"$COZYDOT_TEST_MUTATION\"\nexit 99\n");
    }

    for command in ["apply", "update"] {
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .env("COZYDOT_TEST_MUTATION", &mutation)
            .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
            .arg(command)
            .assert()
            .success()
            .stdout(predicate::str::is_empty());
    }
    assert!(!mutation.exists());
}

#[test]
fn dotfiles_refuse_conflicts_and_replace_only_when_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = config_root(&temp);
    let fake_bin = temp.path().join("bin");
    let source = root.join("dotfiles/bash/.bashrc");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::write(&source, "managed\n").unwrap();
    fs::write(home.join(".bashrc"), "existing\n").unwrap();
    write_config(&root, "dotfiles:\n  packages: [bash]\n", "{}");
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    write_executable(
        &fake_bin.join("stow"),
        r#"#!/bin/sh
[ "${1-}" = "--version" ] && exit 0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dir) dir=$2; shift 2 ;;
    --target) target=$2; shift 2 ;;
    --stow) shift ;;
    --) package=$2; break ;;
  esac
done
ln -s "$dir/$package/.bashrc" "$target/.bashrc"
"#,
    );

    let command = || {
        let mut command = Command::cargo_bin("cozydot").unwrap();
        command
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()));
        command
    };
    command()
        .arg("dotfiles")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unmanaged dotfile conflicts").and(predicate::str::contains("--replace")));
    assert_eq!(fs::read_to_string(home.join(".bashrc")).unwrap(), "existing\n");
    assert!(!state.exists());

    command().args(["dotfiles", "--replace"]).assert().success().stdout("Applying dotfiles\n");
    assert_eq!(fs::canonicalize(home.join(".bashrc")).unwrap(), fs::canonicalize(source).unwrap());
    let backups = state.join("cozydot/dotfile-backups");
    let backup = fs::read_dir(backups).unwrap().next().unwrap().unwrap().path().join("bash/.bashrc");
    assert_eq!(fs::read_to_string(backup).unwrap(), "existing\n");
}

fn repository_config() -> String {
    config(
        "{}",
        r#"packages:
  apt:
    install: [direct-package]
    repositories:
      - name: armored
        key: https://example.com/armored
        key_path: /etc/apt/keyrings/armored.asc
        urls: {default: https://example.com/armored}
        suite: stable
        components: [main]
        conflicts: [old-package, absent-conflict]
        packages: [vendor-one]
      - name: binary
        key: https://example.com/binary
        key_path: /usr/share/keyrings/binary.gpg
        urls: {default: https://example.com/binary}
        suite: vendor-suite
        components: [vendor-component]
        packages: [vendor-two]
"#,
    )
}

fn write_apt_fakes(fake_bin: &Path) {
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    write_executable(
        &fake_bin.join("dpkg-query"),
        r#"#!/bin/sh
last=
for argument in "$@"; do last=$argument; done
printf 'dpkg-query %s\n' "$*" >> "$COZYDOT_TEST_LOG"
if [ -f "$COZYDOT_TEST_STATE/packages/$last" ]; then printf 'installed\n'; else printf 'not-installed\n'; fi
"#,
    );
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/bin/sh
printf 'curl %s\n' "$*" >> "$COZYDOT_TEST_LOG"
while [ "$#" -gt 0 ]; do
  [ "$1" != "--output" ] || { shift; output=$1; }
  shift
done
printf 'key' > "$output"
"#,
    );
    write_executable(
        &fake_bin.join("gpg"),
        r#"#!/bin/sh
printf 'gpg %s\n' "$*" >> "$COZYDOT_TEST_LOG"
case " $* " in
  *" --list-keys "*) [ -z "${COZYDOT_TEST_NO_PUBLIC-}" ] && printf 'pub:x\n'; exit 0 ;;
esac
while [ "$#" -gt 0 ]; do
  [ "$1" != "--output" ] || { shift; output=$1; }
  shift
done
printf 'processed-key' > "$output"
"#,
    );
    write_executable(
        &fake_bin.join("sudo"),
        r#"#!/bin/sh
printf 'sudo %s\n' "$*" >> "$COZYDOT_TEST_LOG"
case " $* " in
  *" apt-get install "*)
    after=false
    for argument in "$@"; do
      $after && touch "$COZYDOT_TEST_STATE/packages/${argument%+}"
      [ "$argument" != "--" ] || after=true
    done
    exit 0 ;;
  *" apt-get purge "*)
    after=false
    for argument in "$@"; do
      $after && rm -f "$COZYDOT_TEST_STATE/packages/$argument"
      [ "$argument" != "--" ] || after=true
    done
    exit 0 ;;
esac
previous=
last=
for argument in "$@"; do previous=$last; last=$argument; done
name=$(basename "$last")
case "$1" in
  install)
    [ "$2" != "-d" ] || exit 0
    cp "$previous" "$COZYDOT_TEST_STATE/files/$name" ;;
  mv) mv "$COZYDOT_TEST_STATE/files/$(basename "$previous")" "$COZYDOT_TEST_STATE/files/$name" ;;
  test)
    case "$*" in
      *" ! -L "*|*" ! -d "*) exit 0 ;;
      *" -L "*) exit 1 ;;
      *" -f "*) [ -f "$COZYDOT_TEST_STATE/files/$name" ]; exit ;;
      *" ! -e "*) [ ! -f "$COZYDOT_TEST_STATE/files/$name" ]; exit ;;
    esac ;;
  cat) cat "$COZYDOT_TEST_STATE/files/$name" ;;
  stat) exit 99 ;;
  *) exit 0 ;;
esac
"#,
    );
}

fn run_apt(
    config_home: &Path,
    fake_bin: &Path,
    state: &Path,
    log: &Path,
    extra_env: Option<(&str, &str)>,
) -> std::process::Output {
    fs::write(log, "").unwrap();
    let mut command = Command::cargo_bin("cozydot").unwrap();
    command
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", log)
        .env("COZYDOT_TEST_STATE", state)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply");
    if let Some((key, value)) = extra_env {
        command.env(key, value);
    }
    command.output().unwrap()
}

#[test]
fn repository_key_validation_precedes_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_root(&temp);
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    let log = temp.path().join("apt.log");
    fs::create_dir_all(state.join("files")).unwrap();
    fs::create_dir_all(state.join("packages")).unwrap();
    for package in ["ca-certificates", "curl", "gnupg"] {
        fs::write(state.join("packages").join(package), "").unwrap();
    }
    write_config(
        &root,
        "{}",
        "packages:\n  apt:\n    repositories:\n      - name: vendor\n        key: https://example.com/key\n        key_path: /etc/apt/keyrings/vendor.gpg\n        urls: {default: https://example.com/repo}\n        suite: stable\n        components: [main]\n",
    );
    write_apt_fakes(&fake_bin);

    let output = run_apt(&temp.path().join("config"), &fake_bin, &state, &log, Some(("COZYDOT_TEST_NO_PUBLIC", "1")));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("repository key validation found no public key"));
    assert!(!state.join("files/vendor.gpg").exists());
    assert!(!state.join("files/vendor.list").exists());
}

#[test]
fn repository_apply_is_ordered_aggregated_and_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_root(&temp);
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    let log = temp.path().join("apt.log");
    fs::create_dir_all(state.join("files")).unwrap();
    fs::create_dir_all(state.join("packages")).unwrap();
    fs::write(root.join("cozydot.yaml"), repository_config()).unwrap();
    for package in ["ca-certificates", "curl", "gnupg", "old-package"] {
        fs::write(state.join("packages").join(package), "").unwrap();
    }
    write_apt_fakes(&fake_bin);

    let output = run_apt(&temp.path().join("config"), &fake_bin, &state, &log, None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let first = fs::read_to_string(&log).unwrap();
    let lines = first.lines().collect::<Vec<_>>();
    let position = |needle: &str| lines.iter().position(|line| line.contains(needle)).unwrap();
    let direct_install = position("apt-get install -y -qq -- direct-package+");
    let repository_download = position("curl ");
    let publication = position("/etc/apt/sources.list.d/armored.list");
    let repository_refresh = lines
        .iter()
        .enumerate()
        .find(|(index, line)| *index > publication && **line == "sudo apt-get update -qq")
        .map(|(index, _)| index)
        .unwrap();
    let purge = position("apt-get purge -y -qq -- old-package");
    let install = position("apt-get install -y -qq -- vendor-one+ vendor-two+");
    assert!(direct_install < repository_download);
    assert!(repository_download < publication && publication < repository_refresh);
    assert!(repository_refresh < purge && purge < install);
    assert!(!lines[purge].contains("absent-conflict"));
    assert_eq!(fs::read(state.join("files/armored.asc")).unwrap(), b"key");
    assert_eq!(fs::read(state.join("files/binary.gpg")).unwrap(), b"processed-key");
    assert_eq!(
        fs::read_to_string(state.join("files/armored.list")).unwrap(),
        "deb [arch=amd64 signed-by=/etc/apt/keyrings/armored.asc] https://example.com/armored stable main\n"
    );
    assert_eq!(
        fs::read_to_string(state.join("files/binary.list")).unwrap(),
        "deb [arch=amd64 signed-by=/usr/share/keyrings/binary.gpg] https://example.com/binary vendor-suite vendor-component\n"
    );

    let output = run_apt(&temp.path().join("config"), &fake_bin, &state, &log, None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let second = fs::read_to_string(log).unwrap();
    assert_eq!(second.matches("sudo apt-get update -qq\n").count(), 2);
    assert!(!second.contains(" apt-get install "));
    assert!(!second.contains(" apt-get purge "));
}

#[test]
fn inapplicable_repositories_have_no_side_effects() {
    let inapplicable_distro = if os_release_value("ID") == "linuxmint" { "pop" } else { "linuxmint" };
    for applicability in [
        "          default: https://example.com/repo\n        arch: [arm64]".to_owned(),
        format!("          {inapplicable_distro}: https://example.com/repo"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = config_root(&temp);
        let fake_bin = temp.path().join("bin");
        let mutation = temp.path().join("mutation");
        write_config(
            &root,
            "{}",
            &format!(
                "packages:\n  apt:\n    repositories:\n      - name: skipped\n        key: https://example.com/key\n        key_path: /etc/apt/keyrings/skipped.gpg\n        urls:\n{applicability}\n        suite: stable\n        components: [main]\n        conflicts: [old]\n        packages: [new]\n"
            ),
        );
        write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
        for command in ["curl", "gpg", "sudo", "dpkg-query"] {
            write_executable(&fake_bin.join(command), "#!/bin/sh\n: > \"$COZYDOT_TEST_MUTATION\"\nexit 99\n");
        }
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .env("COZYDOT_TEST_MUTATION", &mutation)
            .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
            .arg("apply")
            .assert()
            .success()
            .stdout(predicate::str::is_empty());
        assert!(!mutation.exists());
    }
}

#[test]
fn apt_update_runs_only_the_selected_policy() {
    for (policy, expected) in [
        ("standard", "sudo apt-get update -qq\nsudo DEBIAN_FRONTEND=noninteractive apt-get upgrade -y -qq --\n"),
        (
            "full",
            concat!(
                "sudo apt-get update -qq\n",
                "sudo DEBIAN_FRONTEND=noninteractive apt-get full-upgrade -y -qq --\n",
                "sudo DEBIAN_FRONTEND=noninteractive apt-get autoremove --purge -y -qq --\n"
            ),
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = config_root(&temp);
        let fake_bin = temp.path().join("bin");
        let log = temp.path().join("update.log");
        write_config(&root, "{}", &format!("updates:\n  apt: {policy}\n"));
        write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
        write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf 'sudo %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");

        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .env("COZYDOT_TEST_LOG", &log)
            .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
            .arg("update")
            .assert()
            .success();
        assert_eq!(fs::read_to_string(log).unwrap(), expected);
    }
}
