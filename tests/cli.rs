use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn config(extra: &str) -> String {
    let mut value = json!({
        "version": "1",
        "system": {
            "debian": null,
            "ubuntu": null,
            "macos": {"xcode": {}}
        },
        "packages": {
            "linux": {},
            "macos": {"homebrew": {"formulae": [], "casks": []}}
        },
        "tools": {},
        "fonts": {},
        "dotfiles": {"packages": {"all": [], "linux": [], "macos": []}},
        "integrations": {"vscode": {"extensions": []}, "linux": {}},
        "desktop": null,
        "updates": {
            "packages": {"linux": {}, "macos": {"homebrew": {}}},
            "tools": {}
        }
    });
    let extra: Value = yaml_serde::from_str(extra).unwrap();
    fn merge(value: &mut Value, extra: Value) {
        match (value, extra) {
            (Value::Object(value), Value::Object(extra)) => {
                for (key, extra) in extra {
                    merge(value.entry(key).or_insert(Value::Null), extra);
                }
            }
            (value, extra) => *value = extra,
        }
    }
    merge(&mut value, extra);
    serde_json::to_string(&value).unwrap()
}

fn config_dir(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let root = temp.path().join("config/cozydot");
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_config(root: &Path, extra: &str) {
    fs::write(root.join("cozydot.yaml"), config(extra)).unwrap();
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

fn cozydot() -> Command {
    Command::cargo_bin("cozydot").unwrap()
}

fn write_linux_host_fakes(fake_bin: &Path) {
    write_executable(&fake_bin.join("dpkg-query"), "#!/bin/sh\nprintf 'installed\\n'\n");
}

#[test]
fn cli_contracts() {
    for args in [Vec::<&str>::new(), vec!["--help"]] {
        cozydot().args(args).assert().success().stdout(
            predicate::str::contains("Declarative Linux and macOS post-install, update, and dotfile manager")
                .and(predicate::str::contains("init"))
                .and(predicate::str::contains("apply"))
                .and(predicate::str::contains("check"))
                .and(predicate::str::contains("dotfiles"))
                .and(predicate::str::contains("update")),
        );
    }
    cozydot().arg("--version").assert().success().stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));

    let temp = tempfile::tempdir().unwrap();
    for command in ["apply", "check", "dotfiles", "update"] {
        cozydot().env("XDG_CONFIG_HOME", temp.path()).arg(command).assert().failure().stderr(
            predicate::str::contains("active config not found at").and(predicate::str::contains("run `cozydot init`")),
        );
    }
    cozydot()
        .env("XDG_CONFIG_HOME", temp.path())
        .args(["init", "--preset", "unknown"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'unknown'"));
}

#[test]
fn cli_rejects_relative_xdg_config_home() {
    cozydot()
        .env("XDG_CONFIG_HOME", "relative")
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("XDG_CONFIG_HOME must be an absolute path"));
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
        .arg("-v")
        .arg("1")
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .env("COZYDOT_TEST_DOWNLOAD", &download)
        .assert()
        .failure()
        .stderr(predicate::str::contains("error: unsupported platform: Darwin x86_64"));
    assert!(!download.exists());
}

#[test]
fn installer_checksum_failure_preserves_existing_binary() {
    let temp = tempfile::tempdir().unwrap();
    let fake_bin = temp.path().join("bin");
    let home = temp.path().join("home");
    let install_dir = home.join(".local/bin");
    fs::create_dir_all(&install_dir).unwrap();
    fs::write(install_dir.join("cozydot"), "existing\n").unwrap();
    write_executable(
        &fake_bin.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -s) printf 'Linux\\n' ;; -m) printf 'x86_64\\n' ;; esac\n",
    );
    write_executable(
        &fake_bin.join("curl"),
        r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
  [ "$1" != "-w" ] || { printf 'https://example/v1\n'; exit 0; }
  [ "$1" != "-o" ] || { shift; output=$1; }
  shift
done
case "$output" in
  *.sha256) printf '%064d  release.tar.gz\n' 0 > "$output" ;;
  *) printf 'not the published archive' > "$output" ;;
