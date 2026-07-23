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

fn run_npm_apply(query_success: bool, package: &str) -> (bool, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let data_home = home.join(".local/share");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("npm.log");
    fs::create_dir_all(config_home.join("cozydot")).unwrap();
    fs::write(
        config_home.join("cozydot/cozydot.yaml"),
        format!("version: 1.0.0\npackages:\n  npm: [\"{package}\"]\ntools:\n  node: latest\n"),
    )
    .unwrap();
    write_executable(
        &data_home.join("fnm/fnm"),
        &format!(
            r#"#!/bin/sh
printf 'fnm %s\n' "$*" >> "$COZYDOT_TEST_LOG"
if [ "$1" = "exec" ] && [ "$5" = "list" ]; then
  exit {}
fi
"#,
            if query_success { 0 } else { 1 },
        ),
    );
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    write_executable(&fake_bin.join("dpkg-query"), "#!/bin/sh\nprintf 'installed\\n'\n");
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nexit 99\n");
    write_executable(
        &fake_bin.join("npm"),
        "#!/bin/sh\nprintf 'ambient npm %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\nexit 98\n",
    );

    let output = Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .output()
        .unwrap();
    (output.status.success(), fs::read_to_string(log).unwrap_or_default(), String::from_utf8(output.stderr).unwrap())
}

fn os_release_value(key: &str) -> String {
    fs::read_to_string("/etc/os-release")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap()
        .trim_matches('"')
        .to_owned()
}

#[test]
fn help_and_version() {
    for args in [Vec::<&str>::new(), vec!["--help"]] {
        Command::cargo_bin("cozydot").unwrap().args(args).assert().success().stdout(
            predicate::str::contains("init")
                .and(predicate::str::contains("apply"))
                .and(predicate::str::contains("update")),
        );
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

    for command in ["apply", "update"] {
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .arg(command)
            .assert()
            .success()
            .stdout(predicate::str::is_empty());
    }
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
fn unsupported_distros_are_rejected() {
    for distro in ["zorin", "deepin", "kali", "tails"] {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("cozydot");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("cozydot.yaml"),
            format!("version: \"1.0.0\"\nsystem:\n  require:\n    distros: [{distro}]\n"),
        )
        .unwrap();

        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .arg("apply")
            .assert()
            .failure()
            .stderr(predicate::str::contains("unknown variant").and(predicate::str::contains(distro)));
    }
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
    python: false
  packages:
    cargo: false
    npm: false
"#,
    )
    .unwrap();

    for command in ["apply", "update"] {
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .arg(command)
            .assert()
            .success()
            .stdout(predicate::str::is_empty());
    }
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
        ("version: 1.0.0\nupdates:\n  tools:\n    go: true\n", "updates.tools.go: requires a configured Go selector"),
        (
            "version: 1.0.0\nupdates:\n  tools:\n    python: true\n",
            "updates.tools.python: requires a configured Python selector",
        ),
        (
            "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: appimage\n      source:\n        provider: github\n        repository: example/app\n        assets:\n          amd64: ^app\\.AppImage$\n          arm64: ^app\\.AppImage$\n          arm32: ^app\\.AppImage$\n",
            "AppImages require integrations.appimaged: true",
        ),
        (
            "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: appimage\n      commands: [app]\n      source:\n        provider: github\n        repository: example/app\n        assets:\n          amd64: ^app\\.AppImage$\nintegrations:\n  appimaged: true\n",
            "unknown field `commands`",
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
fn apt_repository_conflict_configuration_is_strict() {
    let repository = |conflicts: &str, packages: &str| {
        format!(
            r#"version: 1.0.0
packages:
  apt:
    repositories:
      - name: vendor
        key: https://example.com/key.gpg
        key_path: /etc/apt/keyrings/vendor.gpg
        urls:
          default: https://example.com/repository
        suite: stable
        components: [main]
        conflicts: {conflicts}
        packages: {packages}
"#
        )
    };
    for (config, message) in [
        ("version: 1.0.0\npackages:\n  apt:\n    remove: [obsolete]\n".to_owned(), "unknown field `remove`"),
        (repository("{}", "[vendor-package]"), "conflicts: must be a non-empty mapping"),
        (repository("{default: []}", "[vendor-package]"), "conflicts.default: must be a non-empty sequence"),
        (
            repository("{default: [vendor-package]}", "[vendor-package]"),
            "package \"vendor-package\" is also an applicable repository target",
        ),
        (repository("{default: [distro-package]}", "[]"), "conflicts: requires non-empty repository packages"),
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
fn apt_repository_ownership_validation_is_platform_aware() {
    let unrelated = match os_release_value("ID").as_str() {
        "ubuntu" | "pop" => "debian",
        "debian" => "ubuntu",
        "linuxmint" => "pop",
        distro => panic!("unsupported test distro: {distro}"),
    };
    let repository = |urls: &str, conflicts: &str, packages: &str| {
        format!(
            r#"      - name: vendor
        key: https://example.com/key.gpg
        key_path: /etc/apt/keyrings/vendor.gpg
        urls: {urls}
        suite: stable
        components: [main]
        conflicts: {conflicts}
        packages: {packages}
"#
        )
    };
    let accepted = [
        format!(
            "version: 1.0.0\npackages:\n  apt:\n    install: [shared]\n    repositories:\n{}",
            repository("{default: https://example.com/repository}", &format!("{{{unrelated}: [shared]}}"), "[vendor]")
        ),
        format!(
            "version: 1.0.0\npackages:\n  apt:\n    install: [shared]\n    repositories:\n{}",
            repository(&format!("{{{unrelated}: https://example.com/repository}}"), "{default: [shared]}", "[vendor]")
        ),
    ];
    for config in accepted {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("cozydot");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("cozydot.yaml"), config).unwrap();
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .arg("update")
            .assert()
            .success()
            .stdout(predicate::str::is_empty());
    }

    let rejected = [
        format!(
            "version: 1.0.0\npackages:\n  apt:\n    install: [shared]\n    repositories:\n{}",
            repository("{default: https://example.com/repository}", "{default: [shared]}", "[vendor]")
        ),
        format!(
            "version: 1.0.0\npackages:\n  apt:\n    repositories:\n{}{}",
            repository("{default: https://example.com/one}", "{default: [old-one]}", "[shared]"),
            repository("{default: https://example.com/two}", "{default: [shared]}", "[replacement]")
                .replace("name: vendor", "name: vendor-two")
                .replace("vendor.gpg", "vendor-two.gpg")
        ),
    ];
    for config in rejected {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("cozydot");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("cozydot.yaml"), config).unwrap();
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("XDG_CONFIG_HOME", temp.path())
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .arg("update")
            .assert()
            .failure()
            .stderr(predicate::str::contains("package \"shared\""));
    }
}

#[test]
fn configured_urls_accept_http_credentials_and_fragments() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("url.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        r#"version: 1.0.0
packages:
  binaries:
    - name: url-probe
      format: deb
      source:
        provider: url
        urls:
          amd64: http://user:password@example.com/probe.deb#asset
          arm64: http://user:password@example.com/probe.deb#asset
          arm32: http://user:password@example.com/probe.deb#asset
"#,
    )
    .unwrap();
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$COZYDOT_TEST_LOG"
output=
while [ "$#" -gt 0 ]; do
  [ "$1" != "--output" ] || { shift; output="$1"; }
  shift
done
[ -z "$output" ] || printf 'deb' > "$output"
"#,
    );
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    let log = fs::read_to_string(log).unwrap();
    assert!(log.contains("http://user:password@example.com/probe.deb#asset"));
    assert!(!log.contains("--proto =https"));
}

