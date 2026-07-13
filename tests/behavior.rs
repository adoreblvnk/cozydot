use cozydot::{
    config::Config,
    operations, planner,
    platform::Platform,
    runner::{command_exists_in, Condition, Step},
};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

fn platform(distro: &str) -> Platform {
    Platform::from_parts(
        distro.into(),
        "ubuntu".into(),
        "noble".into(),
        "gnome".into(),
        "x86_64",
    )
    .unwrap()
}

struct Host {
    _dir: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    log: PathBuf,
}

impl Host {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(dir.path().join("tmp")).unwrap();
        symlink(
            assert_cmd::cargo::cargo_bin!("cozydot"),
            bin.join("cozydot"),
        )
        .unwrap();
        Self {
            log: dir.path().join("commands.log"),
            _dir: dir,
            home,
            bin,
        }
    }

    fn fake(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        fs::write(&path, format!("#!/bin/bash\nset -euo pipefail\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn logging_fake(&self, name: &str) {
        self.fake(name, &format!("printf '%s %s\\n' {name} \"$*\" >>\"$LOG\""));
    }

    fn run(&self, step: &Step) -> std::process::Output {
        self.run_with_path(step, format!("{}:/usr/bin:/bin", self.bin.display()))
    }

    fn run_with_path(&self, step: &Step, path: String) -> std::process::Output {
        if let Step::Workflow(operation) = step {
            let env = [
                ("HOME".into(), self.home.as_os_str().to_owned()),
                (
                    "CARGO_HOME".into(),
                    self.home.join(".cargo").into_os_string(),
                ),
                ("USER".into(), "tester".into()),
                ("LOG".into(), self.log.as_os_str().to_owned()),
                (
                    "TMPDIR".into(),
                    self._dir.path().join("tmp").into_os_string(),
                ),
                ("PATH".into(), path.clone().into()),
                (
                    "XDG_CONFIG_HOME".into(),
                    self.home.join(".config").into_os_string(),
                ),
                (
                    "XDG_DATA_HOME".into(),
                    self.home.join(".local/share").into_os_string(),
                ),
            ];
            let result = operations::execute(operation, &env);
            return Command::new("sh")
                .args(["-c", if result.is_ok() { "exit 0" } else { "exit 1" }])
                .output()
                .unwrap();
        }
        if let Step::Conditional { condition, action } = step {
            if !self.condition_matches(condition) {
                return Command::new("sh").args(["-c", "exit 0"]).output().unwrap();
            }
            return self.run_with_path(action, path);
        }
        let command_step = step.command().expect("command or shell step");
        let mut command = Command::new(&command_step.program);
        command
            .args(&command_step.args)
            .env("HOME", &self.home)
            .env("USER", "tester")
            .env("LOG", &self.log)
            .env("TMPDIR", self._dir.path().join("tmp"))
            .env("PATH", path)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_DATA_HOME", self.home.join(".local/share"))
            .env_remove("CARGO_HOME");
        command.output().unwrap()
    }

    fn condition_matches(&self, condition: &Condition) -> bool {
        let command = |program: &str, args: &[&str]| {
            Command::new(self.bin.join(program))
                .args(args)
                .env("HOME", &self.home)
                .env("USER", "tester")
                .env("LOG", &self.log)
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        };
        match condition {
            Condition::CommandExists(name) => command_exists_in(name, self.bin.as_os_str()),
            Condition::CommandMissing(name) => !command_exists_in(name, self.bin.as_os_str()),
            Condition::PackageInstalled(name) => command("dpkg-query", &["-W", name]),
            Condition::PackageMissing(name) => !command("dpkg-query", &["-W", name]),
            Condition::FileExists(path) => path.exists(),
            Condition::FileMissing(path) => !path.exists(),
            other => panic!("condition not used by behavior harness: {other:?}"),
        }
    }

    fn run_ok(&self, step: &Step) {
        let output = self.run(step);
        assert!(
            output.status.success(),
            "{} failed\nstdout: {}\nstderr: {}\nlog: {}",
            step.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            self.log()
        );
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

fn plans(config: &str, command: &str, distro: &str) -> Vec<Step> {
    let cfg = Config::load(Path::new(config)).unwrap();
    planner::plan(command, &cfg, &platform(distro), Path::new(".")).unwrap()
}

fn step_containing(steps: &[Step], needle: &str) -> Step {
    steps
        .iter()
        .find(|step| step.display().contains(needle))
        .unwrap_or_else(|| panic!("no planned step contains {needle}"))
        .clone()
}

#[test]
fn latest_go_ignores_prereleases_and_verifies_matching_stable_checksum() {
    let host = Host::new();
    let fixture = Path::new("tests/fixtures/go-releases-prerelease-first.json");
    host.fake(
        "curl",
        &format!(
            r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; url=''
while [ "$#" -gt 0 ]; do
  case "$1" in -o) out=$2; shift 2 ;; http*) url=$1; shift ;; *) shift ;; esac
done
if [[ "$url" == *'mode=json'* ]]; then cat '{}'; else printf archive >"$out"; fi"#,
            fixture.display()
        ),
    );
    host.fake("go", "printf 'go version go1.25.7 linux/amd64\n'");
    host.fake("sha256sum", "input=$(cat); printf 'sha256sum %s\\n' \"$input\" >>\"$LOG\"; [[ \"$input\" == cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc* ]]");
    host.fake(
        "tar",
        r#"printf 'tar %s\n' "$*" >>"$LOG"
if [ "$1" = -C ]; then mkdir -p "$2/go/bin"; printf '#!/bin/sh\n' >"$2/go/bin/go"; chmod +x "$2/go/bin/go"; fi"#,
    );
    host.logging_fake("sudo");
    let step = step_containing(
        &plans("configs/cli.yaml", "install", "ubuntu"),
        "workflow go-install",
    );
    host.run_ok(&step);
    let log = host.log();
    assert!(log.contains("include=all"));
    assert!(log.contains("go1.26.1.linux-amd64.tar.gz"));
    assert!(
        log.contains("sha256sum cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
    );
    assert!(!log.contains("go1.27rc2.linux"));
}

#[test]
fn exact_go_version_reruns_without_release_metadata() {
    let host = Host::new();
    host.fake("go", "printf 'go version go1.26.1 linux/amd64\\n'");
    host.fake("curl", "printf 'unexpected curl\\n' >>\"$LOG\"; exit 42");
    let step = Step::workflow(operations::Operation::GoInstall {
        version: "1.26.1".into(),
        arch: "amd64".into(),
    });
    host.run_ok(&step);
    assert!(!host.log().contains("unexpected curl"), "{}", host.log());
}

#[test]
fn fresh_fnm_bootstrap_exposes_npm_for_configured_packages_in_same_step() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done
cat >"$out" <<'INSTALL'
#!/bin/bash
mkdir -p "$XDG_DATA_HOME/fnm"
cat >"$XDG_DATA_HOME/fnm/fnm" <<'FNM'
#!/bin/bash
printf 'fnm %s\n' "$*" >>"$LOG"
case "$1" in
 env) printf 'export FNM_MULTISHELL_PATH="%s/multishell"\nexport PATH="%s:$PATH"\n' "$XDG_DATA_HOME/fnm" "$XDG_DATA_HOME/fnm" ;;
 install) [ -n "${FNM_MULTISHELL_PATH:-}" ] || { printf 'missing fnm environment\n' >&2; exit 1; } ;;
 current) [ -n "${FNM_MULTISHELL_PATH:-}" ] || { printf 'missing fnm environment\n' >&2; exit 1; }; printf 'v22.1.0\n' ;;
