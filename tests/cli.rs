use assert_cmd::Command;
use predicates::prelude::*;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn write_executable(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

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
      source:
        provider: github
        repository: example/unsupported
        assets:
          riscv64: ^unsupported$
integrations:
  appimaged: true
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
  appimaged: false
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
fn true_updates_require_nonempty_targets_and_domain_values_stay_valid() {
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
        (
            "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: appimage\n      source:\n        provider: github\n        repository: example/app\n        assets:\n          amd64: ^app\\.AppImage$\n",
            "AppImages require integrations.appimaged: true",
        ),
        (
            "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: appimage\n      commands: [app]\n      source:\n        provider: github\n        repository: example/app\n        assets:\n          amd64: ^app\\.AppImage$\nintegrations:\n  appimaged: true\n",
            "commands: not supported for AppImages managed by appimaged",
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

#[test]
fn toolchains_delegate_convergence_to_native_managers() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let log = temp.path().join("manager.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        "version: 1.0.0\ntools:\n  rust: stable\n  node: latest\n  python: \"3.13\"\nupdates:\n  tools:\n    rust: true\n    node: true\n",
    )
    .unwrap();

    let recorder = "#!/bin/sh\nprintf '%s %s\\n' \"${0##*/}\" \"$*\" >> \"$COZYDOT_TEST_LOG\"\n";
    write_executable(&home.join(".cargo/bin/rustup"), recorder);
    write_executable(&home.join(".local/share/fnm/fnm"), recorder);
    write_executable(&home.join(".local/bin/uv"), recorder);
    let fake_bin = temp.path().join("bin");
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nexit 0\n");

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("CARGO_HOME", home.join(".cargo"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("UV_INSTALL_DIR", home.join(".local/bin"))
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success()
        .stdout(concat!(
            "Applying APT bootstrap packages\n",
            "Applying Rustup bootstrap\n",
            "Applying FNM bootstrap\n",
            "Applying uv bootstrap\n",
            "Applying Rust toolchain\n",
            "Applying Node.js toolchain\n",
            "Applying Python toolchain\n",
        ));

    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        concat!(
            "rustup toolchain install --profile minimal --no-self-update -- stable\n",
            "rustup default -- stable\n",
            "fnm install --progress never -- latest\n",
            "fnm default -- latest\n",
            "uv python install --no-config --managed-python --no-progress --default -- 3.13\n",
        )
    );

    fs::write(config_dir.join("cozydot.yaml"), "version: 1.0.0\ntools:\n  node: lts\n").unwrap();
    fs::write(&log, "").unwrap();
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&log).unwrap(), "fnm install --progress never --lts\nfnm default -- lts-latest\n");

    fs::write(
        config_dir.join("cozydot.yaml"),
        "version: 1.0.0\ntools:\n  node: \"20\"\nupdates:\n  tools:\n    node: true\n",
    )
    .unwrap();
    fs::write(&log, "").unwrap();
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&log).unwrap(), "fnm install --progress never -- 20\nfnm default -- 20\n");
}

#[test]
fn appimaged_is_ensured_once_before_appimages_are_published() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let systemctl_log = temp.path().join("systemctl.log");
    let systemctl_count = temp.path().join("systemctl.count");
    let sudo_log = temp.path().join("sudo.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        r#"version: 1.0.0
packages:
  binaries:
    - name: obsidian
      format: appimage
      source:
        provider: github
        repository: example/obsidian
        assets:
          amd64: ^Obsidian\.AppImage$
          arm64: ^Obsidian\.AppImage$
          arm32: ^Obsidian\.AppImage$
    - name: zen-browser
      format: appimage
      source:
        provider: github
        repository: example/zen
        assets:
          amd64: ^Zen\.AppImage$
          arm64: ^Zen\.AppImage$
          arm32: ^Zen\.AppImage$
    - name: fixed
      format: appimage
      source:
        provider: url
        urls:
          amd64: https://example.com/fixed.AppImage
          arm64: https://example.com/fixed.AppImage
          arm32: https://example.com/fixed.AppImage
        sha256:
          amd64: 13ecaa8c8c200cefa4b472684d16c033d6d4b0e45be25cf6402ecfb19b9bf6cb
          arm64: 13ecaa8c8c200cefa4b472684d16c033d6d4b0e45be25cf6402ecfb19b9bf6cb
          arm32: 13ecaa8c8c200cefa4b472684d16c033d6d4b0e45be25cf6402ecfb19b9bf6cb
integrations:
  appimaged: true
"#,
    )
    .unwrap();

    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$COZYDOT_SUDO_LOG\"\n");
    write_executable(&fake_bin.join("apt-cache"), "#!/bin/sh\nexit 0\n");
    write_executable(&fake_bin.join("dpkg-query"), "#!/bin/sh\nprintf 'rc '\n");
    write_executable(
        &fake_bin.join("dpkg"),
        "#!/bin/sh\ncase \"$*\" in *appimagelauncher*) exit 1;; *) exit 0;; esac\n",
    );
    write_executable(
        &fake_bin.join("systemctl"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$COZYDOT_SYSTEMCTL_LOG"
case "$*" in
  *is-active*)
    count=0
    [ ! -f "$COZYDOT_SYSTEMCTL_COUNT" ] || count=$(cat "$COZYDOT_SYSTEMCTL_COUNT")
    count=$((count + 1))
    printf '%s\n' "$count" > "$COZYDOT_SYSTEMCTL_COUNT"
    [ "$count" -gt 2 ]
    ;;
  *) exit 0 ;;