esac
"#,
    );

    Command::new("bash")
        .arg("install.sh")
        .arg("-v")
        .arg("1")
        .env("HOME", &home)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .assert()
        .failure()
        .stderr(predicate::str::contains("error: checksum verification failed"));
    assert_eq!(fs::read_to_string(install_dir.join("cozydot")).unwrap(), "existing\n");
}

#[test]
fn init_materializes_presets_and_preserves_user_edits() {
    for preset in ["cozydot", "cli", "vm"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cozydot");
        cozydot()
            .env("XDG_CONFIG_HOME", temp.path())
            .args(["init", "--preset", preset])
            .assert()
            .success()
            .stdout(format!("Initialized Cozydot at {}\n", root.display()))
            .stderr("");
        cozydot()
            .env("XDG_CONFIG_HOME", temp.path())
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .arg("check")
            .assert()
            .success()
            .stdout(format!("Validated {}\n", root.join("cozydot.yaml").display()))
            .stderr("");
        assert_eq!(fs::read(root.join("cozydot.yaml")).unwrap(), fs::read(format!("configs/{preset}.yaml")).unwrap());
        assert!(root.join(".managed-files").is_file());
        assert!(root.join("dotfiles/bash/.bashrc").is_file());
    }

    let temp = tempfile::tempdir().unwrap();
    cozydot().env("XDG_CONFIG_HOME", temp.path()).arg("init").assert().success();
    let active = temp.path().join("cozydot/cozydot.yaml");
    fs::write(&active, "user edit\n").unwrap();
    cozydot().env("XDG_CONFIG_HOME", temp.path()).arg("init").assert().success();
    assert_eq!(fs::read_to_string(active).unwrap(), "user edit\n");
}