#[test]
fn inapplicable_repository_skips_its_packages_and_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("side-effects.log");
    fs::create_dir_all(&config_dir).unwrap();
    let inapplicable_distro = if os_release_value("ID") == "linuxmint" { "pop" } else { "linuxmint" };
    fs::write(
        config_dir.join("cozydot.yaml"),
        format!(
            r#"version: 1.0.0
packages:
  apt:
    repositories:
      - name: inapplicable
        key: https://example.com/key.gpg
        key_path: /etc/apt/keyrings/inapplicable.gpg
        urls:
          {inapplicable_distro}: https://example.com/repository
        suite: "APT-owned suite/value"
        components: ["component/value", "component/value"]
        conflicts:
          default: [must-not-purge]
        packages: [must-not-install]
"#
        ),
    )
    .unwrap();
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    for command in ["curl", "gpg", "sudo", "dpkg-query"] {
        write_executable(
            &fake_bin.join(command),
            "#!/bin/sh\nprintf '%s %s\\n' \"$(basename \"$0\")\" \"$*\" >> \"$COZYDOT_TEST_LOG\"\nexit 1\n",
        );
    }

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    assert!(!log.exists(), "inapplicable repository unexpectedly executed a side effect");
}

#[test]
fn binaries_without_a_native_architecture_url_are_noops() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("side-effects.log");
    let selector = match std::env::consts::ARCH {
        "x86_64" => "arm64",
        "aarch64" | "arm" => "amd64",
        architecture => panic!("unsupported test architecture: {architecture}"),
    };
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        format!(
            "version: 1.0.0\npackages:\n  binaries:\n    - name: absent-native-deb\n      format: deb\n      source:\n        provider: url\n        urls:\n          {selector}: https://example.com/absent.deb\n    - name: absent-native-appimage\n      format: appimage\n      source:\n        provider: url\n        urls:\n          {selector}: https://example.com/absent.AppImage\n"
        ),
    )
    .unwrap();
    for command in ["curl", "sudo"] {
        write_executable(
            &fake_bin.join(command),
            "#!/bin/sh\nprintf '%s %s\\n' \"$(basename \"$0\")\" \"$*\" >> \"$COZYDOT_TEST_LOG\"\nexit 1\n",
        );
    }

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    assert!(!log.exists(), "architecture-inapplicable binary unexpectedly executed a side effect");
}