esac
FNM
cat >"$XDG_DATA_HOME/fnm/npm" <<'NPM'
#!/bin/bash
[ -n "${FNM_MULTISHELL_PATH:-}" ] || { printf 'missing fnm environment\n' >&2; exit 1; }
printf 'npm %s\n' "$*" >>"$LOG"
NPM
chmod +x "$XDG_DATA_HOME/fnm/fnm" "$XDG_DATA_HOME/fnm/npm"
INSTALL"#,
    );
    let step = step_containing(
        &plans("configs/cli.yaml", "install", "ubuntu"),
        "workflow node-install",
    );
    host.run_ok(&step);
    let log = host.log();
    assert!(log.contains("fnm install --lts --use"));
    assert!(log.contains("npm install --global"));
    assert!(log.contains("opencode-ai"));
}

#[test]
fn fresh_rustup_cargo_path_bootstraps_binstall_and_installs_packages() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done
cat >"$out" <<'INSTALL'
#!/bin/bash
mkdir -p "$HOME/.cargo/bin"
cat >"$HOME/.cargo/bin/rustup" <<'CMD'
#!/bin/bash
printf 'rustup %s\n' "$*" >>"$LOG"
CMD
cat >"$HOME/.cargo/bin/cargo" <<'CMD'
#!/bin/bash
printf 'cargo %s\n' "$*" >>"$LOG"
if [ "${1:-}" = install ] && [ "${2:-}" = cargo-binstall ]; then
  cat >"$HOME/.cargo/bin/cargo-binstall" <<'BIN'