#[test]
fn init_preserves_entire_dotfile_package_when_one_file_changes() {
    let temp = tempfile::tempdir().unwrap();
    cozydot().env("XDG_CONFIG_HOME", temp.path()).arg("init").assert().success();
    let root = temp.path().join("cozydot");
    let edited = root.join("dotfiles/yazi/.config/yazi/yazi.toml");
    let sibling = root.join("dotfiles/yazi/.config/yazi/theme.toml");
    let bundled_edited = fs::read(&edited).unwrap();
    let bundled_sibling = fs::read(&sibling).unwrap();
    fs::write(&edited, "user edit\n").unwrap();

    let previous = b"previous bundled theme\n";
    fs::write(&sibling, previous).unwrap();
    let relative = "dotfiles/yazi/.config/yazi/theme.toml";
    let hash = hex::encode(Sha256::digest(previous));
    let manifest = fs::read_to_string(root.join(".managed-files"))
        .unwrap()
        .lines()
        .map(|line| {
            let (_, path) = line.split_once('\t').unwrap();
            if path == relative { format!("{hash}\t{path}") } else { line.to_owned() }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(root.join(".managed-files"), format!("{manifest}\n")).unwrap();

    cozydot().env("XDG_CONFIG_HOME", temp.path()).arg("init").assert().success();
    assert_eq!(fs::read_to_string(&edited).unwrap(), "user edit\n");
    assert_eq!(fs::read(&sibling).unwrap(), previous);

    fs::write(&edited, bundled_edited).unwrap();
    cozydot().env("XDG_CONFIG_HOME", temp.path()).arg("init").assert().success();
    assert_eq!(fs::read(&sibling).unwrap(), bundled_sibling);
}

#[test]
#[cfg(target_os = "linux")]
fn invalid_config_prevents_host_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_dir(&temp);
    let fake_bin = temp.path().join("bin");
    let mutation = temp.path().join("mutation");
    fs::write(root.join("cozydot.yaml"), "version: [\n").unwrap();
    for command in ["sudo", "curl", "gpg", "stow", "systemctl", "gsettings", "code"] {
        write_executable(&fake_bin.join(command), "#!/bin/sh\n: > \"$COZYDOT_TEST_MUTATION\"\nexit 99\n");
    }

    cozydot()
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("COZYDOT_TEST_MUTATION", &mutation)
        .env("PATH", &fake_bin)
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error: parse").and(predicate::str::contains("cozydot.yaml")));
    assert!(!mutation.exists());

    let repo = |extra: &str| {
        format!(
            "packages:\n  linux:\n    apt:\n      repos:\n        - name: vendor\n          key_url: https://example.com/key\n          key_path: /etc/apt/keyrings/vendor.gpg\n          uris: {{default: https://example.com/repo}}\n          suite: stable\n          components: [main]\n{extra}"
        )
    };
    for (extra, error) in [
        (repo("          path: /\n"), "packages.linux.apt.repos[0]: unknown field `path`"),
        (repo("          arch: [sparc]\n"), "packages.linux.apt.repos[0].arch[0]: unknown variant `sparc`"),
        (repo("          arch: []\n"), "arch: must not be empty"),
        (repo("").replace("/etc/apt/keyrings/vendor.gpg", "/tmp/vendor.gpg"), "direct child"),
        (
            "packages:\n  linux:\n    apt:\n      repos:\n        - name: vendor\n          key_url: key\n          key_path: /etc/apt/keyrings/vendor.gpg\n          uris: {default: source}\n          components: [main]\n".to_owned(),
            "missing field `suite`",
        ),
        (
            "packages:\n  linux:\n    apt:\n      repos:\n        - name: vendor\n          key_url: key\n          key_path: /etc/apt/keyrings/vendor.gpg\n          uris: {default: source}\n          suite: stable\n".to_owned(),
            "missing field `components`",
        ),
    ] {
        write_config(&root, &extra);
        cozydot()
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .arg("check")
            .assert()
            .failure()
            .stderr(predicate::str::contains(error));
    }
}

#[test]
#[cfg(target_os = "linux")]
fn empty_apply_and_update_establish_the_linux_baseline() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_dir(&temp);
    let fake_bin = temp.path().join("bin");
    let mutation = temp.path().join("mutation");
    let apt_log = temp.path().join("apt.log");
    write_config(&root, "{}");
    write_linux_host_fakes(&fake_bin);
    for command in ["curl", "gpg", "stow", "flatpak", "rustup"] {
        write_executable(&fake_bin.join(command), "#!/bin/sh\n: > \"$COZYDOT_TEST_MUTATION\"\nexit 99\n");
    }
    write_executable(
        &fake_bin.join("sudo"),
        r#"#!/bin/sh
case "$*" in
  "apt-get update -qq"|"DEBIAN_FRONTEND=noninteractive apt-get install --no-upgrade -y -qq -- ca-certificates+ curl+ fontconfig+ gnupg+ stow+ unzip+ xdg-terminal-exec+ xz-utils+") ;;
  *) : > "$COZYDOT_TEST_MUTATION"; exit 99 ;;
esac
printf '%s\n' "$*" >> "$COZYDOT_TEST_APT_LOG"
"#,
    );

    let command = || {
        let mut command = cozydot();
        command
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .env("COZYDOT_TEST_MUTATION", &mutation)
            .env("COZYDOT_TEST_APT_LOG", &apt_log)
            .env("PATH", &fake_bin);
        command
    };
    let baseline = "    Updating APT package metadata\n  Installing APT prerequisites\n";
    command().arg("apply").assert().success().stdout("").stderr(baseline);
    command().arg("update").assert().success().stdout("").stderr(baseline);
    assert_eq!(
        fs::read_to_string(apt_log).unwrap(),
        concat!(
            "apt-get update -qq\n",
            "DEBIAN_FRONTEND=noninteractive apt-get install --no-upgrade -y -qq -- ca-certificates+ curl+ fontconfig+ gnupg+ stow+ unzip+ xdg-terminal-exec+ xz-utils+\n",
            "apt-get update -qq\n",
            "DEBIAN_FRONTEND=noninteractive apt-get install --no-upgrade -y -qq -- ca-certificates+ curl+ fontconfig+ gnupg+ stow+ unzip+ xdg-terminal-exec+ xz-utils+\n"
        )
    );
    assert!(!mutation.exists());
}