#[test]
fn repository_rendering_resolves_system_suite_and_passes_apt_values_through() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(&state).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        r#"version: 1.0.0
packages:
  apt:
    repositories:
      - name: system-suite
        key: https://example.com/key.gpg
        key_path: /etc/apt/keyrings/system-suite.gpg
        urls:
          default: https://example.com/system
        suite: system
        components: [main]
        packages: []
      - name: native-values
        key: https://example.com/key.gpg
        key_path: /etc/apt/keyrings/native-values.gpg
        urls:
          default: https://example.com/native
        suite: "APT-owned suite/value"
        components: ["component/value", "component/value"]
        packages: []
"#,
    )
    .unwrap();
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/bin/sh
output=
while [ "$#" -gt 0 ]; do
  [ "$1" != "--output" ] || { shift; output="$1"; }
  shift
done
printf 'key' > "$output"
"#,
    );
    write_executable(
        &fake_bin.join("gpg"),
        r#"#!/bin/sh
case " $* " in *" --list-keys "*) printf 'pub:x\n'; exit 0;; esac
output=
while [ "$#" -gt 0 ]; do
  [ "$1" != "--output" ] || { shift; output="$1"; }
  shift
done
printf 'processed-key' > "$output"
"#,
    );
    write_executable(
        &fake_bin.join("sudo"),
        r#"#!/bin/sh
last=
previous=
for argument in "$@"; do previous="$last"; last="$argument"; done
name=$(basename "$last")
case "$1" in
  install)
    [ "$2" != "-d" ] || exit 0
    cp "$previous" "$COZYDOT_TEST_STATE/$name"
    ;;
  mv)
    mv "$COZYDOT_TEST_STATE/$(basename "$previous")" "$COZYDOT_TEST_STATE/$name"
    ;;
  test)
    case "$*" in
      *" ! -L "*) exit 0;;
      *" ! -e "*) [ ! -f "$COZYDOT_TEST_STATE/$name" ]; exit;;
    esac
    ;;
  stat) printf '81a4:0:0\n' ;;
  cat) cat "$COZYDOT_TEST_STATE/$name" ;;
  *) exit 0 ;;
esac
"#,
    );

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_STATE", &state)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    let system = fs::read_to_string(state.join("system-suite.list")).unwrap();
    let codename = os_release_value("VERSION_CODENAME");
    assert!(system.ends_with(&format!(" {codename} main\n")), "unexpected system suite source: {system:?}");
    let native = fs::read_to_string(state.join("native-values.list")).unwrap();
    assert!(
        native.ends_with(" APT-owned suite/value component/value component/value\n"),
        "APT-owned values were not rendered unchanged: {native:?}"
    );
}

fn apt_repository_conflict_config(with_updates: bool) -> String {
    let unrelated_distro = match os_release_value("ID").as_str() {
        "ubuntu" | "pop" => "debian",
        "debian" => "ubuntu",
        "linuxmint" => "pop",
        distro => panic!("unsupported test distro: {distro}"),
    };
    format!(
        r#"version: 1.0.0
packages:
  apt:
    install: [direct-package]
    repositories:
      - name: vendor
        key: https://example.com/vendor.gpg
        key_path: /etc/apt/keyrings/vendor.gpg
        urls:
          default: https://example.com/vendor
        suite: stable
        components: [main]
        conflicts:
          default: [selected-conflict, absent-selected-conflict]
          {unrelated_distro}: [unrelated-conflict]
        packages: [vendor-package]
{}
"#,
        if with_updates { "updates:\n  apt: standard" } else { "" }
    )
}

fn write_apt_repository_fakes(fake_bin: &Path) {
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    write_executable(
        &fake_bin.join("dpkg-query"),
        r#"#!/bin/sh
last=
for argument in "$@"; do last="$argument"; done
printf 'dpkg-query %s\n' "$*" >> "$COZYDOT_TEST_LOG"
if [ -f "$COZYDOT_TEST_STATE/packages/$last" ]; then
  printf 'installed\n'
else
  printf 'not-installed\n'
fi
"#,
    );
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/bin/sh
printf 'curl %s\n' "$*" >> "$COZYDOT_TEST_LOG"
output=
while [ "$#" -gt 0 ]; do
  [ "$1" != "--output" ] || { shift; output="$1"; }
  shift
done
printf 'key' > "$output"
"#,
    );
    write_executable(
        &fake_bin.join("gpg"),
        r#"#!/bin/sh
printf 'gpg %s\n' "$*" >> "$COZYDOT_TEST_LOG"
case " $* " in *" --list-keys "*) printf 'pub:x\n'; exit 0;; esac
output=
while [ "$#" -gt 0 ]; do
  [ "$1" != "--output" ] || { shift; output="$1"; }
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
      if $after; then touch "$COZYDOT_TEST_STATE/packages/${argument%+}"; fi
      [ "$argument" != "--" ] || after=true
    done
    exit 0
    ;;
  *" apt-get purge "*)
    after=false
    for argument in "$@"; do
      if $after; then rm -f "$COZYDOT_TEST_STATE/packages/$argument"; fi
      [ "$argument" != "--" ] || after=true
    done
    exit 0
    ;;