#!/bin/bash
printf 'cargo-binstall %s\n' "$*" >>"$LOG"
BIN
  chmod +x "$HOME/.cargo/bin/cargo-binstall"
elif [ "${1:-}" = binstall ]; then
  command cargo-binstall "${@:2}"
fi
CMD
chmod +x "$HOME/.cargo/bin/rustup" "$HOME/.cargo/bin/cargo"
INSTALL"#,
    );
    let steps = plans("configs/cli.yaml", "install", "ubuntu");
    host.run_ok(&step_containing(&steps, "workflow rustup-bootstrap"));
    assert!(
        host.home.join(".cargo/bin/cargo").is_file(),
        "rustup bootstrap did not create cargo; log: {}",
        host.log()
    );
    host.run_ok(&step_containing(&steps, "workflow cargo-packages"));
    let log = host.log();
    assert!(
        log.contains("cargo install cargo-binstall --locked"),
        "{log}"
    );
    assert!(log.contains("cargo-binstall --no-confirm"), "{log}");
}

#[test]
fn configured_cargo_packages_fail_when_bootstrap_did_not_create_cargo() {
    let host = Host::new();
    let step = Step::workflow(operations::Operation::CargoPackages {
        packages: vec!["bat --locked".into()],
        force: false,
    });
    assert!(!host.run(&step).status.success());
}