#[test]
#[cfg(target_os = "linux")]
fn sudo_group_membership_is_not_applied_on_a_non_debian_host() {
    if os_release_value("ID") == "debian" {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = config_dir(&temp);
    let fake_bin = temp.path().join("bin");
    let mutation = temp.path().join("mutation");
    write_config(&root, "system:\n  debian:\n    sudo_group: true\n");
    write_linux_host_fakes(&fake_bin);
    write_executable(
        &fake_bin.join("sudo"),
        "#!/bin/sh\ncase \"$*\" in 'apt-get update -qq'|DEBIAN_FRONTEND=noninteractive\\ apt-get\\ install\\ --no-upgrade\\ *) exit 0 ;; esac\n: > \"$COZYDOT_TEST_MUTATION\"\nexit 99\n",
    );

    cozydot()
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("COZYDOT_TEST_MUTATION", &mutation)
        .env("PATH", &fake_bin)
        .arg("apply")
        .assert()
        .success()
        .stdout("")
        .stderr("    Updating APT package metadata\n  Installing APT prerequisites\n");
    assert!(!mutation.exists());
}

#[test]
#[cfg(target_os = "linux")]
fn dotfiles_refuse_conflicts_and_replace_only_when_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let root = config_dir(&temp);
    let fake_bin = temp.path().join("bin");
    let source = root.join("dotfiles/bash/.bashrc");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::write(&source, "managed\n").unwrap();
    fs::write(home.join(".bashrc"), "existing\n").unwrap();
    write_config(&root, "dotfiles:\n  packages:\n    all: [bash, missing]\n");
    write_executable(&fake_bin.join("uname"), "#!/bin/sh\nprintf 'x86_64\\n'\n");
    write_executable(
        &fake_bin.join("stow"),
        r#"#!/bin/sh
[ "${1-}" = "--version" ] && exit 0
simulate=false
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dir) dir=$2; shift 2 ;;
    --target) target=$2; shift 2 ;;
    --simulate) simulate=true; shift ;;
    --) package=$2; break ;;
  esac
done
$simulate && { [ ! -e "$target/.bashrc" ]; exit; }
ln -s "$dir/$package/.bashrc" "$target/.bashrc"
"#,
    );

    let command = || {
        let mut command = cozydot();
        command
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()));
        command
    };
    command()
        .args(["dotfiles", "--replace"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("dotfiles package \"missing\" does not exist"));
    assert_eq!(fs::read_to_string(home.join(".bashrc")).unwrap(), "existing\n");
    assert!(!state.exists());

    write_config(&root, "dotfiles:\n  packages:\n    all: [bash]\n");
    command().arg("dotfiles").assert().failure().stderr(predicate::str::contains("stow package check"));
    assert_eq!(fs::read_to_string(home.join(".bashrc")).unwrap(), "existing\n");
    assert!(!state.exists());

    command().args(["dotfiles", "--replace"]).assert().success().stdout("").stderr("    Applying dotfiles\n");
    assert_eq!(fs::canonicalize(home.join(".bashrc")).unwrap(), fs::canonicalize(source).unwrap());
    let backups = state.join("cozydot/dotfile-backups");
    let backup = fs::read_dir(backups).unwrap().next().unwrap().unwrap().path().join("bash/.bashrc");
    assert_eq!(fs::read_to_string(backup).unwrap(), "existing\n");
}