esac
last=
previous=
for argument in "$@"; do previous="$last"; last="$argument"; done
name=$(basename "$last")
case "$1" in
  install)
    [ "$2" != "-d" ] || exit 0
    cp "$previous" "$COZYDOT_TEST_STATE/files/$name"
    ;;
  mv)
    mv "$COZYDOT_TEST_STATE/files/$(basename "$previous")" "$COZYDOT_TEST_STATE/files/$name"
    ;;
  test)
    case "$*" in
      *" ! -L "*|*" ! -d "*) exit 0;;
      *" ! -e "*) [ ! -f "$COZYDOT_TEST_STATE/files/$name" ]; exit;;
    esac
    ;;
  stat) printf '81a4:0:0\n' ;;
  cat) cat "$COZYDOT_TEST_STATE/files/$name" ;;
  *) exit 0 ;;
esac
"#,
    );
}

fn run_apt_command(command: &str, config_home: &Path, fake_bin: &Path, state: &Path, log: &Path) {
    fs::write(log, "").unwrap();
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", log)
        .env("COZYDOT_TEST_STATE", state)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg(command)
        .assert()
        .success();
}

#[test]
fn apt_apply_orders_repository_migration_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    let log = temp.path().join("apt.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(state.join("files")).unwrap();
    fs::create_dir_all(state.join("packages")).unwrap();
    fs::write(config_dir.join("cozydot.yaml"), apt_repository_conflict_config(false)).unwrap();
    for package in ["ca-certificates", "curl", "gnupg", "selected-conflict", "unrelated-conflict"] {
        fs::write(state.join("packages").join(package), "").unwrap();
    }
    write_apt_repository_fakes(&fake_bin);

    run_apt_command("apply", &config_home, &fake_bin, &state, &log);
    let first = fs::read_to_string(&log).unwrap();
    let lines = first.lines().collect::<Vec<_>>();
    let direct_refresh = lines.iter().position(|line| *line == "sudo apt-get update -qq").unwrap();
    let direct_install =
        lines.iter().position(|line| line.ends_with("apt-get install -y -qq -- direct-package+")).unwrap();
    let repository_download = lines.iter().position(|line| line.starts_with("curl ")).unwrap();
    let publication = lines
        .iter()
        .position(|line| line.contains("sudo mv -fT") && line.ends_with("/etc/apt/sources.list.d/vendor.list"))
        .unwrap();
    let repository_refresh = lines
        .iter()
        .enumerate()
        .find(|(index, line)| *index > publication && **line == "sudo apt-get update -qq")
        .map(|(index, _)| index)
        .unwrap();
    let purge = lines.iter().position(|line| line.ends_with("apt-get purge -y -qq -- selected-conflict")).unwrap();
    let vendor_install =
        lines.iter().position(|line| line.ends_with("apt-get install -y -qq -- vendor-package+")).unwrap();
    assert!(direct_refresh < direct_install && direct_install < repository_download);
    assert!(repository_download < publication && publication < repository_refresh);
    assert!(repository_refresh < purge && purge < vendor_install, "unexpected apply order: {first}");
    assert!(first.contains("dpkg-query -W -f=${db:Status-Status}\\n -- absent-selected-conflict"));
    assert!(!lines[purge].contains("absent-selected-conflict"), "apply purged an absent selected conflict: {first}");
    assert!(!first.contains("unrelated-conflict"), "apply inspected or purged an unselected distro conflict: {first}");

    run_apt_command("apply", &config_home, &fake_bin, &state, &log);
    let second = fs::read_to_string(&log).unwrap();
    assert_eq!(
        second.matches("sudo apt-get update -qq\n").count(),
        2,
        "second apply skipped required refreshes: {second}"
    );
    assert!(!second.contains(" apt-get install "), "second apply reinstalled a package: {second}");
    assert!(!second.contains(" apt-get purge "), "second apply repurged a conflict: {second}");
    assert!(second.contains("curl "), "second apply did not inspect/converge the repository: {second}");
    assert!(!second.contains("unrelated-conflict"), "second apply inspected an unselected distro conflict: {second}");
}

