use cozydot::{
    config::Config,
    operations, planner,
    platform::{Architecture, Platform},
    runner::{command_exists_in, Condition, Step},
};
use std::{
    fs,
    io::Write,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
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
    root: PathBuf,
}

impl Host {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        let root = dir.path().join("root");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&root).unwrap();
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
            root,
        }
    }

    fn fake(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        let mut temporary = tempfile::NamedTempFile::new_in(&self.bin).unwrap();
        write!(temporary, "#!/bin/bash\nset -euo pipefail\n{body}\n").unwrap();
        temporary.flush().unwrap();
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o755)).unwrap();
        temporary.as_file().sync_all().unwrap();
        temporary.into_temp_path().persist(path).unwrap();
        fs::File::open(&self.bin).unwrap().sync_all().unwrap();
    }

    fn logging_fake(&self, name: &str) {
        self.fake(name, &format!("printf '%s %s\\n' {name} \"$*\" >>\"$LOG\""));
    }

    fn atomic_sudo(&self) {
        self.fake(
            "sudo",
            r#"printf 'sudo %s\n' "$*" >>"$LOG"
command=$1; shift
map_path() { case "$1" in /etc/*) printf '%s%s' "$ROOT" "$1" ;; *) printf '%s' "$1" ;; esac; }
failure=''; [ ! -f "$TMPDIR/publication-failure" ] || failure=$(cat "$TMPDIR/publication-failure")
case "$command" in
  install)
    if [ "${1:-}" = -d ]; then
      [ "$failure" != mkdir ] || exit 41
      destination=$(map_path "${!#}")
      mkdir -p "$destination"
      chmod 0755 "$destination"
    else
      [ "$failure" != stage ] || exit 42
      source=${@: -2:1}; destination=$(map_path "${!#}")
      /usr/bin/install -m 0644 -- "$source" "$destination"
    fi
    ;;
  sync)
    [ "$failure" != sync ] || exit 43
    /bin/sync -- "$(map_path "${!#}")"
    ;;
  test)
    [ "$#" -eq 3 ] && [ "$1" = '!' ] && [ "$2" = -d ] || exit 46
    [ ! -d "$(map_path "$3")" ]
    ;;
  mv)
    [ "$failure" != rename ] || exit 44
    [ "$#" -eq 4 ] && [ "$1" = -fT ] && [ "$2" = -- ] || exit 47
    source=$(map_path "$3"); destination=$(map_path "$4")
    /bin/mv -fT -- "$source" "$destination"
    ;;
  rm)
    /bin/rm -f -- "$(map_path "${!#}")"
    ;;
  *) exit 45 ;;
esac"#,
        );
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
                ("ROOT".into(), self.root.as_os_str().to_owned()),
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
            let mut command = Command::new("sh");
            if let Err(error) = result {
                command
                    .args(["-c", "printf '%s\\n' \"$ERROR\" >&2; exit 1"])
                    .env("ERROR", format!("{error:#}"));
            } else {
                command.args(["-c", "exit 0"]);
            }
            return command.output().unwrap();
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
            .env("ROOT", &self.root)
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

fn direct_step(
    format: operations::DirectPackageFormat,
    provides: &[&str],
    mode: operations::DirectPackageMode,
) -> Step {
    Step::workflow(operations::Operation::DirectPackage(
        operations::DirectPackageOperation::new(
            "sample",
            format,
            provides.iter().map(|value| (*value).into()).collect(),
            operations::GithubRepository::parse("owner/repo").unwrap(),
            Architecture::Amd64,
            operations::DirectPackageSelector::new(
                match format {
                    operations::DirectPackageFormat::Deb => "sample-amd64-*.deb",
                    operations::DirectPackageFormat::AppImage => "sample-amd64-*.AppImage",
                },
                vec![match format {
                    operations::DirectPackageFormat::Deb => "sample-amd64-debug-*.deb".into(),
                    operations::DirectPackageFormat::AppImage => {
                        "sample-amd64-debug-*.AppImage".into()
                    }
                }],
            )
            .unwrap(),
            mode,
        )
        .unwrap(),
    ))
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
        r#"printf 'fnm-installer-download\n' >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done
cat >"$out" <<'INSTALL'
#!/bin/bash
mkdir -p "$XDG_DATA_HOME/fnm"
cat >"$XDG_DATA_HOME/fnm/fnm" <<'FNM'
#!/bin/bash
printf 'fnm %s\n' "$*" >>"$LOG"
case "$1" in
 env) printf 'export FNM_MULTISHELL_PATH="%s/multishell"\nexport PATH="%s:$PATH"\n' "$XDG_DATA_HOME/fnm" "$XDG_DATA_HOME/fnm" ;;
 install) [ -n "${FNM_MULTISHELL_PATH:-}" ] || { printf 'missing fnm environment\n' >&2; exit 1; }; touch "$TMPDIR/fnm-node-installed" ;;
 use) [ -f "$TMPDIR/fnm-node-installed" ] ;;
 current) if [ -f "$TMPDIR/fnm-node-installed" ]; then printf 'v22.1.0\n'; else printf 'none\n'; fi ;;
 default) if [ "$#" -gt 1 ]; then touch "$TMPDIR/fnm-default-set"; elif [ -f "$TMPDIR/fnm-default-set" ]; then printf 'v22.1.0\n'; fi ;;
esac
FNM
cat >"$XDG_DATA_HOME/fnm/npm" <<'NPM'
#!/bin/bash
[ -n "${FNM_MULTISHELL_PATH:-}" ] || { printf 'missing fnm environment\n' >&2; exit 1; }
printf 'npm %s\n' "$*" >>"$LOG"
if [ "${1:-}" = list ]; then
  [ -f "$TMPDIR/npm-opencode-installed" ] || exit 1
fi
if [ "${1:-}" = install ]; then touch "$TMPDIR/npm-opencode-installed"; fi
NPM
chmod +x "$XDG_DATA_HOME/fnm/fnm" "$XDG_DATA_HOME/fnm/npm"
INSTALL"#,
    );
    let step = step_containing(
        &plans("configs/cli.yaml", "install", "ubuntu"),
        "workflow node-install",
    );
    host.run_ok(&step);
    host.run_ok(&step);
    let log = host.log();
    assert_eq!(log.matches("fnm install --lts --use").count(), 1, "{log}");
    assert!(log.contains("fnm use v22.1.0"));
    assert_eq!(log.matches("fnm-installer-download").count(), 1, "{log}");
    assert_eq!(log.matches("npm install --global").count(), 1, "{log}");
    assert!(log.contains("npm list --global --depth=0 opencode-ai"));
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
fn apt_packages_batch_only_missing_packages_into_one_install() {
    let host = Host::new();
    host.fake(
        "dpkg-query",
        r#"printf 'dpkg-query %s\n' "$*" >>"$LOG"
shift 3
status=0
for package in "$@"; do
  case "$package" in bash|curl) printf '%s\tii \n' "$package" ;; *) status=1 ;; esac
done
exit "$status""#,
    );
    host.logging_fake("sudo");
    let step = Step::workflow(operations::Operation::AptPackages {
        packages: vec![
            "bash".into(),
            "missing-one".into(),
            "curl".into(),
            "missing-two".into(),
        ],
    });
    host.run_ok(&step);
    let log = host.log();
    assert_eq!(log.matches("dpkg-query ").count(), 1, "{log}");
    assert_eq!(
        log.matches("sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --")
            .count(),
        1,
        "{log}"
    );
    assert!(
        log.contains(
            "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq -- missing-one+ missing-two+"
        ),
        "{log}"
    );
}

#[test]
fn apt_refresh_has_one_exact_fixed_command() {
    let host = Host::new();
    host.logging_fake("sudo");
    host.run_ok(&Step::workflow(operations::Operation::AptMetadataRefresh));
    assert_eq!(host.log(), "sudo apt-get update -qq\n");
}

#[test]
fn apt_bootstrap_all_installed_is_a_query_only_noop() {
    let host = Host::new();
    host.fake(
        "dpkg-query",
        r#"printf 'dpkg-query %s\n' "$*" >>"$LOG"
shift 3
for package in "$@"; do printf '%s\tii \n' "$package"; done"#,
    );
    host.logging_fake("sudo");
    host.run_ok(&Step::workflow(
        operations::Operation::AptBootstrapPackages {
            packages: vec!["ca-certificates".into(), "curl".into()],
        },
    ));
    assert_eq!(
        host.log(),
        "dpkg-query -W -f=${Package}\\t${db:Status-Abbrev}\\n -- ca-certificates curl\n"
    );
}

#[test]
fn apt_bootstrap_refreshes_once_installs_missing_in_order_and_reapplies_as_noop() {
    let host = Host::new();
    host.fake(
        "dpkg-query",
        r#"printf 'dpkg-query %s\n' "$*" >>"$LOG"
shift 3
status=0
for package in "$@"; do
  if [ "$package" = ca-certificates ] || [ -f "$TMPDIR/bootstrapped" ]; then
    printf '%s\tii \n' "$package"
  else
    status=1
  fi
done
exit "$status""#,
    );
    host.fake(
        "sudo",
        r#"printf 'sudo %s\n' "$*" >>"$LOG"
if [ "$*" = 'DEBIAN_FRONTEND=noninteractive apt-get install -y -qq -- curl+ gnupg+ flatpak+' ]; then
  touch "$TMPDIR/bootstrapped"
fi"#,
    );
    let step = Step::workflow(operations::Operation::AptBootstrapPackages {
        packages: vec![
            "ca-certificates".into(),
            "curl".into(),
            "gnupg".into(),
            "flatpak".into(),
        ],
    });
    host.run_ok(&step);
    host.run_ok(&step);
    let log = host.log();
    assert_eq!(log.matches("dpkg-query ").count(), 2, "{log}");
    assert_eq!(log.matches("sudo apt-get update -qq\n").count(), 1, "{log}");
    assert_eq!(
        log.matches(
            "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq -- curl+ gnupg+ flatpak+\n"
        )
        .count(),
        1,
        "{log}"
    );
}

#[test]
fn apt_bootstrap_stops_on_query_refresh_and_install_failures() {
    for (failure, body, expected_log) in [
        (
            "query",
            "printf 'fatal dpkg state\\n' >&2; exit 2",
            "dpkg-query ",
        ),
        ("refresh", "exit 1", "dpkg-query "),
        ("install", "exit 1", "dpkg-query "),
    ] {
        let host = Host::new();
        host.fake(
            "dpkg-query",
            if failure == "query" {
                body
            } else {
                "printf 'dpkg-query %s\\n' \"$*\" >>\"$LOG\"; exit 1"
            },
        );
        host.fake(
            "sudo",
            &format!(
                r#"printf 'sudo %s\n' "$*" >>"$LOG"
case {failure} in
  refresh) [ "$*" != 'apt-get update -qq' ] ;;
  install) [ "$*" != 'DEBIAN_FRONTEND=noninteractive apt-get install -y -qq -- curl+' ] ;;
  *) {body} ;;
esac"#
            ),
        );
        let output = host.run(&Step::workflow(
            operations::Operation::AptBootstrapPackages {
                packages: vec!["curl".into()],
            },
        ));
        assert!(!output.status.success(), "{failure}");
        let log = host.log();
        if failure == "query" {
            assert!(log.is_empty(), "{log}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("fatal dpkg state"));
        } else {
            assert!(log.starts_with(expected_log), "{failure}: {log}");
            assert_eq!(log.matches("sudo apt-get update -qq\n").count(), 1, "{log}");
            assert_eq!(
                log.matches("sudo DEBIAN_FRONTEND=noninteractive apt-get install")
                    .count(),
                usize::from(failure == "install"),
                "{log}"
            );
        }
    }
}

#[test]
fn apt_bootstrap_rejects_empty_duplicate_and_invalid_names_before_query() {
    for packages in [
        vec![],
        vec!["curl".into(), "curl".into()],
        vec!["curl;reboot".into()],
        vec!["libc6:amd64".into()],
    ] {
        let host = Host::new();
        host.logging_fake("dpkg-query");
        host.logging_fake("sudo");
        let output = host.run(&Step::workflow(
            operations::Operation::AptBootstrapPackages { packages },
        ));
        assert!(!output.status.success());
        assert!(host.log().is_empty(), "{}", host.log());
    }
}

#[test]
fn apt_install_uses_dpkg_status_and_never_refreshes() {
    let host = Host::new();
    host.fake(
        "dpkg-query",
        r#"printf 'dpkg-query %s\n' "$*" >>"$LOG"
shift 3
status=0
for package in "$@"; do
  case "$package" in
    installed) printf '%s\tii \n' "$package" ;;
    held) printf '%s\thi \n' "$package" ;;
    residual) printf '%s\trc \n' "$package" ;;
    *) status=1 ;;
  esac
done
exit "$status""#,
    );
    host.logging_fake("sudo");
    let step = Step::workflow(operations::Operation::AptPackages {
        packages: vec![
            "installed".into(),
            "held".into(),
            "residual".into(),
            "absent".into(),
        ],
    });
    host.run_ok(&step);
    let log = host.log();
    assert_eq!(log.matches("dpkg-query ").count(), 1, "{log}");
    assert!(
        log.contains("dpkg-query -W -f=${Package}\\t${db:Status-Abbrev}\\n -- installed held residual absent\n"),
        "{log}"
    );
    assert!(
        log.contains(
            "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq -- residual+ absent+\n"
        ),
        "{log}"
    );
    assert!(!log.contains(" update "));
}

#[test]
fn apt_purge_batches_installed_targets_and_second_run_is_noop() {
    let host = Host::new();
    host.fake(
        "dpkg-query",
        r#"printf 'dpkg-query %s\n' "$*" >>"$LOG"
[ ! -f "$TMPDIR/purged" ] || exit 1
shift 3
status=0
for package in "$@"; do
  case "$package" in
    installed) printf '%s\tii \n' "$package" ;;
    held) printf '%s\thi \n' "$package" ;;
    residual) printf '%s\trc \n' "$package" ;;
    *) status=1 ;;
  esac
done
exit "$status""#,
    );
    host.fake(
        "sudo",
        r#"printf 'sudo %s\n' "$*" >>"$LOG"
[ "$*" = 'DEBIAN_FRONTEND=noninteractive apt-get purge -y -qq -- installed- held-' ] && touch "$TMPDIR/purged""#,
    );
    let step = Step::workflow(operations::Operation::AptPurge {
        packages: vec![
            "absent".into(),
            "installed".into(),
            "residual".into(),
            "held".into(),
        ],
    });
    host.run_ok(&step);
    host.run_ok(&step);
    let log = host.log();
    assert_eq!(log.matches("dpkg-query ").count(), 2, "{log}");
    assert!(
        log.contains("dpkg-query -W -f=${Package}\\t${db:Status-Abbrev}\\n -- absent installed residual held\n"),
        "{log}"
    );
    assert_eq!(
        log.matches("sudo DEBIAN_FRONTEND=noninteractive apt-get purge")
            .count(),
        1,
        "{log}"
    );
    assert!(
        log.contains(
            "sudo DEBIAN_FRONTEND=noninteractive apt-get purge -y -qq -- installed- held-\n"
        ),
        "{log}"
    );
}

#[test]
fn apt_package_state_fails_closed_for_fatal_status_signal_and_invalid_output() {
    for (name, packages, body, error) in [
        (
            "exit-2",
            &["package"][..],
            "printf 'fatal database error\\n' >&2; exit 2",
            "APT package state query: dpkg-query failed (exit status: 2): fatal database error",
        ),
        (
            "signal",
            &["package"][..],
            "printf 'terminated query\\n' >&2; kill -TERM $$",
            "APT package state query: dpkg-query failed (signal:",
        ),
        (
            "non-UTF-8",
            &["package"][..],
            "printf '\\377'",
            "dpkg-query returned non-UTF-8 package state",
        ),
        (
            "missing-tab",
            &["package"][..],
            "printf 'package ii \\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "empty-package",
            &["package"][..],
            "printf '\\tii \\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "two-byte-status",
            &["package"][..],
            "printf 'package\\tii\\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "four-byte-status",
            &["package"][..],
            "printf 'package\\tii x\\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "extra-tab",
            &["package"][..],
            "printf 'package\\tii \\textra\\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "undocumented-x-status",
            &["package"][..],
            "printf 'package\\txi \\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "invalid-desired",
            &["package"][..],
            "printf 'package\\tzi \\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "invalid-package-status",
            &["package"][..],
            "printf 'package\\tix \\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "invalid-error",
            &["package"][..],
            "printf 'package\\tiiX\\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "trailing-text",
            &["package"][..],
            "printf 'package\\tii junk\\n'",
            "dpkg-query returned malformed package state",
        ),
        (
            "duplicate-request",
            &["package", "package"][..],
            "printf 'package\\tii \\n'",
            "duplicate requested package",
        ),
        (
            "duplicate-record",
            &["package"][..],
            "printf 'package\\tii \\npackage\\tii \\n'",
            "duplicate package record",
        ),
        (
            "unrequested-record",
            &["package"][..],
            "printf 'other\\tii \\n'",
            "unrequested package record",
        ),
        (
            "exit-zero-empty",
            &["package"][..],
            ":",
            "incomplete package state; missing records for: package",
        ),
        (
            "exit-zero-subset",
            &["package", "missing"][..],
            "printf 'package\\tii \\n'",
            "incomplete package state; missing records for: missing",
        ),
    ] {
        for operation in ["install", "purge"] {
            let host = Host::new();
            host.fake(
                "dpkg-query",
                &format!("printf 'dpkg-query %s\\n' \"$*\" >>\"$LOG\"; {body}"),
            );
            host.logging_fake("sudo");
            let step = match operation {
                "install" => Step::workflow(operations::Operation::AptPackages {
                    packages: packages.iter().map(|package| (*package).into()).collect(),
                }),
                "purge" => Step::workflow(operations::Operation::AptPurge {
                    packages: packages.iter().map(|package| (*package).into()).collect(),
                }),
                _ => unreachable!(),
            };
            let output = host.run(&step);
            assert!(!output.status.success(), "{name} {operation}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(error), "{name} {operation}: {stderr}");
            if name == "signal" {
                assert!(stderr.contains("terminated query"), "{stderr}");
            }
            let log = host.log();
            if name == "duplicate-request" {
                assert!(log.is_empty(), "{name} {operation}: {log}");
            } else {
                assert!(
                    log.starts_with("dpkg-query -W "),
                    "{name} {operation}: {log}"
                );
            }
            assert!(!log.contains("sudo "), "{name} {operation}: {log}");
        }
    }
}

#[test]
fn apt_package_state_accepts_exit_one_with_mixed_output() {
    for operation in ["install", "purge"] {
        let host = Host::new();
        host.fake(
            "dpkg-query",
            r#"printf 'dpkg-query %s\n' "$*" >>"$LOG"
printf 'installed\tii \nresidual\trc \n'
exit 1"#,
        );
        host.logging_fake("sudo");
        let step = match operation {
            "install" => Step::workflow(operations::Operation::AptPackages {
                packages: vec!["installed".into(), "residual".into(), "absent".into()],
            }),
            "purge" => Step::workflow(operations::Operation::AptPurge {
                packages: vec!["installed".into(), "residual".into(), "absent".into()],
            }),
            _ => unreachable!(),
        };
        host.run_ok(&step);
        let log = host.log();
        let expected = match operation {
            "install" => {
                "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq -- residual+ absent+\n"
            }
            "purge" => "sudo DEBIAN_FRONTEND=noninteractive apt-get purge -y -qq -- installed-\n",
            _ => unreachable!(),
        };
        assert!(log.contains(expected), "{operation}: {log}");
    }
}

#[test]
fn apt_package_state_accepts_exit_one_with_empty_output() {
    for operation in ["install", "purge"] {
        let host = Host::new();
        host.fake(
            "dpkg-query",
            r#"printf 'dpkg-query %s\n' "$*" >>"$LOG"
exit 1"#,
        );
        host.logging_fake("sudo");
        let step = match operation {
            "install" => Step::workflow(operations::Operation::AptPackages {
                packages: vec!["first".into(), "second".into()],
            }),
            "purge" => Step::workflow(operations::Operation::AptPurge {
                packages: vec!["first".into(), "second".into()],
            }),
            _ => unreachable!(),
        };
        host.run_ok(&step);
        let log = host.log();
        match operation {
            "install" => assert!(
                log.contains(
                    "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq -- first+ second+\n"
                ),
                "{log}"
            ),
            "purge" => assert!(!log.contains("sudo "), "{log}"),
            _ => unreachable!(),
        }
    }
}

#[test]
fn apt_mutations_force_actions_for_ordinary_plus_and_minus_package_names() {
    let packages = vec!["ordinary".into(), "name+".into(), "name-".into()];

    for operation in ["install", "bootstrap"] {
        let host = Host::new();
        host.fake(
            "dpkg-query",
            "printf 'dpkg-query %s\\n' \"$*\" >>\"$LOG\"; exit 1",
        );
        host.logging_fake("sudo");
        let step = match operation {
            "install" => Step::workflow(operations::Operation::AptPackages {
                packages: packages.clone(),
            }),
            "bootstrap" => Step::workflow(operations::Operation::AptBootstrapPackages {
                packages: packages.clone(),
            }),
            _ => unreachable!(),
        };
        host.run_ok(&step);
        let refresh = if operation == "bootstrap" {
            "sudo apt-get update -qq\n"
        } else {
            ""
        };
        assert_eq!(
            host.log(),
            format!(
                "dpkg-query -W -f=${{Package}}\\t${{db:Status-Abbrev}}\\n -- ordinary name+ name-\n{refresh}sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq -- ordinary+ name++ name-+\n"
            )
        );
    }

    let host = Host::new();
    host.fake(
        "dpkg-query",
        r#"printf 'dpkg-query %s\n' "$*" >>"$LOG"
shift 3
for package in "$@"; do printf '%s\tii \n' "$package"; done"#,
    );
    host.logging_fake("sudo");
    host.run_ok(&Step::workflow(operations::Operation::AptPurge {
        packages,
    }));
    assert_eq!(
        host.log(),
        "dpkg-query -W -f=${Package}\\t${db:Status-Abbrev}\\n -- ordinary name+ name-\nsudo DEBIAN_FRONTEND=noninteractive apt-get purge -y -qq -- ordinary- name+- name--\n"
    );
}

#[test]
fn apt_upgrade_policies_have_fixed_order_and_stop_on_failure() {
    let standard = Host::new();
    standard.logging_fake("sudo");
    standard.run_ok(&Step::workflow(operations::Operation::AptUpgrade {
        policy: operations::AptUpgradePolicy::Standard,
    }));
    assert_eq!(
        standard.log(),
        "sudo DEBIAN_FRONTEND=noninteractive apt-get upgrade -y -qq --\n"
    );

    let full = Host::new();
    full.logging_fake("sudo");
    full.run_ok(&Step::workflow(operations::Operation::AptUpgrade {
        policy: operations::AptUpgradePolicy::Full,
    }));
    assert_eq!(
        full.log(),
        "sudo DEBIAN_FRONTEND=noninteractive apt-get full-upgrade -y -qq --\nsudo DEBIAN_FRONTEND=noninteractive apt-get autoremove --purge -y -qq --\n"
    );

    let failing = Host::new();
    failing.fake(
        "sudo",
        r#"printf 'sudo %s\n' "$*" >>"$LOG"
[ "$*" != 'DEBIAN_FRONTEND=noninteractive apt-get full-upgrade -y -qq --' ]"#,
    );
    let output = failing.run(&Step::workflow(operations::Operation::AptUpgrade {
        policy: operations::AptUpgradePolicy::Full,
    }));
    assert!(!output.status.success());
    assert_eq!(
        failing.log(),
        "sudo DEBIAN_FRONTEND=noninteractive apt-get full-upgrade -y -qq --\n"
    );
    assert!(!failing.log().contains("update"));
}

#[test]
fn flatpak_flathub_absent_adds_fixed_remote_and_second_apply_is_query_only() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user remotes --show-disabled --columns=name,url,options')
    [ ! -f "$TMPDIR/flathub-added" ] || printf 'flathub\thttps://dl.flathub.org/repo/\t\n'
    ;;
  '--user remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo')
    touch "$TMPDIR/flathub-added"
    ;;
  *) exit 42 ;;
esac"#,
    );
    let step = Step::workflow(operations::Operation::FlatpakEnsureFlathub);
    host.run_ok(&step);
    host.run_ok(&step);
    assert_eq!(
        host.log(),
        "flatpak --user remotes --show-disabled --columns=name,url,options\nflatpak --user remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo\nflatpak --user remotes --show-disabled --columns=name,url,options\n"
    );
}

#[test]
fn flatpak_flathub_canonical_usable_remote_is_query_only() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
printf 'flathub\thttps://dl.flathub.org/repo/\tgpg-verify-summary,collection-id=org.flathub.Stable\n'"#,
    );
    host.run_ok(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
    assert_eq!(
        host.log(),
        "flatpak --user remotes --show-disabled --columns=name,url,options\n"
    );
}

#[test]
fn flatpak_flathub_rejects_wrong_identity_and_unusable_options() {
    for (name, record) in [
        ("wrong-url", "flathub\thttps://example.invalid/repo\t"),
        (
            "disabled",
            "flathub\thttps://dl.flathub.org/repo/\tdisabled",
        ),
        (
            "no-gpg-verification",
            "flathub\thttps://dl.flathub.org/repo/\tno-gpg-verify",
        ),
        (
            "no-enumeration",
            "flathub\thttps://dl.flathub.org/repo/\tno-enumerate",
        ),
        (
            "no-dependencies",
            "flathub\thttps://dl.flathub.org/repo/\tno-deps",
        ),
        (
            "no-use-for-dependencies",
            "flathub\thttps://dl.flathub.org/repo/\tno-use-for-deps",
        ),
    ] {
        let host = Host::new();
        host.fake("flatpak", &format!("printf '{record}\\n'"));
        let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
        assert!(!output.status.success(), "{name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Flathub remote mismatch"),
            "{name}: {stderr}"
        );
        assert!(stderr.contains("Repair or remove"), "{name}: {stderr}");
    }
}

#[test]
fn flatpak_flathub_fails_closed_on_query_and_malformed_remote_state() {
    for (name, body, error) in [
        (
            "fatal",
            "printf 'remote query failed\\n' >&2; exit 43",
            "remote query failed",
        ),
        (
            "non-UTF-8",
            "printf '\\377'",
            "non-UTF-8 per-user remote state",
        ),
        (
            "missing-column",
            "printf 'flathub\\thttps://dl.flathub.org/repo/\\n'",
            "malformed per-user remote state",
        ),
        (
            "blank-record",
            "printf 'other\\thttps://example.test/repo\\t\\n\\nflathub\\thttps://dl.flathub.org/repo/\\t\\n'",
            "malformed per-user remote state",
        ),
        (
            "duplicate-name",
            "printf 'flathub\\thttps://dl.flathub.org/repo/\\t\\nflathub\\thttps://dl.flathub.org/repo/\\t\\n'",
            "duplicate per-user remote name",
        ),
    ] {
        let host = Host::new();
        host.fake("flatpak", body);
        let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
        assert!(!output.status.success(), "{name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(error), "{name}: {stderr}");
    }
}

#[test]
fn flatpak_ensure_apps_batches_missing_refs_in_config_order_and_second_apply_is_noop() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user list --app --columns=application')
    printf 'org.example.Installed\n'
    [ ! -f "$TMPDIR/flatpak-installed" ] || printf 'com.example.First\nio.example.Second\n'
    ;;
  '--user install --app --noninteractive -y flathub -- com.example.First io.example.Second')
    touch "$TMPDIR/flatpak-installed"
    ;;
  *) exit 42 ;;
esac"#,
    );
    let step = Step::workflow(operations::Operation::FlatpakEnsureApps {
        refs: vec![
            "com.example.First".into(),
            "org.example.Installed".into(),
            "io.example.Second".into(),
        ],
    });
    host.run_ok(&step);
    host.run_ok(&step);
    let log = host.log();
    assert_eq!(
        log.matches("flatpak --user list --app --columns=application\n")
            .count(),
        2,
        "{log}"
    );
    assert_eq!(
        log.matches("flatpak --user install --app --noninteractive -y flathub -- com.example.First io.example.Second\n")
            .count(),
        1,
        "{log}"
    );
}

#[test]
fn flatpak_repeated_branch_and_architecture_ids_count_as_installed() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
printf 'com.example.First\ncom.example.First\norg.example.Second\norg.example.Second\n'"#,
    );
    host.run_ok(&Step::workflow(operations::Operation::FlatpakEnsureApps {
        refs: vec!["com.example.First".into(), "org.example.Second".into()],
    }));
    assert_eq!(
        host.log(),
        "flatpak --user list --app --columns=application\n"
    );
}

#[test]
fn flatpak_ensure_apps_fails_closed_on_list_errors_and_malformed_state() {
    for (name, body, error) in [
        (
            "fatal",
            "printf 'list failed\\n' >&2; exit 43",
            "list failed",
        ),
        (
            "non-UTF-8",
            "printf '\\377'",
            "non-UTF-8 installed application state",
        ),
        (
            "malformed",
            "printf 'not-an-app\\n'",
            "malformed installed application ID",
        ),
        (
            "blank-interior-record",
            "printf 'com.example.App\\n\\norg.example.Other\\n'",
            "malformed installed application ID",
        ),
    ] {
        let host = Host::new();
        host.fake(
            "flatpak",
            &format!("printf 'flatpak %s\\n' \"$*\" >>\"$LOG\"; {body}"),
        );
        let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureApps {
            refs: vec!["com.example.App".into()],
        }));
        assert!(!output.status.success(), "{name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(error), "{name}: {stderr}");
        assert_eq!(
            host.log(),
            "flatpak --user list --app --columns=application\n",
            "{name}"
        );
    }
}

#[test]
fn flatpak_operations_reject_empty_duplicate_and_invalid_refs_before_execution() {
    for operation in ["ensure", "update"] {
        for refs in [
            vec![],
            vec!["com.example.App".into(), "com.example.App".into()],
            vec!["org.example.App".into(), "--system".into()],
            vec!["org.example.bad-id".into()],
        ] {
            let host = Host::new();
            host.logging_fake("flatpak");
            let step = match operation {
                "ensure" => Step::workflow(operations::Operation::FlatpakEnsureApps { refs }),
                "update" => Step::workflow(operations::Operation::FlatpakUpdateApps { refs }),
                _ => unreachable!(),
            };
            let output = host.run(&step);
            assert!(!output.status.success(), "{operation}");
            assert!(host.log().is_empty(), "{operation}: {}", host.log());
        }
    }
}

#[test]
fn flatpak_update_targets_configured_apps_with_dependencies_and_eol_replacements_allowed() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
[ "$*" = '--user update --app --noninteractive -y -- org.example.First com.example.Second' ]"#,
    );
    let step = Step::workflow(operations::Operation::FlatpakUpdateApps {
        refs: vec!["org.example.First".into(), "com.example.Second".into()],
    });
    host.run_ok(&step);
    host.run_ok(&step);
    assert_eq!(
        host.log(),
        "flatpak --user update --app --noninteractive -y -- org.example.First com.example.Second\nflatpak --user update --app --noninteractive -y -- org.example.First com.example.Second\n"
    );
}

#[test]
fn flatpak_install_and_update_propagate_mutation_errors() {
    for operation in ["install", "update"] {
        let host = Host::new();
        host.fake(
            "flatpak",
            &format!(
                r#"printf 'flatpak %s\n' "$*" >>"$LOG"
if [ "$2" = list ]; then exit 0; fi
printf '{operation} failed\n' >&2
exit 44"#
            ),
        );
        let step = match operation {
            "install" => Step::workflow(operations::Operation::FlatpakEnsureApps {
                refs: vec!["com.example.App".into()],
            }),
            "update" => Step::workflow(operations::Operation::FlatpakUpdateApps {
                refs: vec!["com.example.App".into()],
            }),
            _ => unreachable!(),
        };
        let output = host.run(&step);
        assert!(!output.status.success(), "{operation}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(&format!("{operation} failed")),
            "{operation}"
        );
    }
}

#[test]
fn apt_source_publication_is_atomic_retry_safe_and_mode_0644() {
    let host = Host::new();
    host.atomic_sudo();
    let destination = host.root.join("etc/apt/sources.list.d/vendor.list");
    let first = Step::workflow(operations::Operation::AptSource {
        destination: "/etc/apt/sources.list.d/vendor.list".into(),
        contents: "deb [arch=amd64] https://example.test stable main\n".into(),
    });
    host.run_ok(&first);
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"deb [arch=amd64] https://example.test stable main\n"
    );
    assert_eq!(
        fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
        0o644
    );
    let publication_log = host.log();
    assert!(
        publication_log
            .contains("sudo install -d -o root -g root -m 0755 -- /etc/apt/sources.list.d"),
        "{publication_log}"
    );
    assert!(
        publication_log.contains("sudo install -o root -g root -m 0644 --"),
        "{publication_log}"
    );
    assert!(publication_log.contains("sudo sync -- /etc/apt/sources.list.d/.vendor.list."));
    assert!(publication_log.contains("sudo mv -fT -- /etc/apt/sources.list.d/.vendor.list."));
    let rename = publication_log
        .lines()
        .find(|line| line.starts_with("sudo mv "))
        .unwrap();
    let rename_args = rename.split_whitespace().collect::<Vec<_>>();
    assert_eq!(rename_args.len(), 6, "{rename}");
    assert_eq!(rename_args[0..4], ["sudo", "mv", "-fT", "--"]);
    assert!(
        rename_args[4].starts_with("/etc/apt/sources.list.d/.vendor.list."),
        "{rename}"
    );
    assert_eq!(rename_args[5], "/etc/apt/sources.list.d/vendor.list");
    host.run_ok(&first);
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"deb [arch=amd64] https://example.test stable main\n"
    );

    let replacement = Step::workflow(operations::Operation::AptSource {
        destination: "/etc/apt/sources.list.d/vendor.list".into(),
        contents: "deb [arch=amd64] https://example.test testing main\n".into(),
    });
    host.run_ok(&replacement);
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"deb [arch=amd64] https://example.test testing main\n"
    );

    for failure in ["mkdir", "stage", "sync", "rename"] {
        fs::write(host._dir.path().join("tmp/publication-failure"), failure).unwrap();
        assert!(!host.run(&first).status.success(), "{failure}");
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"deb [arch=amd64] https://example.test testing main\n",
            "{failure}"
        );
        assert_eq!(
            fs::read_dir(destination.parent().unwrap()).unwrap().count(),
            1,
            "{failure}"
        );
    }
}

#[test]
fn apt_source_refuses_directory_and_directory_symlink_destinations() {
    for kind in ["directory", "directory-symlink"] {
        let host = Host::new();
        host.atomic_sudo();
        let parent = host.root.join("etc/apt/sources.list.d");
        let destination = parent.join("vendor.list");
        fs::create_dir_all(&parent).unwrap();
        let preserved_directory = if kind == "directory" {
            fs::create_dir(&destination).unwrap();
            destination.clone()
        } else {
            let target = parent.join("vendor-target");
            fs::create_dir(&target).unwrap();
            symlink("vendor-target", &destination).unwrap();
            target
        };
        fs::write(preserved_directory.join("marker"), b"old").unwrap();
        let inode = fs::metadata(&preserved_directory).unwrap().ino();
        let destination_inode = fs::symlink_metadata(&destination).unwrap().ino();
        let link_target = fs::read_link(&destination).ok();
        let step = Step::workflow(operations::Operation::AptSource {
            destination: "/etc/apt/sources.list.d/vendor.list".into(),
            contents: "deb https://example.test stable main\n".into(),
        });

        let output = host.run(&step);
        assert!(!output.status.success(), "{kind}");
        assert_eq!(
            fs::metadata(&preserved_directory).unwrap().ino(),
            inode,
            "{kind}"
        );
        assert_eq!(
            fs::symlink_metadata(&destination).unwrap().ino(),
            destination_inode,
            "{kind}"
        );
        assert_eq!(
            fs::read(preserved_directory.join("marker")).unwrap(),
            b"old",
            "{kind}"
        );
        assert_eq!(fs::read_link(&destination).ok(), link_target, "{kind}");
        let entries = fs::read_dir(&parent)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert!(
            entries
                .iter()
                .all(|name| !name.to_string_lossy().starts_with(".vendor.list.")),
            "{kind}: {entries:?}"
        );
        let log = host.log();
        assert!(
            log.contains("sudo rm -f -- /etc/apt/sources.list.d/.vendor.list."),
            "{kind}: {log}"
        );
        assert!(!log.contains("sudo mv "), "{kind}: {log}");
    }
}

#[test]
fn apt_source_rejects_unsafe_destination_and_content_before_mutation() {
    let host = Host::new();
    host.logging_fake("sudo");
    let invalid_destinations = [
        "etc/apt/sources.list.d/vendor.list",
        "/etc/apt/sources.list.d/../vendor.list",
        "/etc/apt/vendor.list",
        "/etc/apt/sources.list.d/vendor.sources",
        "/etc/apt/sources.list.d/vendor.list\0suffix",
    ];
    for destination in invalid_destinations {
        let step = Step::workflow(operations::Operation::AptSource {
            destination: destination.into(),
            contents: "deb https://example.test stable main\n".into(),
        });
        assert!(!host.run(&step).status.success(), "{destination:?}");
    }
    for contents in [
        "deb https://example.test stable main",
        "deb https://example.test stable main\0\n",
        "deb https://example.test stable main\ndeb https://example.test stable extras\n",
    ] {
        let step = Step::workflow(operations::Operation::AptSource {
            destination: "/etc/apt/sources.list.d/vendor.list".into(),
            contents: contents.into(),
        });
        assert!(!host.run(&step).status.success(), "{contents:?}");
    }
    assert!(host.log().is_empty(), "{}", host.log());
}

fn configure_key_fakes(host: &Host) {
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do if [ "$1" = --output ]; then out=$2; shift 2; else shift; fi; done
kind=$(cat "$TMPDIR/key-input")
if [ "$kind" = interrupted ]; then printf partial >"$out"; exit 52; fi
printf '%s' "$kind" >"$out""#,
    );
    host.fake(
        "gpg",
        r#"printf 'gpg %s\n' "$*" >>"$LOG"
if [[ " $* " = *' --dearmor '* ]]; then
  out=''; input=${!#}
  while [ "$#" -gt 0 ]; do if [ "$1" = --output ]; then out=$2; shift 2; else shift; fi; done
  kind=$(cat "$input")
  case "$kind" in
    armored) printf normalized-armored >"$out" ;;
    binary|validation-failure) printf normalized-binary >"$out" ;;
    empty) : >"$out" ;;
    conversion-failure|malformed) exit 53 ;;
    *) exit 54 ;;
  esac
else
  [ "$(cat "$TMPDIR/key-input")" != validation-failure ] || exit 55
  printf 'pub:-:2048:1:0123456789ABCDEF:0:0::::::\n'
fi"#,
    );
    host.atomic_sudo();
}

#[test]
fn repository_key_normalizes_armored_and_binary_then_publishes_atomically() {
    let host = Host::new();
    configure_key_fakes(&host);
    let input = host._dir.path().join("tmp/key-input");
    let destination = host.root.join("etc/apt/keyrings/vendor.gpg");
    let step = Step::workflow(operations::Operation::RepositoryKey {
        url: "https://example.test/vendor.asc".into(),
        destination: "/etc/apt/keyrings/vendor.gpg".into(),
    });

    fs::write(&input, "armored").unwrap();
    host.run_ok(&step);
    assert_eq!(fs::read(&destination).unwrap(), b"normalized-armored");
    assert_eq!(
        fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
        0o644
    );
    fs::write(&input, "binary").unwrap();
    host.run_ok(&step);
    assert_eq!(fs::read(&destination).unwrap(), b"normalized-binary");
    host.run_ok(&step);
    assert_eq!(fs::read(&destination).unwrap(), b"normalized-binary");

    let log = host.log();
    assert!(log.contains("curl --fail --silent --show-error --location --proto =https --tlsv1.2 --retry 3 --retry-all-errors --output"), "{log}");
    assert!(
        log.contains("gpg --no-options --batch --yes --dearmor --output"),
        "{log}"
    );
    assert!(
        log.contains("gpg --no-options --batch --no-default-keyring --keyring"),
        "{log}"
    );
    assert!(!log.contains("sudo gpg"), "{log}");
    assert!(!log.contains("sh -c"), "{log}");
}

#[test]
fn repository_key_failures_preserve_existing_bytes() {
    for failure in [
        "interrupted",
        "malformed",
        "empty",
        "conversion-failure",
        "validation-failure",
        "publish-mkdir",
        "publish-stage",
        "publish-sync",
        "publish-rename",
    ] {
        let host = Host::new();
        configure_key_fakes(&host);
        let destination = host.root.join("etc/apt/keyrings/vendor.gpg");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, b"old-key-bytes").unwrap();
        let key_input = match failure {
            "publish-mkdir" | "publish-stage" | "publish-sync" | "publish-rename" => "binary",
            other => other,
        };
        fs::write(host._dir.path().join("tmp/key-input"), key_input).unwrap();
        if let Some(point) = failure.strip_prefix("publish-") {
            fs::write(host._dir.path().join("tmp/publication-failure"), point).unwrap();
        }
        let step = Step::workflow(operations::Operation::RepositoryKey {
            url: "https://example.test/vendor.asc".into(),
            destination: "/etc/apt/keyrings/vendor.gpg".into(),
        });
        assert!(!host.run(&step).status.success(), "{failure}");
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"old-key-bytes",
            "{failure}"
        );
        assert_eq!(
            fs::read_dir(destination.parent().unwrap()).unwrap().count(),
            1,
            "{failure}"
        );
    }
}

#[test]
fn repository_key_rejects_url_and_destination_before_subprocesses() {
    let host = Host::new();
    host.logging_fake("curl");
    host.logging_fake("gpg");
    host.logging_fake("sudo");
    let oversized_label = "a".repeat(64);
    let invalid_urls = vec![
        "http://example.test/key".to_owned(),
        "https://example.test/key#fragment".into(),
        "HTTPS://example.test/key".into(),
        "https:///key".into(),
        "https://bad..example/key".into(),
        "https://_bad.example/key".into(),
        "https://-bad.example/key".into(),
        "https://bad-.example/key".into(),
        format!("https://{oversized_label}.example/key"),
        "https://127.1/key".into(),
        "https://0177.0.0.1/key".into(),
        "https://user@example.test/key".into(),
        "https://user:password@example.test/key".into(),
        "https://example.test\\key".into(),
        "https://%65xample.test/key".into(),
        "https://例え.テスト/key".into(),
    ];
    for url in invalid_urls {
        let step = Step::workflow(operations::Operation::RepositoryKey {
            url: url.clone(),
            destination: "/etc/apt/keyrings/vendor.gpg".into(),
        });
        assert!(!host.run(&step).status.success(), "{url}");
    }
    for destination in [
        "/etc/apt/keyrings/../vendor.gpg",
        "/etc/apt/keyrings/vendor.asc",
        "/etc/apt/keyrings/.gpg",
        "/etc/apt/keyrings/vendor.gpg\0suffix",
        "/tmp/vendor.gpg",
    ] {
        let step = Step::workflow(operations::Operation::RepositoryKey {
            url: "https://example.test/key".into(),
            destination: destination.into(),
        });
        assert!(!host.run(&step).status.success(), "{destination}");
    }
    assert!(host.log().is_empty(), "{}", host.log());
}

#[test]
fn repository_key_accepts_canonical_https_url_forms_before_curl() {
    for url in [
        "https://example.test/",
        "https://example.test:8443/key",
        "https://example.test/path/to/key?version=1",
        "https://192.0.2.1/key",
        "https://[2001:db8::1]/key",
    ] {
        let host = Host::new();
        host.fake("curl", "printf 'curl %s\\n' \"$*\" >>\"$LOG\"; exit 67");
        host.logging_fake("gpg");
        host.logging_fake("sudo");
        let step = Step::workflow(operations::Operation::RepositoryKey {
            url: url.into(),
            destination: "/etc/apt/keyrings/vendor.gpg".into(),
        });
        assert!(!host.run(&step).status.success(), "{url}");
        let log = host.log();
        assert!(log.contains(&format!(" {url}\n")), "{url}: {log}");
        assert_eq!(log.matches("curl ").count(), 1, "{url}: {log}");
        assert!(!log.contains("gpg "), "{url}: {log}");
        assert!(!log.contains("sudo "), "{url}: {log}");
    }
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
if [ "${{1:-}}" = install ]; then touch "$TMPDIR/gnome-extension-loaded"; fi
if [ "${{1:-}}" = list ] && {{ [ {present} = true ] || [ -f "$TMPDIR/gnome-extension-loaded" ]; }}; then printf '%s\n' '{extension}'; fi"#
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
fn newly_installed_gnome_extension_defers_enable_without_blocking_remaining_installs() {
    let steps = plans("configs/full.yaml", "configure", "ubuntu");
    let step = step_containing(&steps, "workflow gnome-extension");
    let host = Host::new();
    host.logging_fake("gnome-extensions");
    host.fake("gnome-shell", "printf 'GNOME Shell 48.4\\n'");
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done; [ -z "$out" ] || : >"$out"; printf '{"shell_version_map":{"48":{"version":13}}}\n'"#,
    );

    let output = host.run(&step);
    assert!(output.status.success());
    assert!(host.log().contains("gnome-extensions install --force"));
    assert!(!host.log().contains("gnome-extensions enable"));
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
fn direct_appimage_ensure_skips_network_but_update_forces_resolution() {
    let host = Host::new();
    let artifact = host
        .home
        .join(".local/share/cozydot/direct/sample.AppImage");
    let links = [
        host.home.join(".local/bin/sample"),
        host.home.join(".local/bin/sample-cli"),
    ];
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::create_dir_all(links[0].parent().unwrap()).unwrap();
    fs::write(&artifact, b"\x7fELFold").unwrap();
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
    for link in &links {
        symlink(&artifact, link).unwrap();
    }
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf '\177ELFnew' >"$out"; else printf '{"assets":[{"name":"sample-amd64-1.AppImage","browser_download_url":"https://example.test/sample.AppImage"}]}'; fi"#,
    );

    host.run_ok(&direct_step(
        operations::DirectPackageFormat::AppImage,
        &["sample", "sample-cli"],
        operations::DirectPackageMode::EnsurePresent,
    ));
    assert!(host.log().is_empty());

    host.run_ok(&direct_step(
        operations::DirectPackageFormat::AppImage,
        &["sample", "sample-cli"],
        operations::DirectPackageMode::Update,
    ));
    assert_eq!(fs::read(&artifact).unwrap(), b"\x7fELFnew");
    for link in &links {
        assert_eq!(fs::read_link(link).unwrap(), artifact);
    }
    let log = host.log();
    assert!(log.contains("api.github.com/repos/owner/repo/releases/latest"));
    assert!(log.contains("--proto =https"), "{log}");
    assert!(log.contains("User-Agent: cozydot/0.0.1"), "{log}");
}

#[test]
fn direct_appimage_ensure_repairs_missing_managed_link_despite_path_executable() {
    let host = Host::new();
    let artifact = host
        .home
        .join(".local/share/cozydot/direct/sample.AppImage");
    let link = host.home.join(".local/bin/sample");
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::write(&artifact, b"\x7fELFold").unwrap();
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
    host.fake("sample", "exit 0");
    host.fake("curl", "printf 'curl\n' >>\"$LOG\"; exit 97");

    host.run_ok(&direct_step(
        operations::DirectPackageFormat::AppImage,
        &["sample"],
        operations::DirectPackageMode::EnsurePresent,
    ));

    assert_eq!(fs::read_link(link).unwrap(), artifact);
    assert!(host.log().is_empty());
}

#[test]
fn direct_appimage_ensure_restores_all_missing_managed_links_without_network() {
    let host = Host::new();
    let artifact = host
        .home
        .join(".local/share/cozydot/direct/sample.AppImage");
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::write(&artifact, b"\x7fELFold").unwrap();
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
    host.fake("curl", "printf 'curl\n' >>\"$LOG\"; exit 97");

    host.run_ok(&direct_step(
        operations::DirectPackageFormat::AppImage,
        &["sample", "sample-cli"],
        operations::DirectPackageMode::EnsurePresent,
    ));

    for provide in ["sample", "sample-cli"] {
        assert_eq!(
            fs::read_link(host.home.join(".local/bin").join(provide)).unwrap(),
            artifact
        );
    }
    assert!(host.log().is_empty());
}

#[test]
fn direct_appimage_ensure_repairs_only_missing_managed_link_without_network() {
    let host = Host::new();
    let artifact = host
        .home
        .join(".local/share/cozydot/direct/sample.AppImage");
    let existing = host.home.join(".local/bin/sample");
    let missing = host.home.join(".local/bin/sample-cli");
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::create_dir_all(existing.parent().unwrap()).unwrap();
    fs::write(&artifact, b"\x7fELFold").unwrap();
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
    symlink(&artifact, &existing).unwrap();
    let existing_inode = fs::symlink_metadata(&existing).unwrap().ino();
    host.fake("curl", "printf 'curl\n' >>\"$LOG\"; exit 97");

    host.run_ok(&direct_step(
        operations::DirectPackageFormat::AppImage,
        &["sample", "sample-cli"],
        operations::DirectPackageMode::EnsurePresent,
    ));

    assert_eq!(
        fs::symlink_metadata(&existing).unwrap().ino(),
        existing_inode
    );
    assert_eq!(fs::read_link(&existing).unwrap(), artifact);
    assert_eq!(fs::read_link(&missing).unwrap(), artifact);
    assert!(host.log().is_empty());
}

#[test]
fn direct_appimage_ensure_rejects_foreign_link_despite_path_executable() {
    let host = Host::new();
    let artifact = host
        .home
        .join(".local/share/cozydot/direct/sample.AppImage");
    let link = host.home.join(".local/bin/sample");
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    fs::write(&artifact, b"\x7fELFold").unwrap();
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
    symlink("/foreign", &link).unwrap();
    host.fake("sample", "exit 0");
    host.fake("curl", "printf 'curl\n' >>\"$LOG\"; exit 97");

    let output = host.run(&direct_step(
        operations::DirectPackageFormat::AppImage,
        &["sample"],
        operations::DirectPackageMode::EnsurePresent,
    ));

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("link conflict"));
    assert_eq!(fs::read_link(link).unwrap(), Path::new("/foreign"));
    assert!(host.log().is_empty());
}

#[test]
fn direct_appimage_ensure_replaces_symlink_and_non_elf_artifacts() {
    for state in ["elf-symlink", "non-elf-symlink", "regular-non-elf", "fifo"] {
        let host = Host::new();
        let artifact = host
            .home
            .join(".local/share/cozydot/direct/sample.AppImage");
        let link = host.home.join(".local/bin/sample");
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        match state {
            "elf-symlink" | "non-elf-symlink" => {
                let target = host.home.join(format!("{state}.target"));
                fs::write(
                    &target,
                    if state == "elf-symlink" {
                        b"\x7fELFold".as_slice()
                    } else {
                        b"not-elf".as_slice()
                    },
                )
                .unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
                symlink(target, &artifact).unwrap();
            }
            "regular-non-elf" => {
                fs::write(&artifact, b"not-elf").unwrap();
                fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
            }
            "fifo" => {
                assert!(Command::new("mkfifo")
                    .arg(&artifact)
                    .status()
                    .unwrap()
                    .success());
            }
            _ => unreachable!(),
        }
        symlink(&artifact, &link).unwrap();
        host.fake(
            "curl",
            r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf '\177ELFnew' >"$out"; else printf '{"assets":[{"name":"sample-amd64-1.AppImage","browser_download_url":"https://example.test/sample.AppImage"}]}'; fi"#,
        );

        host.run_ok(&direct_step(
            operations::DirectPackageFormat::AppImage,
            &["sample"],
            operations::DirectPackageMode::EnsurePresent,
        ));
        assert_eq!(fs::read(&artifact).unwrap(), b"\x7fELFnew", "{state}");
        assert!(
            fs::symlink_metadata(&artifact)
                .unwrap()
                .file_type()
                .is_file(),
            "{state}"
        );
        assert!(!host.log().is_empty(), "{state}");
    }
}

#[test]
fn direct_appimage_directory_artifact_does_not_satisfy_managed_state() {
    let host = Host::new();
    let artifact = host
        .home
        .join(".local/share/cozydot/direct/sample.AppImage");
    let link = host.home.join(".local/bin/sample");
    fs::create_dir_all(&artifact).unwrap();
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    symlink(&artifact, &link).unwrap();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf '\177ELFnew' >"$out"; else printf '{"assets":[{"name":"sample-amd64-1.AppImage","browser_download_url":"https://example.test/sample.AppImage"}]}'; fi"#,
    );

    let output = host.run(&direct_step(
        operations::DirectPackageFormat::AppImage,
        &["sample"],
        operations::DirectPackageMode::EnsurePresent,
    ));
    assert!(!output.status.success());
    assert!(artifact.is_dir());
    assert!(!host.log().is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("publish direct AppImage"));
}

#[test]
fn direct_appimage_failed_downloads_preserve_old_artifact_and_links() {
    for failure in ["interrupted", "empty", "checksum"] {
        let host = Host::new();
        let artifact = host
            .home
            .join(".local/share/cozydot/direct/sample.AppImage");
        let link = host.home.join(".local/bin/sample");
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        fs::write(&artifact, b"\x7fELFold").unwrap();
        fs::set_permissions(&artifact, fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&artifact, &link).unwrap();
        let digest = if failure == "checksum" {
            r#", "digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000""#
        } else {
            ""
        };
        host.fake(
            "curl",
            &format!(
                r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -z "$out" ]; then printf '{{"assets":[{{"name":"sample-amd64-1.AppImage","browser_download_url":"https://example.test/sample.AppImage"{digest}}}]}}'; exit 0; fi
case {failure} in interrupted) printf partial >"$out"; exit 42 ;; empty) : >"$out" ;; checksum) printf '\177ELFnew' >"$out" ;; esac"#
            ),
        );

        let output = host.run(&direct_step(
            operations::DirectPackageFormat::AppImage,
            &["sample"],
            operations::DirectPackageMode::Update,
        ));
        assert!(!output.status.success(), "{failure}");
        assert_eq!(fs::read(&artifact).unwrap(), b"\x7fELFold", "{failure}");
        assert_eq!(fs::read_link(&link).unwrap(), artifact, "{failure}");
    }
}

#[test]
fn direct_debian_preflight_install_and_post_install_verification_are_fixed() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf deb >"$out"; else printf '{"assets":[{"name":"sample-amd64-1.deb","browser_download_url":"https://example.test/sample.deb"}]}'; fi"#,
    );
    host.fake(
        "dpkg-deb",
        r#"printf 'dpkg-deb %s\n' "$*" >>"$LOG"
[ "$1" = --info ] && [ "$2" = -- ]"#,
    );
    host.fake(
        "sudo",
        r#"printf 'sudo-arg <%s>\n' "$@" >>"$LOG"
[ "$#" -eq 7 ] && [ "$1" = DEBIAN_FRONTEND=noninteractive ] && [ "$2" = apt-get ] && [ "$3" = install ] && [ "$4" = -y ] && [ "$5" = -qq ] && [ "$6" = -- ] && [[ "$7" = *.deb ]]
bin=${PATH%%:*}; printf '#!/bin/sh\n' >"$bin/sample"; chmod 0755 "$bin/sample""#,
    );
    host.run_ok(&direct_step(
        operations::DirectPackageFormat::Deb,
        &["sample"],
        operations::DirectPackageMode::EnsurePresent,
    ));
    let log = host.log();
    assert!(log.contains("dpkg-deb --info -- "), "{log}");
    assert!(
        log.lines()
            .find(|line| line.starts_with("dpkg-deb "))
            .is_some_and(|line| line.ends_with(".deb")),
        "{log}"
    );
    let sudo_args = log
        .lines()
        .filter(|line| line.starts_with("sudo-arg "))
        .collect::<Vec<_>>();
    assert_eq!(sudo_args.len(), 7, "{log}");
    assert_eq!(sudo_args[0], "sudo-arg <DEBIAN_FRONTEND=noninteractive>");
    assert_eq!(sudo_args[1], "sudo-arg <apt-get>");
    assert_eq!(sudo_args[2], "sudo-arg <install>");
    assert_eq!(sudo_args[3], "sudo-arg <-y>");
    assert_eq!(sudo_args[4], "sudo-arg <-qq>");
    assert_eq!(sudo_args[5], "sudo-arg <-->");
    assert!(sudo_args[6].ends_with(".deb>"), "{log}");
    assert!(!log.contains("apt-get update"), "{log}");

    let skipped_log = host.log();
    host.run_ok(&direct_step(
        operations::DirectPackageFormat::Deb,
        &["sample"],
        operations::DirectPackageMode::EnsurePresent,
    ));
    assert_eq!(host.log(), skipped_log);
}

#[test]
fn direct_debian_install_failure_propagates_before_provides_verification() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf deb >"$out"; else printf '{"assets":[{"name":"sample-amd64-1.deb","browser_download_url":"https://example.test/sample.deb"}]}'; fi"#,
    );
    host.fake("dpkg-deb", r#"[ "$1" = --info ] && [ "$2" = -- ]"#);
    host.fake(
        "sudo",
        r#"printf 'sudo-arg <%s>\n' "$@" >>"$LOG"
[ "$#" -eq 7 ] && [ "$1" = DEBIAN_FRONTEND=noninteractive ] && [ "$2" = apt-get ] && [ "$3" = install ] && [ "$4" = -y ] && [ "$5" = -qq ] && [ "$6" = -- ] && [[ "$7" = *.deb ]]
exit 42"#,
    );

    let output = host.run(&direct_step(
        operations::DirectPackageFormat::Deb,
        &["sample"],
        operations::DirectPackageMode::EnsurePresent,
    ));
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("direct Debian install"), "{stderr}");
    assert!(!stderr.contains("remain unavailable"), "{stderr}");
    let log = host.log();
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("sudo-arg "))
            .count(),
        7,
        "{log}"
    );
}

#[test]
fn direct_debian_preflight_failure_prevents_sudo_and_missing_provide_fails() {
    for preflight_succeeds in [false, true] {
        let host = Host::new();
        host.fake(
            "curl",
            r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf deb >"$out"; else printf '{"assets":[{"name":"sample-amd64-1.deb","browser_download_url":"https://example.test/sample.deb"}]}'; fi"#,
        );
        host.fake(
            "dpkg-deb",
            &format!(
                "printf 'dpkg-deb %s\\n' \"$*\" >>\"$LOG\"; exit {}",
                if preflight_succeeds { 0 } else { 42 }
            ),
        );
        host.logging_fake("sudo");
        let output = host.run(&direct_step(
            operations::DirectPackageFormat::Deb,
            &["sample"],
            operations::DirectPackageMode::EnsurePresent,
        ));
        assert!(!output.status.success());
        assert_eq!(
            host.log()
                .contains("sudo DEBIAN_FRONTEND=noninteractive apt-get"),
            preflight_succeeds
        );
        if preflight_succeeds {
            assert!(String::from_utf8_lossy(&output.stderr).contains("remain unavailable"));
        }
    }
}

#[test]
fn direct_appimage_publishes_elf_mode_and_multiple_retry_safe_links() {
    let host = Host::new();
    let artifact = host
        .home
        .join(".local/share/cozydot/direct/sample.AppImage");
    let first_link = host.home.join(".local/bin/sample");
    fs::create_dir_all(first_link.parent().unwrap()).unwrap();
    symlink(&artifact, &first_link).unwrap();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf '\177ELFpayload' >"$out"; else printf '{"assets":[{"name":"sample-amd64-1.AppImage","browser_download_url":"https://example.test/sample.AppImage","digest":"sha256:f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb"}]}'; fi"#,
    );
    let step = direct_step(
        operations::DirectPackageFormat::AppImage,
        &["sample", "sample-cli"],
        operations::DirectPackageMode::EnsurePresent,
    );
    host.run_ok(&step);
    assert_eq!(fs::read(&artifact).unwrap(), b"\x7fELFpayload");
    assert_eq!(
        fs::metadata(&artifact).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read_link(&first_link).unwrap(), artifact);
    assert_eq!(
        fs::read_link(host.home.join(".local/bin/sample-cli")).unwrap(),
        artifact
    );
    let first_log = host.log();
    host.run_ok(&step);
    assert_eq!(host.log(), first_log);
}

#[test]
fn direct_appimage_rejects_non_elf_and_link_conflicts_without_publication() {
    for conflict in ["regular", "directory", "symlink", "none"] {
        let host = Host::new();
        let artifact = host
            .home
            .join(".local/share/cozydot/direct/sample.AppImage");
        let link = host.home.join(".local/bin/sample");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        match conflict {
            "regular" => fs::write(&link, b"foreign").unwrap(),
            "directory" => fs::create_dir(&link).unwrap(),
            "symlink" => symlink("/foreign", &link).unwrap(),
            "none" => {}
            _ => unreachable!(),
        }
        host.fake(
            "curl",
            r#"printf 'curl\n' >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf not-elf >"$out"; else printf '{"assets":[{"name":"sample-amd64-1.AppImage","browser_download_url":"https://example.test/sample.AppImage"}]}'; fi"#,
        );
        let output = host.run(&direct_step(
            operations::DirectPackageFormat::AppImage,
            &["sample"],
            operations::DirectPackageMode::EnsurePresent,
        ));
        assert!(!output.status.success(), "{conflict}");
        assert!(!artifact.exists(), "{conflict}");
        if conflict == "none" {
            assert!(String::from_utf8_lossy(&output.stderr).contains("ELF magic"));
        } else {
            assert!(host.log().is_empty(), "{conflict}: {}", host.log());
        }
    }
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
    host.fake(
        "uv",
        r#"printf 'uv %s\n' "$*" >>"$LOG"
if [ "${1:-}" = python ] && [ "${2:-}" = find ]; then exit 1; fi"#,
    );
    let step = step_containing(
        &plans("configs/cli.yaml", "install", "ubuntu"),
        "workflow uv-install",
    );
    host.run_ok(&step);
    let log = host.log();
    assert!(!log.contains("uv self update"), "{log}");
    assert!(log.contains("uv python find 3.13"), "{log}");
    assert!(log.contains("uv python install 3.13"), "{log}");
    assert!(!log.contains("uv python list"), "{log}");
}

#[test]
fn uv_skips_python_install_when_requested_series_is_resolvable() {
    let host = Host::new();
    host.fake(
        "uv",
        r#"printf 'uv %s\n' "$*" >>"$LOG"
if [ "${1:-}" = python ] && [ "${2:-}" = find ]; then printf '%s\n' '/managed/python'; exit 0; fi"#,
    );
    let step = step_containing(
        &plans("configs/cli.yaml", "install", "ubuntu"),
        "workflow uv-install",
    );
    host.run_ok(&step);
    let log = host.log();
    assert!(log.contains("uv python find 3.13"), "{log}");
    assert!(!log.contains("uv python install"), "{log}");
}

#[test]
fn standalone_npm_packages_skip_installed_specs() {
    let host = Host::new();
    host.fake(
        "npm",
        r#"printf 'npm %s\n' "$*" >>"$LOG"
if [ "${1:-}" = list ]; then [ -f "$TMPDIR/npm-standalone-installed" ]; exit; fi
if [ "${1:-}" = install ]; then touch "$TMPDIR/npm-standalone-installed"; fi"#,
    );
    let step = Step::workflow(operations::Operation::NpmPackages {
        packages: vec!["opencode-ai".into()],
    });
    host.run_ok(&step);
    host.run_ok(&step);
    let log = host.log();
    assert_eq!(
        log.matches("npm list --global --depth=0 opencode-ai")
            .count(),
        2,
        "{log}"
    );
    assert_eq!(
        log.matches("npm install --global opencode-ai").count(),
        1,
        "{log}"
    );
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
if [ "${1:-}" = python ] && [ "${2:-}" = find ]; then exit 1; fi
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