fn repo_config() -> String {
    config(
        r#"packages:
  linux:
    apt:
      install: [direct-package]
      repos:
        - name: armored
          key_url: https://example.com/armored
          key_path: /etc/apt/keyrings/armored.asc
          uris: {default: https://example.com/armored}
          suite: stable
          components: [main]
          conflicts: [old-package, absent-conflict]
          packages: [vendor-one]
        - name: binary
          key_url: https://example.com/binary
          key_path: /usr/share/keyrings/binary.gpg
          uris: {default: https://example.com/binary}
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

#[test]
#[cfg(target_os = "linux")]
fn theme_configures_the_gnome_color_scheme() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_dir(&temp);
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    let log = temp.path().join("apply.log");
    fs::create_dir_all(state.join("files")).unwrap();
    fs::create_dir_all(state.join("packages")).unwrap();
    fs::write(state.join("files/debian.sources"), "Components: main\n").unwrap();
    write_config(&root, "desktop:\n  theme: dark\n");
    write_apt_fakes(&fake_bin);
    write_executable(
        &fake_bin.join("gsettings"),
        "#!/bin/sh\nprintf 'gsettings %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n",
    );

    cozydot()
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_CURRENT_DESKTOP", "gnome")
        .env("COZYDOT_TEST_LOG", &log)
        .env("COZYDOT_TEST_STATE", &state)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .assert()
        .success();

    assert!(
        fs::read_to_string(log)
            .unwrap()
            .lines()
            .any(|line| line == "gsettings set org.gnome.desktop.interface color-scheme 'prefer-dark'")
    );
}

fn run_terminal_apply(desktop: &str, terminal_key: bool, custom_keybindings: &str) -> (Vec<String>, String, bool) {
    let temp = tempfile::tempdir().unwrap();
    let root = config_dir(&temp);
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    let log = temp.path().join("apply.log");
    fs::create_dir_all(state.join("files")).unwrap();
    fs::create_dir_all(state.join("packages")).unwrap();
    fs::write(state.join("files/debian.sources"), "Components: main\n").unwrap();
    write_config(&root, "desktop:\n  linux:\n    gnome:\n      terminal: wezterm\n");
    write_apt_fakes(&fake_bin);
    write_executable(&fake_bin.join("wezterm"), "#!/bin/sh\nexit 0\n");
    write_executable(
        &fake_bin.join("xdg-terminal-exec"),
        "#!/bin/sh\n[ -f \"$COZYDOT_TEST_STATE/packages/xdg-terminal-exec\" ] || exit 99\n[ \"$1\" = --print-id ] && printf 'wezterm.desktop\\n'\n",
    );
    write_executable(
        &fake_bin.join("gsettings"),
        r#"#!/bin/sh
printf 'gsettings %s\n' "$*" >> "$COZYDOT_TEST_LOG"
case "$1:$2:$3" in
  get:org.gnome.settings-daemon.plugins.media-keys:terminal)
    [ "${COZYDOT_TEST_TERMINAL_KEY-}" = true ] || exit 1
    printf "'<Primary><Alt>t'\n" ;;
  get:org.gnome.settings-daemon.plugins.media-keys:custom-keybindings)
    printf '%s\n' "$COZYDOT_TEST_CUSTOM_KEYBINDINGS" ;;
esac
"#,
    );

    let output = cozydot()
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_CURRENT_DESKTOP", desktop)
        .env("COZYDOT_TEST_LOG", &log)
        .env("COZYDOT_TEST_STATE", &state)
        .env("COZYDOT_TEST_TERMINAL_KEY", terminal_key.to_string())
        .env("COZYDOT_TEST_CUSTOM_KEYBINDINGS", custom_keybindings)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .arg("apply")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let calls = fs::read_to_string(log)
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("gsettings "))
        .map(str::to_owned)
        .collect();
    let preference = fs::read_to_string(temp.path().join("config/xdg-terminals.list")).unwrap_or_default();
    (calls, preference, state.join("packages/xdg-terminal-exec").exists())
}