#[test]
fn apt_update_orders_repository_migration_before_broad_upgrade_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    let log = temp.path().join("apt.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(state.join("files")).unwrap();
    fs::create_dir_all(state.join("packages")).unwrap();
    fs::write(config_dir.join("cozydot.yaml"), apt_repository_conflict_config(true)).unwrap();
    for package in ["ca-certificates", "curl", "gnupg", "selected-conflict", "unrelated-conflict"] {
        fs::write(state.join("packages").join(package), "").unwrap();
    }
    write_apt_repository_fakes(&fake_bin);

    run_apt_command("update", &config_home, &fake_bin, &state, &log);
    let first = fs::read_to_string(&log).unwrap();
    let lines = first.lines().collect::<Vec<_>>();
    let publication = lines
        .iter()
        .position(|line| line.contains("sudo mv -fT") && line.ends_with("/etc/apt/sources.list.d/vendor.list"))
        .unwrap();
    let refresh = lines.iter().position(|line| *line == "sudo apt-get update -qq").unwrap();
    let direct_install =
        lines.iter().position(|line| line.ends_with("apt-get install -y -qq -- direct-package+")).unwrap();
    let purge = lines.iter().position(|line| line.ends_with("apt-get purge -y -qq -- selected-conflict")).unwrap();
    let vendor_install =
        lines.iter().position(|line| line.ends_with("apt-get install -y -qq -- vendor-package+")).unwrap();
    let upgrade = lines.iter().position(|line| line.ends_with("apt-get upgrade -y -qq --")).unwrap();
    assert!(publication < refresh && refresh < direct_install);
    assert!(
        direct_install < purge && purge < vendor_install && vendor_install < upgrade,
        "unexpected update order: {first}"
    );
    assert!(first.contains("dpkg-query -W -f=${db:Status-Status}\\n -- absent-selected-conflict"));
    assert!(!lines[purge].contains("absent-selected-conflict"), "update purged an absent selected conflict: {first}");
    assert!(!first.contains("unrelated-conflict"), "update inspected an unselected distro conflict: {first}");

    run_apt_command("update", &config_home, &fake_bin, &state, &log);
    let second = fs::read_to_string(&log).unwrap();
    assert!(!second.contains(" apt-get install "), "second update reinstalled a package: {second}");
    assert!(!second.contains(" apt-get purge "), "second update repurged a conflict: {second}");
    assert!(second.contains("sudo DEBIAN_FRONTEND=noninteractive apt-get upgrade -y -qq --"));
    assert!(second.contains("curl "), "second update did not inspect/converge the repository: {second}");
    assert!(!second.contains("unrelated-conflict"), "second update inspected an unselected distro conflict: {second}");
}

#[test]
fn apt_update_converges_applicable_repository_without_targets() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    let log = temp.path().join("apt.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(state.join("files")).unwrap();
    fs::create_dir_all(state.join("packages")).unwrap();
    for package in ["ca-certificates", "curl", "gnupg"] {
        fs::write(state.join("packages").join(package), "").unwrap();
    }
    fs::write(
        config_dir.join("cozydot.yaml"),
        r#"version: 1.0.0
packages:
  apt:
    repositories:
      - name: vendor
        key: https://example.com/vendor.gpg
        key_path: /etc/apt/keyrings/vendor.gpg
        urls:
          default: https://example.com/vendor
        suite: stable
        components: [main]
        packages: []
updates:
  apt: standard
"#,
    )
    .unwrap();
    write_apt_repository_fakes(&fake_bin);

    run_apt_command("update", &config_home, &fake_bin, &state, &log);
    let log = fs::read_to_string(log).unwrap();
    let publication = log.find("/etc/apt/sources.list.d/vendor.list").unwrap();
    let refresh = log.find("sudo apt-get update -qq").unwrap();
    let upgrade = log.find("sudo DEBIAN_FRONTEND=noninteractive apt-get upgrade -y -qq --").unwrap();
    assert!(publication < refresh && refresh < upgrade, "unexpected empty-repository update order: {log}");
}

#[test]
fn apt_inspection_operational_failure_does_not_guess_package_state() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("sudo.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("cozydot.yaml"), "version: 1.0.0\npackages:\n  apt:\n    install: [ripgrep]\n").unwrap();
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    write_executable(&fake_bin.join("dpkg-query"), "#!/bin/sh\nprintf 'database failure\\n' >&2\nexit 2\n");
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("APT package inspection failed for \"ripgrep\": database failure"));
    let log = fs::read_to_string(log).unwrap();
    assert!(log.contains("apt-get update -qq"));
    assert!(!log.contains("apt-get install"));
}

#[test]
fn npm_apply_uses_per_target_native_presence_checks() {
    let cases = [
        ("present scoped package", true, "@scope/tool@^1", "@scope/tool", false),
        ("missing scoped package", false, "@scope/tool@^1", "@scope/tool", true),
        ("present aliased package", true, "tool-alias@npm:tool@^1", "tool-alias", false),
        ("missing scoped alias", false, "@scope/tool-alias@npm:@scope/tool@^1", "@scope/tool-alias", true),
    ];

    for (label, query_success, package, identity, should_install) in cases {
        let (success, log, stderr) = run_npm_apply(query_success, package);
        assert!(success, "{label} failed: {stderr}\n{log}");
        let query = format!("fnm exec --using=default -- npm list --global --depth=0 -- {identity}\n");
        assert_eq!(log.matches(&query).count(), 1, "{label} did not query the configured identity once: {log}");
        assert!(!log.contains("ambient npm"), "{label} invoked ambient npm: {log}");
        let mutation = format!("fnm exec --using=default -- npm install --global -- {package}\n");
        assert_eq!(log.contains(&mutation), should_install, "{label} selected the wrong npm mutation: {log}");
    }
}