#[test]
fn real_cli_check_disables_purge_after_fake_package_purge() {
    let host = Host::new();
    let root = host.home.join(".config/cozydot");
    fs::create_dir_all(root.join("dotfiles/bash")).unwrap();
    fs::copy("dotfiles/bash/.bashrc", root.join("dotfiles/bash/.bashrc")).unwrap();
    let yaml = fs::read_to_string("configs/cli.yaml")
        .unwrap()
        .replace("!enabled", "!disabled")
        .replace("purge: !disabled", "purge: !enabled")
        .replace("    - docker.io\n", "    - fake-package\n")
        .replace("distroCfg: true", "distroCfg: false")
        .replace("rustupCheck: true", "rustupCheck: false")
        .replace("  cargo: true", "  cargo: false");
    fs::write(root.join("cozydot.yaml"), yaml).unwrap();
    host.fake(
        "dpkg-query",
        r#"[ "${1:-}" = -W ] && [ "${2:-}" = '-f=${db:Status-Abbrev}\n' ] && printf 'ii \n'"#,
    );
    host.logging_fake("sudo");
    let output = Command::new(assert_cmd::cargo::cargo_bin!("cozydot"))
        .arg("apply")
        .env("HOME", &host.home)
        .env("USER", "tester")
        .env("LOG", &host.log)
        .env("PATH", format!("{}:/usr/bin:/bin", host.bin.display()))
        .env("XDG_CONFIG_HOME", host.home.join(".config"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let updated = fs::read_to_string(root.join("cozydot.yaml")).unwrap();
    assert!(updated.contains("purge: !disabled"));
    assert!(updated.contains("fake-package"));
    assert!(host.log().contains("sudo apt-get purge -qq fake-package"));
}

#[test]
fn bashrc_regular_file_is_replaced_but_symlink_is_preserved() {
    let host = Host::new();
    let source = host.home.join("dotfiles/bash/.bashrc");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(&source, "managed\n").unwrap();
    fs::write(host.home.join(".bashrc"), "old\n").unwrap();
    let cfg = Config::load(Path::new("configs/default.yaml")).unwrap();
    let mut step = planner::plan("check", &cfg, &platform("ubuntu"), &host.home)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let Step::Shell(command) = &mut step else {
        panic!("expected shell bridge");
    };
    *command.args.last_mut().unwrap() = host.home.join(".bashrc").display().to_string();
    host.run_ok(&step);
    assert_eq!(
        fs::read_to_string(host.home.join(".bashrc")).unwrap(),
        "managed\n"
    );
    fs::remove_file(host.home.join(".bashrc")).unwrap();
    let target = host.home.join("custom-bashrc");
    fs::write(&target, "custom\n").unwrap();
    symlink(&target, host.home.join(".bashrc")).unwrap();
    host.run_ok(&step);
    assert!(fs::symlink_metadata(host.home.join(".bashrc"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(target).unwrap(), "custom\n");
}

#[test]
fn ubuntu_and_mint_codecs_execute_apt_update_before_install() {
    for (distro, package) in [
        ("ubuntu", "ubuntu-restricted-extras"),
        ("linuxmint", "mint-meta-codecs"),
    ] {
        let host = Host::new();
        host.fake("dpkg-query", "exit 1");
        host.logging_fake("sudo");
        let step = step_containing(&plans("configs/default.yaml", "check", distro), package);
        host.run_ok(&step);
        let log = host.log();
        let update = log.find("apt-get update -qq").unwrap();
        let install = log.find(&format!("apt-get install -qq {package}")).unwrap();
        assert!(update < install, "{log}");
    }
}

#[test]
fn appimaged_active_and_inactive_branches_execute_against_fake_state() {
    for active in [true, false] {
        let host = Host::new();
        host.fake(
            "systemctl",
            if active {
                "printf 'systemctl %s\\n' \"$*\" >>\"$LOG\"; exit 0"
            } else {
                r#"printf 'systemctl %s\n' "$*" >>"$LOG"
if [ "$*" = '--user -q is-active appimaged' ]; then [ -f "$TMPDIR/appimaged-active" ]; fi"#
            },
        );
        host.fake(
            "sudo",
            r#"printf 'sudo %s\n' "$*" >>"$LOG"
if [ "$*" = 'apt-get install -qq libfuse2' ]; then touch "$TMPDIR/fuse-installed"; fi"#,
        );
        host.fake("apt-cache", "exit 1");
        host.fake("dpkg", if active { "exit 0" } else { "exit 1" });
        host.fake(
            "curl",
            r#"out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done
if [ -n "$out" ]; then printf '#!/bin/bash\n[ -f "$TMPDIR/fuse-installed" ] || exit 42\ntouch "$TMPDIR/appimaged-active"\nprintf "appimaged-run\\n" >>"$LOG"\n' >"$out"; else printf '{"assets":[{"name":"appimaged-x86_64.AppImage","browser_download_url":"https://example.test/appimaged.AppImage"}]}\n'; fi"#,
        );
        let step = step_containing(
            &plans("configs/default.yaml", "check", "ubuntu"),
            "workflow appimaged",
        );
        host.run_ok(&step);
        let log = host.log();
        assert_eq!(log.contains("appimaged-run"), !active);
        if !active {
            assert!(log.contains("systemctl --user daemon-reload"), "{log}");
            assert_eq!(
                log.matches("systemctl --user -q is-active appimaged")
                    .count(),
                2
            );
        }
    }
}

#[test]
fn snap_cleanup_parses_packages_and_handles_present_and_absent_snap() {
    for present in [true, false] {
        let host = Host::new();
        host.fake("systemctl", "exit 1");
        host.logging_fake("sudo");
        if present {
            host.fake(
                "snap",
                r#"printf 'snap %s\n' "$*" >>"$LOG"
if [ "${1:-}" = list ]; then
  printf 'Name Version Rev Tracking Publisher Notes\nfirefox 1 1 latest x -\ncore22 1 1 latest x -\nbare 1 1 latest x -\nsnapd 1 1 latest x -\n'
fi"#,
            );
        }
        let step = step_containing(
            &plans("configs/default.yaml", "check", "ubuntu"),
            "workflow snap-cleanup",
        );
        let output = host.run_with_path(&step, host.bin.display().to_string());
        assert!(
            output.status.success(),
            "{} failed: {}",
            step.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let log = host.log();
        assert_eq!(
            log.contains("snap remove --purge firefox"),
            present,
            "{log}"
        );
        assert_eq!(
            log.contains("sudo snap remove --purge core22"),
            present,
            "{log}"
        );
        assert_eq!(log.contains("apt-get purge -qq snapd"), present, "{log}");
    }
}

#[test]
fn group_membership_branches_only_modify_missing_membership() {
    for member in [true, false] {
        let host = Host::new();
        host.logging_fake("docker");
        let planned_user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        host.fake(
            "getent",
            &if member {
                format!("printf 'docker:x:999:{planned_user}\\n'")
            } else {
                "printf 'docker:x:999:\\n'".into()
            },
        );
        host.fake(
            "sudo",
            "printf 'sudo %s\\n' \"$*\" >>\"$LOG\"; if [ \"${1:-}\" = cat ]; then printf '{}\\n'; elif [ \"${1:-}\" = tee ]; then cat >/dev/null; fi",
        );
        host.logging_fake("newgrp");
        let step = step_containing(
            &plans("configs/full.yaml", "configure", "ubuntu"),
            "workflow docker-config",
        );
        host.run_ok(&step);
        assert_eq!(
            host.log().contains(&format!(
                "usermod -aG docker {}",
                std::env::var("USER").unwrap_or_else(|_| "user".into())
            )),
            !member,
            "{}",
            host.log()
        );
    }
}

#[test]
fn enabled_desktop_integrations_fail_or_install_dependencies_instead_of_silently_skipping() {
    let host = Host::new();
    host.logging_fake("sudo");
    host.run_ok(&Step::workflow(operations::Operation::GnomeDependencies));
    assert!(host.log().contains("dconf-cli"), "{}", host.log());

    let extension = Step::workflow(operations::Operation::GnomeExtension {
        extension: "example@test".into(),
    });
    assert!(!host.run(&extension).status.success());
    let vscode = Step::workflow(operations::Operation::VsCodeExtension {
        extension: "example.test".into(),
    });
    assert!(!host.run(&vscode).status.success());
    let terminal = Step::workflow(operations::Operation::GnomeTerminal {
        terminal: "definitely-missing-terminal".into(),
    });
    assert!(!host.run(&terminal).status.success());
}

#[test]
fn gnome_extension_present_enables_and_absent_installs() {
    let steps = plans("configs/full.yaml", "configure", "ubuntu");
    let step = step_containing(&steps, "workflow gnome-extension");
    let Step::Workflow(operations::Operation::GnomeExtension { ref extension }) = step else {
        panic!("expected GNOME extension workflow");
    };
    let extension = extension.clone();
    for present in [true, false] {
        let host = Host::new();
        host.fake(
            "gnome-extensions",
            &format!(
                r#"printf 'gnome-extensions %s\n' "$*" >>"$LOG"
if [ "${{1:-}}" = list ] && [ {present} = true ]; then printf '%s\n' '{extension}'; fi"#
            ),
        );
        host.fake("gnome-shell", "printf 'GNOME Shell 48.4\\n'");
        host.fake(
            "curl",
            r#"printf 'curl %s\n' "$*" >>"$LOG"; out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done; [ -z "$out" ] || : >"$out"; printf '{"shell_version_map":{"48":{"version":13},"50":{"version":23}}}\n'"#,
        );
        host.run_ok(&step);
        let log = host.log();
        assert!(log.contains("gnome-extensions enable"), "{log}");
        assert_eq!(log.contains("gnome-extensions install --force"), !present);
        if !present {
            assert!(log.contains(".v13.shell-extension.zip"), "{log}");
            assert!(!log.contains(".v23.shell-extension.zip"), "{log}");
        }
    }
}

#[test]
fn appimage_download_is_executable_and_existing_destination_is_idempotent() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done
if [ -n "$out" ]; then printf appimage >"$out"; else printf '{"assets":[{"name":"Obsidian-1.AppImage","browser_download_url":"https://example.test/Obsidian.AppImage"}]}'; fi"#,
    );
    let step = step_containing(
        &plans("configs/default.yaml", "install", "ubuntu"),
        "download-binary Obsidian.AppImage",
    );
    host.run_ok(&step);
    let destination = host.home.join("Applications/Obsidian.AppImage");
    assert!(destination.is_file());
    assert_ne!(
        fs::metadata(&destination).unwrap().permissions().mode() & 0o111,
        0
    );
    let first_log = host.log();
    host.run_ok(&step);
    assert_eq!(host.log(), first_log);

    fs::write(&destination, []).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o644)).unwrap();
    host.run_ok(&step);
    assert_ne!(host.log(), first_log);
    let repaired = fs::metadata(&destination).unwrap();
    assert!(repaired.len() > 0);
    assert_ne!(repaired.permissions().mode() & 0o111, 0);
}

#[test]
fn orphaned_debian_package_is_retried_instead_of_treated_as_installed() {
    let host = Host::new();
    let destination = host.home.join("Applications/git-credential-manager.deb");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, "stale").unwrap();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done
if [ -n "$out" ]; then printf new-deb >"$out"; else printf '{"assets":[{"name":"gcm-linux-x64-1.deb","browser_download_url":"https://example.test/gcm.deb"}]}'; fi"#,
    );
    host.logging_fake("sudo");
    let step = step_containing(
        &plans("configs/vm.yaml", "install", "debian"),
        "download-binary git-credential-manager.deb",
    );
    host.run_ok(&step);
    let log = host.log();
    assert!(log.contains("https://example.test/gcm.deb"), "{log}");
    assert!(log.contains("apt-get install -qq"), "{log}");
    assert!(!destination.exists());
}