#[test]
#[cfg(target_os = "linux")]
fn terminal_configuration_handles_gnome_shortcut_capabilities() {
    const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
    const CUSTOM_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/cozydot-terminal/";
    let custom_schema = format!("{MEDIA_KEYS}.custom-keybinding:{CUSTOM_PATH}");

    let (calls, preference, prerequisite) = run_terminal_apply("gnome", true, "@as []");
    assert_eq!(calls, ["gsettings get org.gnome.settings-daemon.plugins.media-keys terminal"]);
    assert_eq!(preference, "wezterm.desktop\n");
    assert!(prerequisite);

    let (calls, preference, prerequisite) = run_terminal_apply(
        "gnome",
        false,
        "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']",
    );
    assert_eq!(
        calls,
        [
            "gsettings get org.gnome.settings-daemon.plugins.media-keys terminal".to_owned(),
            "gsettings get org.gnome.settings-daemon.plugins.media-keys custom-keybindings".to_owned(),
            format!("gsettings set {custom_schema} name 'Terminal'"),
            format!("gsettings set {custom_schema} command 'xdg-terminal-exec'"),
            format!("gsettings set {custom_schema} binding '<Primary><Alt>T'"),
            format!(
                "gsettings set {MEDIA_KEYS} custom-keybindings ['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/', '{CUSTOM_PATH}']"
            ),
        ]
    );
    assert_eq!(preference, "wezterm.desktop\n");
    assert!(prerequisite);
}

fn run_apt(
    config_home: &Path,
    fake_bin: &Path,
    state: &Path,
    log: &Path,
    extra_env: Option<(&str, &str)>,
) -> std::process::Output {
    fs::write(log, "").unwrap();
    let mut command = cozydot();
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
#[cfg(target_os = "linux")]
fn repo_key_validation_precedes_repo_file_write() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_dir(&temp);
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
        "packages:\n  linux:\n    apt:\n      repos:\n        - name: vendor\n          key_url: https://example.com/key\n          key_path: /etc/apt/keyrings/vendor.gpg\n          uris: {default: https://example.com/repo}\n          suite: stable\n          components: [main]\n",
    );
    write_apt_fakes(&fake_bin);

    let output = run_apt(&temp.path().join("config"), &fake_bin, &state, &log, Some(("COZYDOT_TEST_NO_PUBLIC", "1")));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("repo key validation found no public key"));
    assert!(!state.join("files/vendor.gpg").exists());
    assert!(!state.join("files/vendor.list").exists());
}

#[test]
#[cfg(target_os = "linux")]
fn apply_writes_repo_files_and_installs_packages_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let root = config_dir(&temp);
    let fake_bin = temp.path().join("bin");
    let state = temp.path().join("state");
    let log = temp.path().join("apt.log");
    fs::create_dir_all(state.join("files")).unwrap();
    fs::create_dir_all(state.join("packages")).unwrap();
    fs::write(root.join("cozydot.yaml"), repo_config()).unwrap();
    for package in ["ca-certificates", "curl", "gnupg", "old-package"] {
        fs::write(state.join("packages").join(package), "").unwrap();
    }
    write_apt_fakes(&fake_bin);

    let output = run_apt(&temp.path().join("config"), &fake_bin, &state, &log, None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let first = fs::read_to_string(&log).unwrap();
    let lines = first.lines().collect::<Vec<_>>();
    let position = |needle: &str| lines.iter().position(|line| line.contains(needle)).unwrap();
    let direct_install = position("apt-get install --no-upgrade -y -qq -- direct-package+");
    let repo_download = position("curl ");
    let source_list_write = position("/etc/apt/sources.list.d/armored.list");
    let apt_update = lines
        .iter()
        .enumerate()
        .find(|(index, line)| *index > source_list_write && **line == "sudo apt-get update -qq")
        .map(|(index, _)| index)
        .unwrap();
    let purge = position("apt-get purge -y -qq -- old-package");
    let install = position("apt-get install --no-upgrade -y -qq -- vendor-one+ vendor-two+");
    assert!(direct_install < repo_download);
    assert!(repo_download < source_list_write && source_list_write < apt_update);
    assert!(apt_update < purge && purge < install);
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
    assert_eq!(second.matches(" apt-get install --no-upgrade ").count(), 3);
    assert!(!second.contains(" apt-get purge "));
}