#[test]
fn mixed_package_states_keep_apply_ensure_only_and_update_configured_only() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let cargo_home = home.join(".cargo");
    let data_home = home.join(".local/share");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("packages.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        r#"version: 1.0.0
packages:
  cargo: [ripgrep, bat, probe]
  npm: [typescript, eslint]
  flatpak: [org.example.Present, org.example.Missing]
tools:
  rust: stable
  node: latest
updates:
  flatpak: true
  tools:
    rust: false
    node: false
  packages:
    cargo: true
    npm: true
"#,
    )
    .unwrap();

    write_executable(
        &cargo_home.join("bin/rustup"),
        "#!/bin/sh\nprintf 'rustup %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n",
    );
    write_executable(
        &cargo_home.join("bin/cargo"),
        r#"#!/bin/sh
printf 'cargo %s\n' "$*" >> "$COZYDOT_TEST_LOG"
if [ "$*" = "install --list" ]; then
  printf 'ripgrep v13.0.0:\n    rg\nprobe v1.0.0 (/tmp/probe source):\n    probe\neza v0.1.0:\n    eza\n'
fi
"#,
    );
    write_executable(
        &cargo_home.join("bin/cargo-binstall"),
        "#!/bin/sh\nprintf 'cargo-binstall %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n",
    );
    write_executable(
        &data_home.join("fnm/fnm"),
        r#"#!/bin/sh
printf 'fnm %s\n' "$*" >> "$COZYDOT_TEST_LOG"
case "$*" in
  "exec --using=default -- npm list --global --depth=0 -- typescript") exit 0 ;;
  "exec --using=default -- npm list --global --depth=0 -- eslint") exit 1 ;;
esac
"#,
    );
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    write_executable(&fake_bin.join("dpkg-query"), "#!/bin/sh\nprintf 'installed\\n'\n");
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf 'sudo %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");
    write_executable(&fake_bin.join("curl"), "#!/bin/sh\nexit 97\n");
    write_executable(
        &fake_bin.join("flatpak"),
        r#"#!/bin/sh
printf 'flatpak %s\n' "$*" >> "$COZYDOT_TEST_LOG"
case "$*" in
  "--user info --show-ref -- org.example.Present") printf 'app/org.example.Present/x86_64/stable\n' ;;
  "--user info --show-ref -- org.example.Missing") exit 1 ;;
  "--user info --show-ref -- org.example.Unrelated") printf 'app/org.example.Unrelated/x86_64/stable\n' ;;
esac
"#,
    );

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("CARGO_HOME", &cargo_home)
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    let apply = fs::read_to_string(&log).unwrap();
    assert!(apply.contains("cargo-binstall --no-confirm -- bat probe\n"), "missing Cargo apply convergence: {apply}");
    assert!(
        apply.contains("fnm exec --using=default -- npm install --global -- eslint\n"),
        "missing npm apply convergence through managed FNM: {apply}"
    );
    assert!(
        apply.contains("flatpak --user install --app --noninteractive -y flathub -- org.example.Missing\n"),
        "missing Flatpak apply convergence: {apply}"
    );
    for forbidden in [
        "cargo-binstall --no-confirm -- ripgrep",
        "cargo-binstall --no-confirm -- eza",
        "npm install --global -- typescript",
        "npm install --global -- prettier",
        "flatpak --user install --app --noninteractive -y flathub -- org.example.Present",
        "org.example.Unrelated",
    ] {
        assert!(!apply.contains(forbidden), "apply mutated a present or unrelated package via {forbidden:?}: {apply}");
    }

    fs::write(&log, "").unwrap();
    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("CARGO_HOME", &cargo_home)
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("update")
        .assert()
        .success();

    let update = fs::read_to_string(&log).unwrap();
    assert!(
        update.contains("cargo install --locked -- ripgrep bat probe\n"),
        "Cargo update was not configured-only: {update}"
    );
    assert!(
        update.contains("fnm exec --using=default -- npm install --global -- typescript eslint\n"),
        "npm update was not configured-only through managed FNM: {update}"
    );
    assert!(
        update.contains(
            "flatpak --user install --or-update --app --noninteractive -y flathub -- org.example.Present org.example.Missing\n"
        ),
        "Flatpak update did not use configured install --or-update convergence: {update}"
    );
    for unrelated in ["eza", "prettier", "org.example.Unrelated"] {
        assert!(!update.contains(unrelated), "update included unrelated package {unrelated:?}: {update}");
    }
    assert!(!update.contains("cargo-binstall"), "Cargo update did not use managed cargo: {update}");
}

#[test]
fn docker_logging_passes_max_size_through_to_daemon_json() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let captured = temp.path().join("daemon.json");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        "version: 1.0.0\nintegrations:\n  docker:\n    logging:\n      driver: local\n      max_size: native-tool-value\n",
    )
    .unwrap();
    write_executable(&fake_bin.join("docker"), "#!/bin/sh\nexit 0\n");
    write_executable(
        &fake_bin.join("sudo"),
        r#"#!/bin/sh
[ "$1" != "stat" ] || exit 1
if [ "$1" = "install" ] && [ "$2" = "-o" ]; then
  previous=
  before=
  for argument in "$@"; do before="$previous"; previous="$argument"; done
  cp "$before" "$COZYDOT_CAPTURE"
fi
"#,
    );

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_CAPTURE", &captured)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    let daemon: serde_json::Value = serde_json::from_str(&fs::read_to_string(captured).unwrap()).unwrap();
    assert_eq!(daemon["log-driver"], "local");
    assert_eq!(daemon["log-opts"]["max-size"], "native-tool-value");
}