#[test]
fn uv_installs_the_requested_python_series_without_parsing_display_output() {
    let host = Host::new();
    host.logging_fake("uv");
    let step = step_containing(
        &plans("configs/cli.yaml", "install", "ubuntu"),
        "workflow uv-install",
    );
    host.run_ok(&step);
    let log = host.log();
    assert!(!log.contains("uv self update"), "{log}");
    assert!(log.contains("uv python install 3.13"), "{log}");
    assert!(!log.contains("uv python list"), "{log}");
}

#[test]
fn fresh_uv_install_uses_a_deterministic_verified_destination() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done
cat >"$out" <<'INSTALL'
#!/bin/sh
mkdir -p "$UV_UNMANAGED_INSTALL"
cat >"$UV_UNMANAGED_INSTALL/uv" <<'UV'
#!/bin/sh
printf 'uv %s\n' "$*" >>"$LOG"
UV
chmod +x "$UV_UNMANAGED_INSTALL/uv"
INSTALL"#,
    );
    let step = step_containing(
        &plans("configs/cli.yaml", "install", "ubuntu"),
        "workflow uv-install",
    );
    host.run_ok(&step);
    let uv = host.home.join(".local/bin/uv");
    assert!(uv.is_file());
    assert_ne!(fs::metadata(uv).unwrap().permissions().mode() & 0o111, 0);
    assert!(
        host.log().contains("uv python install 3.13"),
        "{}",
        host.log()
    );
}