#[test]
#[cfg(target_os = "linux")]
fn inapplicable_repos_have_no_side_effects() {
    let inapplicable_distro = if os_release_value("ID") == "linuxmint" { "pop" } else { "linuxmint" };
    for applicability in [
        "            default: https://example.com/repo\n          arch: [arm64]".to_owned(),
        format!("            {inapplicable_distro}: https://example.com/repo"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = config_dir(&temp);
        let fake_bin = temp.path().join("bin");
        let mutation = temp.path().join("mutation");
        write_config(
            &root,
            &format!(
                "packages:\n  linux:\n    apt:\n      repos:\n        - name: skipped\n          key_url: https://example.com/key\n          key_path: /etc/apt/keyrings/skipped.gpg\n          uris:\n{applicability}\n          suite: stable\n          components: [main]\n          conflicts: [old]\n          packages: [new]\n"
            ),
        );
        write_linux_host_fakes(&fake_bin);
        for command in ["curl", "gpg"] {
            write_executable(&fake_bin.join(command), "#!/bin/sh\n: > \"$COZYDOT_TEST_MUTATION\"\nexit 99\n");
        }
        write_executable(
            &fake_bin.join("sudo"),
            "#!/bin/sh\ncase \"$*\" in 'apt-get update -qq'|DEBIAN_FRONTEND=noninteractive\\ apt-get\\ install\\ --no-upgrade\\ *) exit 0 ;; esac\n: > \"$COZYDOT_TEST_MUTATION\"\nexit 99\n",
        );
        cozydot()
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_CURRENT_DESKTOP", "gnome")
            .env("COZYDOT_TEST_MUTATION", &mutation)
            .env("PATH", &fake_bin)
            .arg("apply")
            .assert()
            .success()
            .stdout("")
            .stderr("    Updating APT package metadata\n  Installing APT prerequisites\n");
        assert!(!mutation.exists());
    }
}

#[test]
#[cfg(target_os = "linux")]
fn update_runs_only_the_selected_apt_upgrade_command() {
    for (policy, expected) in [
        (
            "upgrade",
            concat!(
                "sudo apt-get update -qq\n",
                "sudo DEBIAN_FRONTEND=noninteractive apt-get upgrade -y -qq\n",
                "sudo DEBIAN_FRONTEND=noninteractive apt-get install --no-upgrade -y -qq -- ca-certificates+ curl+ fontconfig+ gnupg+ stow+ unzip+ xdg-terminal-exec+ xz-utils+\n"
            ),
        ),
        (
            "full-upgrade",
            concat!(
                "sudo apt-get update -qq\n",
                "sudo DEBIAN_FRONTEND=noninteractive apt-get full-upgrade -y -qq\n",
                "sudo DEBIAN_FRONTEND=noninteractive apt-get autoremove --purge -y -qq\n",
                "sudo DEBIAN_FRONTEND=noninteractive apt-get install --no-upgrade -y -qq -- ca-certificates+ curl+ fontconfig+ gnupg+ stow+ unzip+ xdg-terminal-exec+ xz-utils+\n"
            ),
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = config_dir(&temp);
        let fake_bin = temp.path().join("bin");
        let log = temp.path().join("update.log");
        write_config(&root, &format!("updates:\n  packages:\n    linux:\n      apt: {policy}\n"));
        write_linux_host_fakes(&fake_bin);
        write_executable(&fake_bin.join("sudo"), "#!/bin/sh\nprintf 'sudo %s\\n' \"$*\" >> \"$COZYDOT_TEST_LOG\"\n");

        cozydot()
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