#[test]
fn gnome_extension_state_uses_exact_info_query() {
    let temp = tempfile::tempdir().unwrap();
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("gnome.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        "version: 1.0.0\ndesktop:\n  gnome:\n    extensions: [installed@example.com, absent@example.com]\n",
    )
    .unwrap();
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nexit 0\n");
    write_executable(
        &fake_bin.join("gnome-extensions"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$COZYDOT_TEST_LOG"
[ "$*" != "info absent@example.com" ]
"#,
    );
    write_executable(&fake_bin.join("gnome-shell"), "#!/bin/sh\nprintf 'GNOME Shell 48.1\\n'\n");
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/bin/sh
case "$*" in
  *extension-info*) printf '%s' '{"shell_version_map":{"48":{"version":7}}}' ;;
  *)
    while [ "$#" -gt 0 ]; do
      [ "$1" != "-o" ] || { shift; printf 'zip' > "$1"; }
      shift
    done
    ;;
esac
"#,
    );

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    let log = fs::read_to_string(log).unwrap();
    assert!(log.starts_with("info installed@example.com\nenable installed@example.com\ninfo absent@example.com\n"));
    assert!(log.contains("install --force"));
    assert!(!log.contains("list"));
    assert!(!log.contains("enable absent@example.com"));
}

#[test]
fn python_update_control_validates_and_apply_uses_local_state() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_dir = temp.path().join("cozydot");
    let log = temp.path().join("tools.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        "version: 1.0.0\ntools:\n  python: \"3.13\"\nupdates:\n  tools:\n    python: true\n",
    )
    .unwrap();
    write_executable(&home.join(".local/bin/uv"), "#!/bin/sh\nprintf 'uv %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");
    let fake_bin = temp.path().join("bin");
    write_executable(&fake_bin.join("curl"), "#!/bin/sh\nexit 99\n");

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("UV_INSTALL_DIR", home.join(".local/bin"))
        .env("XDG_CONFIG_HOME", temp.path())
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    let log = fs::read_to_string(log).unwrap();
    assert!(log.contains("python find --no-config --managed-python --no-python-downloads --show-version -- 3.13"));
    assert!(!log.contains("python install"));
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
        "version: 1.0.0\ntools:\n  rust: stable\n  node: latest\n  python: \"3.13\"\nupdates:\n  tools:\n    rust: true\n    node: true\n    python: true\n",
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
            "rustup toolchain install --profile minimal --no-self-update --no-update -- stable\n",
            "rustup default -- stable\n",
            "fnm exec --using latest -- node --version\n",
            "fnm default -- latest\n",
            "uv python find --no-config --managed-python --no-python-downloads --show-version -- 3.13\n",
        )
    );

    fs::write(&log, "").unwrap();
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
        .arg("update")
        .assert()
        .success()
        .stdout(concat!(
            "Updating APT bootstrap packages\n",
            "Updating Rustup bootstrap\n",
            "Updating FNM bootstrap\n",
            "Updating uv bootstrap\n",
            "Updating Rust toolchain\n",
            "Updating Node.js toolchain\n",
            "Updating Python toolchain\n",
        ));
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        concat!(
            "rustup update --no-self-update stable\n",
            "rustup default -- stable\n",
            "fnm install --progress never -- latest\n",
            "fnm default -- latest\n",
            "uv python install --no-config --managed-python --no-progress --upgrade --default -- 3.13\n",
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
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "fnm exec --using lts-latest -- node --version\nfnm default -- lts-latest\n"
    );

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
    assert_eq!(fs::read_to_string(&log).unwrap(), "fnm exec --using 20 -- node --version\nfnm default -- 20\n");
}

#[test]
fn nerd_fonts_install_system_wide_after_download() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("fonts.log");
    let family = format!("CozydotTestFont{}", std::process::id());
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("cozydot.yaml"), format!("version: 1.0.0\nfonts:\n  nerd: [{family}]\n")).unwrap();
    write_executable(&fake_bin.join("curl"), "#!/bin/sh\nprintf 'curl %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf 'sudo %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    let log = fs::read_to_string(log).unwrap();
    let download = log.find(&format!("{family}.tar.xz")).unwrap();
    let replace = log.find(&format!("sudo rm --recursive --force -- /usr/share/fonts/{family}\n")).unwrap();
    let create = log.find(&format!("sudo mkdir --parents -- /usr/share/fonts/{family}\n")).unwrap();
    let extract = log.find(&format!("sudo tar --extract --xz --directory /usr/share/fonts/{family} --file ")).unwrap();
    let cache = log.find("sudo fc-cache --force /usr/share/fonts\n").unwrap();
    assert!(download < replace && replace < create && create < extract && extract < cache);
    assert!(!log.contains("--list"));
    assert!(!log.contains("font-stage"));
}