#[test]
fn nerdfont_skips_present_font_and_refreshes_after_installing_absent_font() {
    for present in [true, false] {
        let host = Host::new();
        host.fake(
            "fc-list",
            if present {
                r#"printf 'fc-list %s\n' "$*" >>"$LOG"
[ "$*" = ':family=GeistMono Nerd Font' ] && printf 'GeistMono Nerd Font\n'"#
            } else {
                r#"printf 'fc-list %s\n' "$*" >>"$LOG""#
            },
        );
        host.logging_fake("fc-cache");
        host.logging_fake("sudo");
        host.fake(
            "curl",
            "out=''; while [ \"$#\" -gt 0 ]; do if [ \"$1\" = -o ]; then out=$2; shift 2; else shift; fi; done; : >\"$out\"",
        );
        let step = step_containing(
            &plans("configs/default.yaml", "check", "ubuntu"),
            "workflow nerdfont",
        );
        host.run_ok(&step);
        let log = host.log();
        assert_eq!(log.contains("fc-cache -f"), !present, "{log}");
        if !present {
            assert!(
                log.contains("sudo rm -rf /usr/share/fonts/GeistMono"),
                "{log}"
            );
            host.run_ok(&step);
            assert_eq!(
                host.log()
                    .matches("sudo rm -rf /usr/share/fonts/GeistMono")
                    .count(),
                2
            );
        }
    }
}