esac
"#,
    );
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/bin/sh
args="$*"
output=
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    shift
    output="$1"
  fi
  shift
done
if [ -n "$output" ]; then
  case "$args" in
    *fixed.AppImage*) printf '\177ELFcozydot-test-appimage\n' > "$output" ;;
    *) cp /bin/true "$output" ;;
  esac
  exit 0
fi
case "$args" in
  *go-appimage/releases/tags/continuous*)
    printf '%s' '{"assets":[{"name":"appimaged-1-x86_64.AppImage","browser_download_url":"https://example.com/appimaged-amd64.AppImage"},{"name":"appimaged-1-aarch64.AppImage","browser_download_url":"https://example.com/appimaged-arm64.AppImage"},{"name":"appimaged-1-armhf.AppImage","browser_download_url":"https://example.com/appimaged-arm32.AppImage"}]}'
    ;;
  *example/obsidian/releases/latest*)
    printf '%s' '{"draft":false,"prerelease":false,"tag_name":"1","assets":[{"name":"Obsidian.AppImage","browser_download_url":"https://example.com/Obsidian.AppImage","digest":null}]}'
    ;;
  *example/zen/releases/latest*)
    printf '%s' '{"draft":false,"prerelease":false,"tag_name":"1","assets":[{"name":"Zen.AppImage","browser_download_url":"https://example.com/Zen.AppImage","digest":null}]}'
    ;;
  *) exit 1 ;;
esac
"#,
    );

    fs::create_dir_all(home.join("Applications")).unwrap();
    fs::copy("/bin/true", home.join("Applications/fixed.AppImage")).unwrap();
    let user_cache_directory = home.join(".local/share/applications/appimage-backup");
    fs::create_dir_all(&user_cache_directory).unwrap();

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_SYSTEMCTL_LOG", &systemctl_log)
        .env("COZYDOT_SYSTEMCTL_COUNT", &systemctl_count)
        .env("COZYDOT_SUDO_LOG", &sudo_log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success()
        .stdout(concat!(
            "Applying APT bootstrap packages\n",
            "Applying appimaged\n",
            "Applying binary package\n",
            "Applying binary package\n",
            "Applying binary package\n",
        ));

    for name in ["obsidian.AppImage", "zen-browser.AppImage", "fixed.AppImage"] {
        let appimage = home.join("Applications").join(name);
        assert!(fs::metadata(appimage).unwrap().permissions().mode() & 0o111 != 0);
    }
    assert_eq!(fs::read(home.join("Applications/fixed.AppImage")).unwrap(), b"\x7fELFcozydot-test-appimage\n");
    assert!(user_cache_directory.is_dir());
    assert!(fs::read_to_string(&sudo_log).unwrap().contains("apt-get install -y -qq -- libfuse2t64"));
    assert!(!home.join(".local/bin").exists());
    let systemctl_calls = fs::read_to_string(&systemctl_log).unwrap();
    assert_eq!(systemctl_calls.matches("is-active").count(), 3);

    write_executable(&fake_bin.join("curl"), "#!/bin/sh\nexit 1\n");
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_SYSTEMCTL_LOG", &systemctl_log)
        .env("COZYDOT_SYSTEMCTL_COUNT", &systemctl_count)
        .env("COZYDOT_SUDO_LOG", &sudo_log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();
    let systemctl_calls = fs::read_to_string(systemctl_log).unwrap();
    assert_eq!(systemctl_calls.matches("is-active").count(), 4);
}