#[test]
fn go_minor_selector_tracks_its_latest_patch_and_extracts_directly() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("go.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("cozydot.yaml"), "version: 1.0.0\ntools:\n  go: \"99.88\"\n").unwrap();

    let archive_arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "arm" => "armv6l",
        architecture => panic!("unsupported test architecture: {architecture}"),
    };
    let metadata = format!(
        r#"[
  {{"version":"go99.89.1","stable":true,"files":[{{"filename":"go99.89.1.linux-{archive_arch}.tar.gz"}}]}},
  {{"version":"go99.88.5","stable":true,"files":[{{"filename":"go99.88.5.linux-{archive_arch}.tar.gz"}}]}},
  {{"version":"go99.88.4","stable":true,"files":[{{"filename":"go99.88.4.linux-{archive_arch}.tar.gz"}}]}}
]"#,
    );
    write_executable(
        &fake_bin.join("curl"),
        &format!(
            "#!/bin/sh\nprintf 'curl %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\ncase \"$*\" in\n  *mode=json*) printf '%s' '{}' ;;\nesac\n",
            metadata
        ),
    );
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf 'sudo %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");
    write_executable(&fake_bin.join("tar"), "#!/bin/sh\nprintf 'tar %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");

    Command::cargo_bin("cozydot")
        .unwrap()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    let log = fs::read_to_string(log).unwrap();
    assert!(log.contains(&format!("go99.88.5.linux-{archive_arch}.tar.gz")));
    assert!(!log.contains(&format!("go99.89.1.linux-{archive_arch}.tar.gz\n")));
    assert!(log.contains("sudo rm -rf -- /usr/local/go\n"));
    assert!(log.contains("sudo tar --extract --gzip --directory /usr/local --file"));
    assert!(!log.contains("sha256sum"));
    assert!(!log.contains("go-stage"));
    assert!(!log.contains("sudo mv"));
}

#[test]
fn deb_binary_uses_name_as_command_and_installs_only_when_missing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let config_dir = config_home.join("cozydot");
    let fake_bin = temp.path().join("bin");
    let log = temp.path().join("binary.log");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cozydot.yaml"),
        r#"version: 1.0.0
packages:
  binaries:
    - name: fastfetch
      format: deb
      source:
        provider: github
        repository: example/fastfetch
        assets:
          amd64: ""
          arm64: ""
          arm32: ""
"#,
    )
    .unwrap();
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\n/usr/bin/uname \"$@\"\n");
    write_executable(&fake_bin.join("dpkg-query"), "#!/bin/sh\nprintf 'not-installed\\n'\n");
    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf 'sudo %s\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/bin/sh
printf 'curl %s\n' "$*" >> "$COZYDOT_TEST_LOG"
output=
while [ "$#" -gt 0 ]; do
  [ "$1" != "--output" ] || { shift; output="$1"; }
  shift
done
if [ -n "$output" ]; then
  printf 'deb' > "$output"
else
  printf '%s' '{"assets":[{"name":"fastfetch.deb","browser_download_url":"https://example.com/fastfetch.deb"}]}'
fi
"#,
    );

    let apply = || {
        Command::cargo_bin("cozydot")
            .unwrap()
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &config_home)
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .env("COZYDOT_TEST_LOG", &log)
            .env("PATH", &fake_bin)
            .arg("apply")
            .assert()
            .success();
    };
    apply();
    let first = fs::read_to_string(&log).unwrap();
    assert!(first.contains("example/fastfetch/releases/latest"));
    assert!(first.contains("apt-get install -y -qq --"));
    assert!(!first.contains("dpkg-deb"));
    assert!(!first.contains("sha256"));

    fs::write(&log, "").unwrap();
    write_executable(&fake_bin.join("fastfetch"), "#!/bin/sh\nexit 0\n");
    write_executable(&fake_bin.join("curl"), "#!/bin/sh\nexit 1\n");
    apply();
    assert!(!fs::read_to_string(log).unwrap().contains("fastfetch/releases/latest"));
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
    let corrupt_launch = temp.path().join("corrupt-appimaged-launched");
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
integrations:
  appimaged: true
"#,
    )
    .unwrap();

    write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$COZYDOT_SUDO_LOG\"\n");
    write_executable(&fake_bin.join("apt-cache"), "#!/bin/sh\nexit 0\n");
    write_executable(&fake_bin.join("dpkg-query"), "#!/bin/sh\nprintf 'not-installed\\n'\n");
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
    write_executable(
        &home.join("Applications/appimaged.AppImage"),
        "#!/bin/sh\nprintf launched > \"$COZYDOT_CORRUPT_LAUNCH\"\n",
    );
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
        .env("COZYDOT_CORRUPT_LAUNCH", &corrupt_launch)
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
    assert_eq!(fs::read(home.join("Applications/fixed.AppImage")).unwrap(), fs::read("/bin/true").unwrap());
    assert_eq!(fs::read(home.join("Applications/appimaged.AppImage")).unwrap(), fs::read("/bin/true").unwrap());
    assert!(!corrupt_launch.exists());
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
        .env("COZYDOT_CORRUPT_LAUNCH", &corrupt_launch)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();
    let systemctl_calls = fs::read_to_string(systemctl_log).unwrap();
    assert_eq!(systemctl_calls.matches("is-active").count(), 4);
}
