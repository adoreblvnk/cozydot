use cozydot::{config::HttpsUrl, operations, platform::Architecture, runner::Step};
use std::{
    fs,
    io::Write,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

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
map_path() { case "$1" in /etc/*|/run/*|/snap|/var/snap|/var/lib/snapd|/usr/share/keyrings/*|/var/lib/cozydot/*) printf '%s%s' "$ROOT" "$1" ;; *) printf '%s' "$1" ;; esac; }
failure=''; [ ! -f "$TMPDIR/publication-failure" ] || failure=$(cat "$TMPDIR/publication-failure")
case "$command" in
  stat)
    destination=$(map_path "${!#}")
    if [ "$1" = --format=%f:%u:%g ]; then
      printf '%s:0:0\n' "$(/usr/bin/stat --format=%f -- "$destination")"
    elif [ "$1" = --format=%f:%s ]; then
      /usr/bin/stat --format=%f:%s -- "$destination"
    else
      /usr/bin/stat --format=%f -- "$destination"
    fi
    ;;
  cat)
    logical=${!#}
    destination=$(map_path "${!#}")
    if [ -f "$TMPDIR/repository-postcondition-failure" ] && [ "$logical" = "$(cat "$TMPDIR/repository-postcondition-failure")" ]; then
      printf 'corrupt-postcondition'
      exit
    fi
    if [ "$logical" = /etc/docker/daemon.json ]; then
      count=0; [ ! -f "$TMPDIR/publication-cat-count" ] || count=$(cat "$TMPDIR/publication-cat-count")
      count=$((count + 1)); printf '%s' "$count" >"$TMPDIR/publication-cat-count"
      if [ -f "$TMPDIR/publication-postcondition-failure" ] && [ "$count" -gt 1 ]; then
        printf '{}\n'
        exit
      fi
      if [ -f "$TMPDIR/publication-pause-after-read" ] && [ "$count" -eq 1 ]; then
        touch "$TMPDIR/publication-read-observed"
        while [ ! -f "$TMPDIR/publication-read-release" ]; do sleep 0.01; done
      fi
    fi
    /bin/cat -- "$destination"
    ;;
  find)
    [ "$#" -eq 15 ] && [ "$1" = /etc/apt ] && [ "$2" = -xdev ] && [ "$3" = -maxdepth ] && [ "$4" = 2 ] && [ "$5" = '(' ] && [ "$6" = -path ] && [ "$7" = /etc/apt/sources.list ] && [ "$8" = -o ] && [ "$9" = -path ] && [ "${10}" = '/etc/apt/sources.list.d/*.list' ] && [ "${11}" = -o ] && [ "${12}" = -path ] && [ "${13}" = '/etc/apt/sources.list.d/*.sources' ] && [ "${14}" = ')' ] && [ "${15}" = -print0 ] || exit 54
    apt_root=$(map_path /etc/apt)
    while IFS= read -r -d '' path; do
      printf '%s\0' "${path#"$ROOT"}"
    done < <(/usr/bin/find "$apt_root" -xdev -maxdepth 2 \( -path "$apt_root/sources.list" -o -path "$apt_root/sources.list.d/*.list" -o -path "$apt_root/sources.list.d/*.sources" \) -print0)
    ;;
  install)
    if [ "${1:-}" = -d ]; then
      { [ "$failure" != mkdir ] || [ "${!#}" = /run/cozydot ]; } || exit 41
      destination=$(map_path "${!#}")
      mkdir -p "$destination"
      chmod 0755 "$destination"
    else
      [ "$failure" != stage ] || exit 42
      source=${@: -2:1}; destination=$(map_path "${!#}")
      { [ "$failure" != source-stage ] || [[ "$destination" != *cozydot-vendor-name.list* ]]; } || exit 57
      [ "$5" = -m ] && { [ "$6" = 0600 ] || [ "$6" = 0644 ]; } || exit 55
      /usr/bin/install -m "$6" -- "$source" "$destination"
    fi
    ;;
  cp)
    [ "$failure" != lock-setup ] || exit 50
    [ "$#" -eq 5 ] && [ "$1" = --no-clobber ] && [ "$2" = --no-target-directory ] && [ "$3" = -- ] && [ "$4" = /dev/null ] || exit 51
    destination=$(map_path "$5")
    /bin/cp --no-clobber --no-target-directory -- /dev/null "$destination"
    ;;
  chown)
    [ "$#" -eq 4 ] && [ "$1" = --no-dereference ] && [ "$2" = root:root ] && [ "$3" = -- ] || exit 52
    ;;
  chmod)
    [ "$#" -eq 3 ] && [ "$1" = 0644 ] && [ "$2" = -- ] || exit 53
    ;;
  sync)
    target=$(map_path "${!#}")
    [ "$failure" != sync ] || exit 43
    { [ "$failure" != parent-sync ] || [ ! -d "$target" ]; } || exit 49
    /bin/sync -- "$target"
    ;;
  test)
    [ "$#" -eq 3 ] && [ "$1" = '!' ] || exit 46
    destination=$(map_path "$3")
    case "$2" in
      -d) [ ! -d "$destination" ] ;;
      -e) [ ! -e "$destination" ] ;;
      -L) [ ! -L "$destination" ] ;;
      *) exit 46 ;;
    esac
    ;;
  ln)
    [ "$#" -eq 3 ] && [ "$1" = -- ] || exit 58
    source=$(map_path "$2"); logical=$3; destination=$(map_path "$3")
    if [ -f "$TMPDIR/repository-inject-before-link" ] && [ "$logical" = "$(cat "$TMPDIR/repository-inject-before-link")" ]; then
      printf foreign-race-bytes >"$destination"
    fi
    /bin/ln -- "$source" "$destination"
    ;;
  mv)
    [ "$failure" != rename ] || exit 44
    { [ "$failure" != managed-second-rewrite ] || [ "$4" != /etc/apt/sources.list.d/second.list ]; } || exit 56
    [ "$#" -eq 4 ] && [ "$1" = -fT ] && [ "$2" = -- ] || exit 47
    source=$(map_path "$3"); destination=$(map_path "$4")
    /bin/mv -fT -- "$source" "$destination"
    ;;
  rm)
    if [ "${1:-}" = -rf ]; then
      shift; [ "${1:-}" != -- ] || shift
      for path in "$@"; do /bin/rm -rf -- "$(map_path "$path")"; done
    else
      /bin/rm -f -- "$(map_path "${!#}")"
    fi
    ;;
  apt-get|snap|systemctl)
    command "$command" "$@"
    ;;
  DEBIAN_FRONTEND=noninteractive)
    program=$1; shift
    command "$program" "$@"
    ;;
  *) exit 45 ;;
esac"#,
        );
    }

    fn run(&self, step: &Step) -> std::process::Output {
        self.run_with_path(step, format!("{}:/usr/bin:/bin", self.bin.display()))
    }

    fn execute_operation_as(
        &self,
        operation: &operations::Operation,
        user: &str,
    ) -> anyhow::Result<()> {
        self.execute_operation_as_with_path(
            operation,
            user,
            format!("{}:/usr/bin:/bin", self.bin.display()),
        )
    }

    fn execute_operation_as_with_state_home(
        &self,
        operation: &operations::Operation,
        state_home: &Path,
    ) -> anyhow::Result<()> {
        let env = [
            ("HOME".into(), self.home.as_os_str().to_owned()),
            ("USER".into(), "tester".into()),
            ("LOGNAME".into(), "tester".into()),
            ("SUDO_USER".into(), "tester".into()),
            ("LOG".into(), self.log.as_os_str().to_owned()),
            ("ROOT".into(), self.root.as_os_str().to_owned()),
            (
                "TMPDIR".into(),
                self._dir.path().join("tmp").into_os_string(),
            ),
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", self.bin.display()).into(),
            ),
            ("XDG_STATE_HOME".into(), state_home.as_os_str().to_owned()),
        ];
        operations::execute_with_docker_lock_for_test(
            operation,
            &env,
            &self.root.join("run/cozydot/docker-daemon.lock"),
        )
    }

    fn execute_operation_with_xdg_roots(
        &self,
        operation: &operations::Operation,
        data_home: &Path,
        bin_home: &Path,
    ) -> anyhow::Result<()> {
        let env = [
            ("HOME".into(), self.home.as_os_str().to_owned()),
            ("USER".into(), "tester".into()),
            ("LOG".into(), self.log.as_os_str().to_owned()),
            ("ROOT".into(), self.root.as_os_str().to_owned()),
            (
                "TMPDIR".into(),
                self._dir.path().join("tmp").into_os_string(),
            ),
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", self.bin.display()).into(),
            ),
            ("XDG_DATA_HOME".into(), data_home.as_os_str().to_owned()),
            ("XDG_BIN_HOME".into(), bin_home.as_os_str().to_owned()),
        ];
        operations::execute_with_docker_lock_for_test(
            operation,
            &env,
            &self.root.join("run/cozydot/docker-daemon.lock"),
        )
    }

    fn execute_operation_with_lock(
        &self,
        operation: &operations::Operation,
        lock: &Path,
    ) -> anyhow::Result<()> {
        let env = [
            ("HOME".into(), self.home.as_os_str().to_owned()),
            ("USER".into(), "tester".into()),
            ("LOG".into(), self.log.as_os_str().to_owned()),
            ("ROOT".into(), self.root.as_os_str().to_owned()),
            (
                "TMPDIR".into(),
                self._dir.path().join("tmp").into_os_string(),
            ),
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", self.bin.display()).into(),
            ),
        ];
        operations::execute_with_docker_lock_for_test(operation, &env, lock)
    }

    fn execute_operation_as_with_path(
        &self,
        operation: &operations::Operation,
        user: &str,
        path: String,
    ) -> anyhow::Result<()> {
        let env = [
            ("HOME".into(), self.home.as_os_str().to_owned()),
            ("USER".into(), user.into()),
            ("LOGNAME".into(), user.into()),
            ("SUDO_USER".into(), user.into()),
            ("LOG".into(), self.log.as_os_str().to_owned()),
            ("ROOT".into(), self.root.as_os_str().to_owned()),
            (
                "TMPDIR".into(),
                self._dir.path().join("tmp").into_os_string(),
            ),
            ("PATH".into(), path.into()),
        ];
        operations::execute_with_docker_lock_for_test(
            operation,
            &env,
            &self.root.join("run/cozydot/docker-daemon.lock"),
        )
    }

    fn run_with_path(&self, step: &Step, path: String) -> std::process::Output {
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
            ("PATH".into(), path.into()),
            (
                "XDG_CONFIG_HOME".into(),
                self.home.join(".config").into_os_string(),
            ),
            (
                "XDG_DATA_HOME".into(),
                self.home.join(".local/share").into_os_string(),
            ),
        ];
        let result = operations::execute_with_docker_lock_for_test(
            step.operation(),
            &env,
            &self.root.join("run/cozydot/docker-daemon.lock"),
        );
        let mut command = Command::new("sh");
        if let Err(error) = result {
            command
                .args(["-c", "printf '%s\\n' \"$ERROR\" >&2; exit 1"])
                .env("ERROR", format!("{error:#}"));
        } else {
            command.args(["-c", "exit 0"]);
        }
        command.output().unwrap()
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

fn binary_step(
    format: operations::BinaryPackageFormat,
    commands: &[&str],
    source: operations::BinarySourceOperation,
    mode: operations::BinaryPackageMode,
) -> Step {
    Step::workflow(operations::Operation::BinaryPackage(
        operations::BinaryPackageOperation::new(
            "sample",
            format,
            commands.iter().map(|value| (*value).into()).collect(),
            Architecture::Amd64,
            source,
            mode,
        )
        .unwrap(),
    ))
}

fn github_source(sha256: Option<&str>) -> operations::BinarySourceOperation {
    operations::BinarySourceOperation::GithubLatest {
        repository: operations::GithubRepository::parse("owner/repo").unwrap(),
        selector: operations::BinaryPackageSelector::new(
            "sample-amd64-*",
            vec!["sample-amd64-debug-*".into()],
        )
        .unwrap(),
        sha256: sha256
            .map(operations::BinarySha256::parse)
            .transpose()
            .unwrap(),
    }
}

fn fixed_source(url: &str, sha256: &str) -> operations::BinarySourceOperation {
    operations::BinarySourceOperation::ChecksummedUrl {
        url: serde_yaml::from_str(url).unwrap(),
        sha256: operations::BinarySha256::parse(sha256).unwrap(),
    }
}

fn cargo_package_step(packages: &[&str], mode: operations::CargoPackageMode) -> Step {
    Step::workflow(operations::Operation::CargoPackageSet(
        operations::CargoPackageOperation::new(
            packages.iter().map(|package| (*package).into()).collect(),
            mode,
        )
        .unwrap(),
    ))
}

fn npm_package_step(packages: &[&str], mode: operations::NpmPackageMode) -> Step {
    Step::workflow(operations::Operation::NpmPackageSet(
        operations::NpmPackageOperation::new(
            packages.iter().map(|package| (*package).into()).collect(),
            mode,
        )
        .unwrap(),
    ))
}

fn rust_toolchain_step(mode: operations::ToolMutationMode) -> Step {
    rust_toolchain_selector_step(operations::RustToolchainSelector::Stable, mode)
}

fn rust_toolchain_selector_step(
    selector: operations::RustToolchainSelector,
    mode: operations::ToolMutationMode,
) -> Step {
    Step::workflow(operations::Operation::RustToolchain(
        operations::RustToolchainOperation::new(selector, Architecture::Amd64, mode).unwrap(),
    ))
}

fn node_toolchain_step(mode: operations::ToolMutationMode) -> Step {
    node_toolchain_selector_step(operations::NodeToolchainSelector::Lts, mode)
}

fn node_toolchain_selector_step(
    selector: operations::NodeToolchainSelector,
    mode: operations::ToolMutationMode,
) -> Step {
    Step::workflow(operations::Operation::NodeToolchain(
        operations::NodeToolchainOperation::new(selector, Architecture::Amd64, mode).unwrap(),
    ))
}

fn python_toolchain_step(version: &str) -> Step {
    Step::workflow(operations::Operation::PythonToolchain(
        operations::PythonToolchainOperation::new(version, Architecture::Amd64).unwrap(),
    ))
}

fn bootstrap_step(operation: operations::Operation) -> Step {
    Step::workflow(operation)
}

fn cargo_binstall_bootstrap_step(architecture: Architecture) -> Step {
    bootstrap_step(operations::Operation::CargoBinstallBootstrap(
        operations::CargoBinstallBootstrapOperation::new(architecture),
    ))
}

fn nerd_fonts_step(families: &[&str]) -> Step {
    Step::workflow(operations::Operation::NerdFonts(
        operations::NerdFontsOperation::new(
            families.iter().map(|family| (*family).into()).collect(),
        )
        .unwrap(),
    ))
}

fn dotfiles_step(root: &Path, packages: &[&str]) -> Step {
    Step::workflow(operations::Operation::Dotfiles(
        operations::DotfilesOperation::new(
            root.to_path_buf(),
            packages.iter().map(|package| (*package).into()).collect(),
        )
        .unwrap(),
    ))
}

fn desktop_setting_step(
    target: operations::DesktopEnvironment,
    setting: operations::DesktopSetting,
) -> Step {
    Step::workflow(operations::Operation::DesktopSetting(
        operations::DesktopSettingOperation::new(target, setting).unwrap(),
    ))
}

fn gnome_extensions_step(extensions: &[&str]) -> Step {
    Step::workflow(operations::Operation::GnomeExtensions(
        operations::GnomeExtensionsOperation::new(
            extensions
                .iter()
                .map(|extension| (*extension).into())
                .collect(),
        )
        .unwrap(),
    ))
}

fn gnome_dock_step() -> Step {
    Step::workflow(operations::Operation::GnomeDock(
        operations::GnomeDockOperation::new(),
    ))
}

fn gnome_rounded_corners_step() -> Step {
    Step::workflow(operations::Operation::GnomeRoundedCorners(
        operations::GnomeRoundedCornersOperation::new(),
    ))
}

fn ensure_admin_step() -> Step {
    Step::workflow(operations::Operation::EnsureAdmin(
        operations::EnsureAdminOperation::new(),
    ))
}

fn unattended_upgrades_step(enabled: bool) -> Step {
    Step::workflow(operations::Operation::UnattendedUpgrades(
        operations::UnattendedUpgradesOperation::new(enabled),
    ))
}

fn ubuntu_snap_step(enabled: bool) -> Step {
    Step::workflow(operations::Operation::UbuntuSnap(
        operations::UbuntuSnapOperation::new(enabled),
    ))
}

fn configure_rust_toolchain_fake(host: &Host) {
    let cargo_bin = host.home.join(".cargo/bin");
    fs::create_dir_all(&cargo_bin).unwrap();
    fs::write(host._dir.path().join("tmp/rust-release"), b"1.90.0").unwrap();
    fs::write(host._dir.path().join("tmp/rust-date"), b"2026-01-01").unwrap();
    host.fake(
        "curl",
        r#"{ printf 'curl'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
release=$(cat "$TMPDIR/rust-release"); date=$(cat "$TMPDIR/rust-date")
printf 'manifest-version = "2"\ndate = "%s"\n\n[pkg.rust]\nversion = "%s (abc %s)"\n\n[pkg.rust.target.x86_64-unknown-linux-gnu]\navailable = true\n' "$date" "$release" "$date""#,
    );
    host.fake(
        "rustup",
        r#"{ printf 'rustup'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ "$1" = toolchain ] && [ "$2" = install ]; then
  [ ! -f "$TMPDIR/rust-install-failure" ] || exit 71
  release=${3%-x86_64-unknown-linux-gnu}
  printf '%s' "$release" >"$TMPDIR/rust-installed"
  exit
fi
if [ "$1" = run ]; then
  [ -f "$TMPDIR/rust-installed" ] || exit 1
  release=$(cat "$TMPDIR/rust-installed")
  printf 'rustc %s (abc 2026-01-01)\nbinary: rustc\ncommit-hash: abc\ncommit-date: 2026-01-01\nhost: x86_64-unknown-linux-gnu\nrelease: %s\nLLVM version: 20.1.0\n' "$release" "$release"
  exit
fi
if [ "$1" = default ]; then
  if [ "$#" -eq 2 ]; then printf '%s' "$2" >"$TMPDIR/rust-default"; exit; fi
  if [ -f "$TMPDIR/rust-default" ]; then printf '%s (default)\n' "$(cat "$TMPDIR/rust-default")"; else printf 'no default toolchain configured\n'; fi
  exit
fi
exit 40"#,
    );
    fs::rename(host.bin.join("rustup"), cargo_bin.join("rustup")).unwrap();
}

fn configure_node_toolchain_fake(host: &Host) {
    host.fake(
        "fnm",
        r#"{ printf 'fnm'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
case "$1" in
  exec)
    alias=$3
    [ -f "$TMPDIR/fnm-alias-$alias" ] || exit 1
    cat "$TMPDIR/fnm-alias-$alias"
    ;;
  list-remote)
    cat "$TMPDIR/fnm-remote"
    ;;
  install)
    [ ! -f "$TMPDIR/fnm-install-failure" ] || exit 71
    printf '%s\n' "$2" >"$TMPDIR/fnm-installed"
    ;;
  alias)
    printf '%s\n' "$2" >"$TMPDIR/fnm-alias-$3"
    ;;
  unalias)
    rm -f "$TMPDIR/fnm-alias-$2"
    ;;
  default)
    if [ "$#" -eq 2 ]; then printf '%s\n' "$2" >"$TMPDIR/fnm-default"; elif [ -f "$TMPDIR/fnm-default" ]; then cat "$TMPDIR/fnm-default"; else printf 'none\n'; fi
    ;;
  *) exit 40 ;;
esac"#,
    );
    let managed = host.home.join(".local/share/fnm");
    fs::create_dir_all(&managed).unwrap();
    fs::rename(host.bin.join("fnm"), managed.join("fnm")).unwrap();
    fs::write(host._dir.path().join("tmp/fnm-remote"), b"v22.14.0 (Jod)\n").unwrap();
}

fn configure_python_toolchain_fake(host: &Host) {
    fs::write(host._dir.path().join("tmp/python-remote"), b"3.13.7").unwrap();
    host.fake(
        "uv",
        r#"{ printf 'uv'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
[ "$1" = python ] || exit 40
if [ "$2" = find ]; then [ -f "$TMPDIR/python-version" ] || exit 1; cat "$TMPDIR/python-version"; exit; fi
if [ "$2" = list ]; then version=$(cat "$TMPDIR/python-remote"); printf '[{"version":"%s","url":"https://example.test/python.tar.gz","implementation":"cpython","os":"linux","variant":"default","arch":"x86_64","libc":"gnu"}]\n' "$version"; exit; fi
if [ "$2" = install ]; then [ ! -f "$TMPDIR/python-install-failure" ] || exit 71; printf '%s\n' "${!#}" >"$TMPDIR/python-version"; exit; fi
exit 41"#,
    );
    fs::create_dir_all(host.home.join(".local/bin")).unwrap();
    fs::rename(host.bin.join("uv"), host.home.join(".local/bin/uv")).unwrap();
}

fn configure_system_state_fakes(host: &Host) {
    host.atomic_sudo();
    host.fake(
        "dpkg-query",
        r#"{ printf 'dpkg-query'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
package=${!#}
if [ -f "$TMPDIR/package-$package" ]; then printf '%s\tii \n' "$package"; else exit 1; fi"#,
    );
    host.fake(
        "apt-get",
        r#"{ printf 'apt-get'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
for argument in "$@"; do
  case "$argument" in
    *+) touch "$TMPDIR/package-${argument%+}" ;;
    *-) rm -f "$TMPDIR/package-${argument%-}" ;;
  esac
done"#,
    );
    host.fake(
        "systemctl",
        r#"{ printf 'systemctl'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ "$1" = --quiet ]; then
  query=$2; unit=${3//./_}
  [ -f "$TMPDIR/systemd-$unit-${query#is-}" ]
  exit
fi
action=$1; shift
[ "${1:-}" != --now ] || shift
unit=${1//./_}
case "$action" in
  enable) touch "$TMPDIR/systemd-$unit-enabled" "$TMPDIR/systemd-$unit-active" ;;
  disable) rm -f "$TMPDIR/systemd-$unit-enabled" "$TMPDIR/systemd-$unit-active" ;;
  *) exit 40 ;;
esac"#,
    );
    host.fake(
        "snap",
        r#"{ printf 'snap'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ "$1" = list ]; then
  printf 'Name Version Rev Tracking Publisher Notes\n'
  [ ! -f "$TMPDIR/snap-firefox" ] || printf 'firefox 1 1 latest canonical -\n'
  exit
fi
if [ "$1" = remove ] && [ "$2" = --purge ]; then rm -f "$TMPDIR/snap-$3"; exit; fi
exit 41"#,
    );
}

fn configure_cargo_package_fakes(host: &Host, state: &str) {
    fs::write(host._dir.path().join("tmp/cargo-state"), state).unwrap();
    host.fake(
        "cargo",
        r#"{ printf 'cargo'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ "$1" = install ] && [ "$2" = --list ]; then cat "$TMPDIR/cargo-state"; exit; fi
exit 43"#,
    );
    host.fake(
        "cargo-binstall",
        r#"{ printf 'cargo-binstall'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
[ ! -f "$TMPDIR/cargo-mutation-failure" ] || exit 42
for package in "$@"; do
  case "$package" in --*) continue ;; esac
  if ! grep -q "^$package v" "$TMPDIR/cargo-state"; then
    printf '%s v1.0.0:\n    %s\n' "$package" "$package" >>"$TMPDIR/cargo-state"
  fi
done"#,
    );
    let cargo_bin = host.home.join(".cargo/bin");
    fs::create_dir_all(&cargo_bin).unwrap();
    fs::rename(host.bin.join("cargo"), cargo_bin.join("cargo")).unwrap();
    fs::rename(
        host.bin.join("cargo-binstall"),
        cargo_bin.join("cargo-binstall"),
    )
    .unwrap();
}

fn configure_npm_package_fakes(host: &Host, version: &[u8], state: &[u8], post_state: &[u8]) {
    let tmp = host._dir.path().join("tmp");
    fs::write(tmp.join("fnm-version"), version).unwrap();
    fs::write(tmp.join("npm-state"), state).unwrap();
    fs::write(tmp.join("npm-post-state"), post_state).unwrap();
    host.fake(
        "fnm",
        r#"{ printf 'fnm'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ "$1" = default ]; then cat "$TMPDIR/fnm-version"; exit; fi
[ "$1" = exec ] && [ "$2" = --using ] && [ "$4" = -- ] && [ "$5" = npm ] || exit 51
shift 5
if [ "$1" = list ]; then
  [ ! -f "$TMPDIR/npm-query-failure" ] || exit 52
  cat "$TMPDIR/npm-state"
  exit
fi
if [ "$1" = install ] || [ "$1" = update ]; then
  [ ! -f "$TMPDIR/npm-mutation-failure" ] || exit 53
  cp "$TMPDIR/npm-post-state" "$TMPDIR/npm-state"
  exit
fi
exit 54"#,
    );
    let managed = host.home.join(".local/share/fnm");
    fs::create_dir_all(&managed).unwrap();
    fs::rename(host.bin.join("fnm"), managed.join("fnm")).unwrap();
    host.fake(
        "npm",
        "printf 'ambient-npm <%s>\\n' \"$*\" >>\"$LOG\"; exit 90",
    );
}

fn docker_local_log_step(max_size: Option<&str>) -> Step {
    Step::workflow(operations::Operation::DockerLocalLog(
        operations::DockerLocalLogOperation::new(max_size.map(str::to_owned)).unwrap(),
    ))
}

fn managed_apt_sources_step(
    distro: &str,
    release: &str,
    architecture: Architecture,
    components: &[&str],
) -> Step {
    Step::workflow(operations::Operation::ManagedAptSources(
        operations::ManagedAptSourcesOperation::new(
            distro.into(),
            release.into(),
            architecture,
            components
                .iter()
                .map(|component| (*component).into())
                .collect(),
        )
        .unwrap(),
    ))
}

fn apt_repository_operation(key_url: &str, source_url: &str, suite: &str) -> operations::Operation {
    let key_url: HttpsUrl = serde_yaml::from_str(key_url).unwrap();
    let source_url: HttpsUrl = serde_yaml::from_str(source_url).unwrap();
    operations::Operation::AptRepository(
        operations::AptRepositoryOperation::new(
            "Vendor_Name",
            "vendor-name",
            key_url,
            source_url,
            Architecture::Amd64,
            operations::AptRepositorySourceLayout::SuiteComponents {
                suite: operations::AptRepositoryToken::parse(suite).unwrap(),
                components: vec![operations::AptRepositoryToken::parse("main").unwrap()],
            },
        )
        .unwrap(),
    )
}

fn assert_lock_released(path: &Path) {
    for _ in 0..100 {
        let file = fs::File::open(path).unwrap();
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => return,
            Err(rustix::io::Errno::WOULDBLOCK) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(error) => panic!("unexpected lock probe failure: {error}"),
        }
    }
    panic!("Docker operation did not release {}", path.display());
}

fn vscode_extension_step(extensions: &[&str]) -> Step {
    Step::workflow(operations::Operation::VsCodeExtensionSet(
        operations::VsCodeExtensionOperation::new(
            extensions
                .iter()
                .map(|extension| (*extension).into())
                .collect(),
        )
        .unwrap(),
    ))
}

fn configure_group_fakes(host: &Host, product: &str, version: &str, group: &str) {
    let uid = rustix::process::geteuid().as_raw();
    host.fake(
        product,
        &format!(
            "printf '{product} <%s>\\n' \"$*\" >>\"$LOG\"; [ \"$1\" = --version ] || exit 40; printf '%s\\n' '{version}'"
        ),
    );
    host.fake(
        "getent",
        &format!(
            r#"{{ printf 'getent'; printf ' <%s>' "$@"; printf '\n'; }} >>"$LOG"
if [ "$1" = passwd ] && [ "$2" = {uid} ]; then
  [ ! -f "$TMPDIR/passwd-query-failure" ] || exit 40
  [ ! -f "$TMPDIR/passwd-malformed" ] || {{ cat "$TMPDIR/passwd-malformed"; exit; }}
  printf 'tester:x:{uid}:1000:Tester:/home/tester:/bin/bash\n'; exit
fi
if [ "$1" = group ] && [ "$2" = {group} ]; then
  [ ! -f "$TMPDIR/group-query-failure" ] || exit 41
  [ -f "$TMPDIR/group-exists" ] || exit 2
  [ ! -f "$TMPDIR/group-malformed" ] || {{ cat "$TMPDIR/group-malformed"; exit; }}
  printf '{group}:x:997:\n'; exit
fi
exit 42"#
        ),
    );
    host.fake(
        "id",
        r#"{ printf 'id'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
[ "$1" = -G ] && [ "$2" = -- ] && [ "$3" = tester ] || exit 43
[ ! -f "$TMPDIR/id-query-failure" ] || exit 44
cat "$TMPDIR/group-state""#,
    );
    host.fake(
        "sudo",
        &format!(
            r#"{{ printf 'sudo'; printf ' <%s>' "$@"; printf '\n'; }} >>"$LOG"
if [ "$1" = groupadd ]; then
  [ "$2" = --system ] && [ "$3" = {group} ] || exit 45
  [ ! -f "$TMPDIR/groupadd-failure" ] || exit 46
  touch "$TMPDIR/group-exists"; exit
fi
if [ "$1" = usermod ]; then
  [ "$2" = -aG ] && [ "$3" = {group} ] && [ "$4" = -- ] && [ "$5" = tester ] || exit 47
  [ ! -f "$TMPDIR/usermod-failure" ] || exit 48
  [ -f "$TMPDIR/postcondition-failure" ] || printf '1000 997\n' >"$TMPDIR/group-state"
  exit
fi
exit 49"#
        ),
    );
}

fn configure_vscode_fake(host: &Host, state: &[u8]) {
    fs::write(host._dir.path().join("tmp/code-state"), state).unwrap();
    host.fake(
        "code",
        r#"{ printf 'code'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ "$1" = --version ]; then
  [ ! -f "$TMPDIR/code-version-failure" ] || exit 51
  [ ! -f "$TMPDIR/code-version-malformed" ] || { cat "$TMPDIR/code-version-malformed"; exit; }
  printf '1.102.0\nabcdef0123456789\nx64\n'; exit
fi
if [ "$1" = --list-extensions ]; then
  [ ! -f "$TMPDIR/code-list-failure" ] || exit 52
  cat "$TMPDIR/code-state"; exit
fi
if [ "$1" = --install-extension ] && [ "$#" -eq 2 ]; then
  [ ! -f "$TMPDIR/code-install-failure" ] || exit 53
  [ -f "$TMPDIR/code-postcondition-failure" ] || printf '%s\n' "$2" >>"$TMPDIR/code-state"
  exit
fi
exit 54"#,
    );
}

#[test]
fn typed_integration_display_and_input_validation_are_operation_owned() {
    let docker_display = docker_local_log_step(Some("10m")).display();
    assert_eq!(docker_display, "workflow docker-local-log 10m");
    assert!(!docker_display.contains("docker-daemon.lock"));
    assert_eq!(
        vscode_extension_step(&["rust-lang.rust-analyzer", "ms-vscode.cpptools"]).display(),
        "workflow vscode-extension-set rust-lang.rust-analyzer ms-vscode.cpptools"
    );
    for invalid in [Some("0m"), Some("10M"), Some("1.5g"), Some("m")] {
        assert!(operations::DockerLocalLogOperation::new(invalid.map(str::to_owned)).is_err());
    }
    for invalid in [
        Vec::<String>::new(),
        vec!["rust-analyzer".into()],
        vec!["publisher.extension.extra".into()],
        vec!["publisher.extension".into(), "publisher.extension".into()],
    ] {
        assert!(operations::VsCodeExtensionOperation::new(invalid).is_err());
    }
}

#[test]
fn product_group_operations_use_exact_state_and_converge_without_newgrp() {
    for (operation, product, version, group) in [
        (
            operations::Operation::DockerGroup,
            "docker",
            "Docker version 28.3.2, build abcdef0",
            "docker",
        ),
        (
            operations::Operation::VirtualBoxGroup,
            "VBoxManage",
            "7.1.10r169112",
            "vboxusers",
        ),
    ] {
        let host = Host::new();
        configure_group_fakes(&host, product, version, group);
        fs::write(host._dir.path().join("tmp/group-state"), "1000 100\n").unwrap();

        host.run_ok(&Step::workflow(operation.clone()));
        host.run_ok(&Step::workflow(operation));

        let log = host.log();
        assert!(log.contains(&format!("{product} <--version>")), "{log}");
        assert!(
            log.contains(&format!(
                "getent <passwd> <{}>",
                rustix::process::geteuid().as_raw()
            )),
            "{log}"
        );
        assert!(log.contains(&format!("getent <group> <{group}>")), "{log}");
        assert!(log.contains("id <-G> <--> <tester>"), "{log}");
        assert!(
            log.contains(&format!("sudo <groupadd> <--system> <{group}>")),
            "{log}"
        );
        assert_eq!(
            log.matches(&format!("sudo <usermod> <-aG> <{group}> <--> <tester>"))
                .count(),
            1,
            "{log}"
        );
        assert!(!log.contains("newgrp"), "{log}");
        assert!(!log.contains("apt"), "{log}");
    }
}

#[test]
fn product_group_exact_membership_rejects_similar_and_malformed_state() {
    let host = Host::new();
    configure_group_fakes(
        &host,
        "docker",
        "Docker version 28.3.2, build abcdef0",
        "docker",
    );
    fs::write(host._dir.path().join("tmp/group-exists"), "").unwrap();
    fs::write(host._dir.path().join("tmp/group-state"), "1000 9970\n").unwrap();
    host.run_ok(&Step::workflow(operations::Operation::DockerGroup));
    assert!(host
        .log()
        .contains("sudo <usermod> <-aG> <docker> <--> <tester>"));

    let malformed = Host::new();
    configure_group_fakes(
        &malformed,
        "docker",
        "Docker version 28.3.2, build abcdef0",
        "docker",
    );
    fs::write(malformed._dir.path().join("tmp/group-exists"), "").unwrap();
    fs::write(
        malformed._dir.path().join("tmp/group-state"),
        "1000 997 997\n",
    )
    .unwrap();
    assert!(!malformed
        .run(&Step::workflow(operations::Operation::DockerGroup))
        .status
        .success());
    assert!(!malformed.log().contains("sudo <usermod>"));

    for (marker, record) in [
        (
            "passwd-malformed",
            "tester-helper:x:1000:1000:Tester:/home/tester:/bin/bash\n",
        ),
        ("group-malformed", "docker-helper:x:997:tester\n"),
    ] {
        let host = Host::new();
        configure_group_fakes(
            &host,
            "docker",
            "Docker version 28.3.2, build abcdef0",
            "docker",
        );
        fs::write(host._dir.path().join("tmp/group-exists"), "").unwrap();
        fs::write(host._dir.path().join("tmp/group-state"), "1000 100\n").unwrap();
        fs::write(host._dir.path().join("tmp").join(marker), record).unwrap();
        assert!(!host
            .run(&Step::workflow(operations::Operation::DockerGroup))
            .status
            .success());
        assert!(!host.log().contains("sudo <usermod>"));
    }
}

#[test]
fn product_group_failures_stop_at_the_failed_boundary_and_validate_user_first() {
    for marker in [
        "passwd-query-failure",
        "group-query-failure",
        "groupadd-failure",
        "usermod-failure",
        "postcondition-failure",
    ] {
        let host = Host::new();
        configure_group_fakes(
            &host,
            "docker",
            "Docker version 28.3.2, build abcdef0",
            "docker",
        );
        fs::write(host._dir.path().join("tmp/group-state"), "1000 100\n").unwrap();
        fs::write(host._dir.path().join("tmp").join(marker), "").unwrap();
        let output = host.run(&Step::workflow(operations::Operation::DockerGroup));
        assert!(!output.status.success(), "{marker}: {}", host.log());
    }

    let spoofed = Host::new();
    configure_group_fakes(
        &spoofed,
        "docker",
        "Docker version 28.3.2, build abcdef0",
        "docker",
    );
    fs::write(spoofed._dir.path().join("tmp/group-state"), "1000 100\n").unwrap();
    spoofed
        .execute_operation_as(&operations::Operation::DockerGroup, "root")
        .unwrap();
    let log = spoofed.log();
    assert!(
        log.contains(&format!(
            "getent <passwd> <{}>",
            rustix::process::geteuid().as_raw()
        )),
        "{log}"
    );
    assert!(
        log.contains("sudo <usermod> <-aG> <docker> <--> <tester>"),
        "{log}"
    );
    assert!(!log.contains("<root>"), "{log}");
}

#[test]
fn product_group_uses_numeric_nss_state_without_validating_unrelated_names() {
    let host = Host::new();
    configure_group_fakes(
        &host,
        "docker",
        "Docker version 28.3.2, build abcdef0",
        "docker",
    );
    fs::write(host._dir.path().join("tmp/group-exists"), "").unwrap();
    fs::write(
        host._dir.path().join("tmp/group-malformed"),
        "docker:x:997:UPPER.Name,DOMAIN\\User,user with spaces\n",
    )
    .unwrap();
    fs::write(host._dir.path().join("tmp/group-state"), "1000\t997\n").unwrap();

    host.run_ok(&Step::workflow(operations::Operation::DockerGroup));
    assert!(!host.log().contains("sudo <usermod>"), "{}", host.log());

    for state in ["", "1000 nope\n", "1000 997 997\n", "1000 +997\n"] {
        let malformed = Host::new();
        configure_group_fakes(
            &malformed,
            "docker",
            "Docker version 28.3.2, build abcdef0",
            "docker",
        );
        fs::write(malformed._dir.path().join("tmp/group-exists"), "").unwrap();
        fs::write(malformed._dir.path().join("tmp/group-state"), state).unwrap();
        assert!(!malformed
            .run(&Step::workflow(operations::Operation::DockerGroup))
            .status
            .success());
        assert!(!malformed.log().contains("sudo <usermod>"));
    }
}

#[test]
fn docker_local_log_preserves_unrelated_json_and_is_retry_safe() {
    let host = Host::new();
    host.fake(
        "docker",
        "printf 'docker <%s>\\n' \"$*\" >>\"$LOG\"; printf 'Docker version 28.3.2, build abcdef0\\n'",
    );
    host.atomic_sudo();
    let destination = host.root.join("etc/docker/daemon.json");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(
        &destination,
        br#"{"data-root":"/srv/docker","log-driver":"json-file","log-opts":{"labels":"service","max-size":"5m"}}"#,
    )
    .unwrap();
    let step = docker_local_log_step(None);

    host.run_ok(&step);
    host.run_ok(&step);

    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
    assert_eq!(value["data-root"], "/srv/docker");
    assert_eq!(value["log-driver"], "local");
    assert_eq!(value["log-opts"]["labels"], "service");
    assert_eq!(value["log-opts"]["max-size"], "5m");
    let log = host.log();
    assert_eq!(
        log.matches("sudo install -o root -g root -m 0644 --")
            .count(),
        1,
        "{log}"
    );
    assert!(
        log.contains("sudo stat --format=%f -- /etc/docker/daemon.json"),
        "{log}"
    );
    assert!(log.contains("sudo cat -- /etc/docker/daemon.json"), "{log}");
    assert!(!log.contains("restart"), "{log}");
    assert!(!log.contains("systemctl"), "{log}");
}

#[test]
fn docker_local_log_handles_missing_file_and_requested_max_size() {
    let host = Host::new();
    host.fake(
        "docker",
        "printf 'docker <%s>\\n' \"$*\" >>\"$LOG\"; printf 'Docker version 28.3.2, build abcdef0\\n'",
    );
    host.atomic_sudo();
    host.run_ok(&docker_local_log_step(Some("10m")));
    let destination = host.root.join("etc/docker/daemon.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(destination).unwrap()).unwrap();
    assert_eq!(value["log-driver"], "local");
    assert_eq!(value["log-opts"]["max-size"], "10m");
    let log = host.log();
    assert!(
        log.contains("sudo test ! -e /etc/docker/daemon.json"),
        "{log}"
    );
    assert!(
        log.contains("sudo test ! -L /etc/docker/daemon.json"),
        "{log}"
    );
}

#[test]
fn docker_local_log_rejects_hostile_existing_state_before_publication() {
    for (name, contents, kind) in [
        ("invalid-json", Some(b"{".as_slice()), "file"),
        ("non-utf8", Some(b"{\"key\":\"\xff\"}".as_slice()), "file"),
        ("non-object", Some(b"[]".as_slice()), "file"),
        (
            "non-object-log-opts",
            Some(br#"{"log-opts":[]}"#.as_slice()),
            "file",
        ),
        ("directory", None, "directory"),
        ("symlink", None, "symlink"),
        ("fifo", None, "fifo"),
    ] {
        let host = Host::new();
        host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\\n'");
        host.atomic_sudo();
        let destination = host.root.join("etc/docker/daemon.json");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        match kind {
            "file" => fs::write(&destination, contents.unwrap()).unwrap(),
            "directory" => fs::create_dir(&destination).unwrap(),
            "symlink" => symlink("missing-target", &destination).unwrap(),
            "fifo" => {
                assert!(Command::new("mkfifo")
                    .arg(&destination)
                    .status()
                    .unwrap()
                    .success())
            }
            _ => unreachable!(),
        }
        let output = host.run(&docker_local_log_step(Some("10m")));
        assert!(!output.status.success(), "{name}");
        assert!(
            !host.log().contains("sudo install -o root"),
            "{name}: {}",
            host.log()
        );
    }
}

#[test]
fn docker_publication_failures_preserve_old_bytes_and_postcondition_is_required() {
    for failure in ["mkdir", "stage", "sync", "rename"] {
        let host = Host::new();
        host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\\n'");
        host.atomic_sudo();
        let destination = host.root.join("etc/docker/daemon.json");
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let old = br#"{"keep":true}"#;
        fs::write(&destination, old).unwrap();
        fs::write(host._dir.path().join("tmp/publication-failure"), failure).unwrap();
        assert!(!host
            .run(&docker_local_log_step(Some("10m")))
            .status
            .success());
        assert_eq!(fs::read(destination).unwrap(), old, "{failure}");
        assert_lock_released(&host.root.join("run/cozydot/docker-daemon.lock"));
    }

    let host = Host::new();
    host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\\n'");
    host.atomic_sudo();
    let destination = host.root.join("etc/docker/daemon.json");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"{}").unwrap();
    fs::write(
        host._dir
            .path()
            .join("tmp/publication-postcondition-failure"),
        "",
    )
    .unwrap();
    assert!(!host.run(&docker_local_log_step(None)).status.success());
    assert_lock_released(&host.root.join("run/cozydot/docker-daemon.lock"));
}

#[test]
fn docker_lock_failures_precede_reads_and_parent_sync_failure_keeps_publication() {
    let host = Host::new();
    host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\n'");
    host.atomic_sudo();
    let destination = host.root.join("etc/docker/daemon.json");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"{\"keep\":true}").unwrap();
    fs::write(
        host._dir.path().join("tmp/publication-failure"),
        "lock-setup",
    )
    .unwrap();

    assert!(!host.run(&docker_local_log_step(None)).status.success());
    assert_eq!(fs::read(&destination).unwrap(), b"{\"keep\":true}");
    let log = host.log();
    assert!(log.contains(
        "sudo cp --no-clobber --no-target-directory -- /dev/null /run/cozydot/docker-daemon.lock"
    ));
    assert!(!log.contains("sudo stat --format=%f -- /etc/docker/daemon.json"));
    assert!(!log.contains("sudo cat -- /etc/docker/daemon.json"));
    assert!(!log.contains("/usr/bin/flock"));

    let host = Host::new();
    host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\n'");
    host.atomic_sudo();
    let destination = host.root.join("etc/docker/daemon.json");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"{}").unwrap();
    fs::write(
        host._dir.path().join("tmp/publication-failure"),
        "parent-sync",
    )
    .unwrap();

    assert!(!host.run(&docker_local_log_step(None)).status.success());
    let published_bytes = fs::read(&destination).unwrap();
    let published: serde_json::Value = serde_json::from_slice(&published_bytes).unwrap();
    assert_eq!(published["log-driver"], "local");
    assert!(destination.exists());

    fs::remove_file(host._dir.path().join("tmp/publication-failure")).unwrap();
    host.run_ok(&docker_local_log_step(None));
    assert_eq!(fs::read(&destination).unwrap(), published_bytes);
    assert_lock_released(&host.root.join("run/cozydot/docker-daemon.lock"));
    assert_eq!(
        host.log()
            .lines()
            .filter(|line| *line == "sudo sync -- /etc/docker")
            .count(),
        2,
        "{}",
        host.log()
    );
}

#[test]
fn docker_lock_open_symlink_and_type_failures_precede_config_reads() {
    for failure in ["open", "symlink", "type"] {
        let host = Host::new();
        host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\n'");
        host.atomic_sudo();
        let configured_lock = host.root.join("run/cozydot/docker-daemon.lock");
        let open_path = match failure {
            "open" => host.root.join("missing/docker-daemon.lock"),
            "symlink" => {
                let path = host.root.join("open-path-symlink");
                symlink(&configured_lock, &path).unwrap();
                path
            }
            "type" => {
                fs::create_dir_all(&configured_lock).unwrap();
                configured_lock.clone()
            }
            _ => unreachable!(),
        };
        let operation = operations::Operation::DockerLocalLog(
            operations::DockerLocalLogOperation::new(None).unwrap(),
        );

        assert!(host
            .execute_operation_with_lock(&operation, &open_path)
            .is_err());
        let log = host.log();
        assert!(
            !log.contains("sudo stat --format=%f -- /etc/docker/daemon.json"),
            "{failure}: {log}"
        );
        assert!(
            !log.contains("sudo cat -- /etc/docker/daemon.json"),
            "{failure}: {log}"
        );
    }
}

#[test]
fn docker_lock_setup_does_not_follow_or_replace_a_symlink() {
    let host = Host::new();
    host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\n'");
    host.atomic_sudo();
    let lock = host.root.join("run/cozydot/docker-daemon.lock");
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let referent = host.root.join("foreign-lock-target");
    fs::write(&referent, b"foreign bytes").unwrap();
    let referent_inode = fs::metadata(&referent).unwrap().ino();
    symlink(&referent, &lock).unwrap();

    assert!(!host.run(&docker_local_log_step(None)).status.success());
    assert_eq!(fs::read(&referent).unwrap(), b"foreign bytes");
    assert_eq!(fs::metadata(&referent).unwrap().ino(), referent_inode);
    assert_eq!(fs::read_link(&lock).unwrap(), referent);
    let log = host.log();
    assert!(log.contains(
        "sudo cp --no-clobber --no-target-directory -- /dev/null /run/cozydot/docker-daemon.lock"
    ));
    assert!(!log.contains("sudo chown"));
    assert!(!log.contains("sudo cat -- /etc/docker/daemon.json"));
}

#[test]
fn two_docker_operations_serialize_on_the_descriptor_held_inode() {
    let host = Host::new();
    host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\n'");
    host.atomic_sudo();
    let destination = host.root.join("etc/docker/daemon.json");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"{}\n").unwrap();
    fs::write(
        host._dir.path().join("tmp/publication-pause-after-read"),
        "",
    )
    .unwrap();

    let spawn_operation = |max_size: Option<&str>| {
        let home = host.home.clone();
        let log = host.log.clone();
        let root = host.root.clone();
        let temp = host._dir.path().join("tmp");
        let path = format!("{}:/usr/bin:/bin", host.bin.display());
        let max_size = max_size.map(str::to_owned);
        std::thread::spawn(move || {
            let docker_lock = root.join("run/cozydot/docker-daemon.lock");
            let env = [
                ("HOME".into(), home.into_os_string()),
                ("USER".into(), "tester".into()),
                ("LOG".into(), log.into_os_string()),
                ("ROOT".into(), root.into_os_string()),
                ("TMPDIR".into(), temp.into_os_string()),
                ("PATH".into(), path.into()),
            ];
            let operation = operations::Operation::DockerLocalLog(
                operations::DockerLocalLogOperation::new(max_size).unwrap(),
            );
            operations::execute_with_docker_lock_for_test(&operation, &env, &docker_lock)
        })
    };

    let first = spawn_operation(Some("10m"));
    let observed = host._dir.path().join("tmp/publication-read-observed");
    for _ in 0..500 {
        if observed.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(observed.exists());

    let second = spawn_operation(None);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(!second.is_finished(), "second operation bypassed the flock");
    assert_eq!(
        fs::read_to_string(host._dir.path().join("tmp/publication-cat-count")).unwrap(),
        "1"
    );

    fs::write(host._dir.path().join("tmp/publication-read-release"), "").unwrap();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    let state: serde_json::Value = serde_json::from_slice(&fs::read(destination).unwrap()).unwrap();
    assert_eq!(state["log-driver"], "local");
    assert_eq!(state["log-opts"]["max-size"], "10m");
}

#[test]
fn docker_transaction_lock_preserves_a_contending_unrelated_edit() {
    let host = Host::new();
    host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\n'");
    host.atomic_sudo();
    let destination = host.root.join("etc/docker/daemon.json");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"{\"existing\":true}\n").unwrap();
    fs::write(
        host._dir.path().join("tmp/publication-pause-after-read"),
        "",
    )
    .unwrap();

    let home = host.home.clone();
    let log = host.log.clone();
    let root = host.root.clone();
    let temp = host._dir.path().join("tmp");
    let path = format!("{}:/usr/bin:/bin", host.bin.display());
    let operation = operations::Operation::DockerLocalLog(
        operations::DockerLocalLogOperation::new(Some("10m".into())).unwrap(),
    );
    let worker = std::thread::spawn(move || {
        let docker_lock = root.join("run/cozydot/docker-daemon.lock");
        let env = [
            ("HOME".into(), home.into_os_string()),
            ("USER".into(), "spoofed".into()),
            ("LOG".into(), log.into_os_string()),
            ("ROOT".into(), root.into_os_string()),
            ("TMPDIR".into(), temp.into_os_string()),
            ("PATH".into(), path.into()),
        ];
        operations::execute_with_docker_lock_for_test(&operation, &env, &docker_lock)
    });

    let observed = host._dir.path().join("tmp/publication-read-observed");
    for _ in 0..500 {
        if observed.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        observed.exists(),
        "Docker operation did not reach locked read"
    );

    let lock = host.root.join("run/cozydot/docker-daemon.lock");
    let contended = host._dir.path().join("tmp/editor-contended");
    let script = r#"
if /usr/bin/flock --nonblock -- "$LOCK" /bin/true; then exit 90; fi
touch "$CONTENDED"
/usr/bin/flock --exclusive -- "$LOCK" /bin/bash -c '
  sed '\''1s/{/{"concurrent-edit":true,/'\'' "$DEST" >"$DEST.edit"
  mv -f -- "$DEST.edit" "$DEST"
'
"#;
    let mut editor = Command::new("/bin/bash")
        .args(["-euo", "pipefail", "-c", script])
        .env("LOCK", &lock)
        .env("CONTENDED", &contended)
        .env("DEST", &destination)
        .spawn()
        .unwrap();
    for _ in 0..500 {
        if contended.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        contended.exists(),
        "simulated editor did not observe lock contention"
    );
    assert!(editor.try_wait().unwrap().is_none());
    fs::write(host._dir.path().join("tmp/publication-read-release"), "").unwrap();

    worker.join().unwrap().unwrap();
    assert!(editor.wait().unwrap().success());
    let final_state: serde_json::Value =
        serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
    assert_eq!(final_state["existing"], true);
    assert_eq!(final_state["log-driver"], "local");
    assert_eq!(final_state["log-opts"]["max-size"], "10m");
    assert_eq!(final_state["concurrent-edit"], true);
}

#[test]
fn integration_preflights_fail_before_state_or_mutation() {
    let docker = Host::new();
    docker.fake("docker", "printf 'not a docker version\\n'");
    docker.atomic_sudo();
    assert!(!docker.run(&docker_local_log_step(None)).status.success());
    assert!(!docker.log().contains("sudo"), "{}", docker.log());

    let vscode = Host::new();
    configure_vscode_fake(&vscode, b"");
    fs::write(
        vscode._dir.path().join("tmp/code-version-malformed"),
        "bad\n",
    )
    .unwrap();
    assert!(!vscode
        .run(&vscode_extension_step(&["publisher.extension"]))
        .status
        .success());
    assert!(!vscode.log().contains("--list-extensions"));
}

#[test]
fn existing_product_preflight_rejects_missing_nonexecutable_nonzero_and_non_utf8_cli() {
    for mode in ["missing", "nonexecutable", "nonzero", "non-utf8"] {
        let host = Host::new();
        match mode {
            "missing" => {}
            "nonexecutable" => {
                host.fake("docker", "printf 'Docker version 28.3.2, build abcdef0\\n'");
                fs::set_permissions(host.bin.join("docker"), fs::Permissions::from_mode(0o644))
                    .unwrap();
            }
            "nonzero" => host.fake("docker", "exit 42"),
            "non-utf8" => host.fake("docker", "printf '\\377\\n'"),
            _ => unreachable!(),
        }
        host.atomic_sudo();
        let operation = operations::Operation::DockerLocalLog(
            operations::DockerLocalLogOperation::new(None).unwrap(),
        );
        assert!(
            host.execute_operation_as_with_path(
                &operation,
                "tester",
                host.bin.display().to_string()
            )
            .is_err(),
            "{mode}"
        );
        assert!(!host.log().contains("sudo"), "{mode}: {}", host.log());
    }
}

#[test]
fn vscode_extension_set_selects_exact_missing_ids_in_order_and_converges() {
    let host = Host::new();
    configure_vscode_fake(&host, b"publisher.present\npublisher.extension-helper\n");
    let step =
        vscode_extension_step(&["publisher.present", "publisher.extension", "other.missing"]);
    host.run_ok(&step);
    host.run_ok(&step);
    let log = host.log();
    let first = log
        .find("code <--install-extension> <publisher.extension>")
        .unwrap();
    let second = log
        .find("code <--install-extension> <other.missing>")
        .unwrap();
    assert!(first < second, "{log}");
    assert_eq!(
        log.matches("code <--install-extension>").count(),
        2,
        "{log}"
    );
    assert_eq!(log.matches("code <--list-extensions>").count(), 3, "{log}");
}

#[test]
fn vscode_complete_state_is_one_inspection_after_preflight() {
    let host = Host::new();
    configure_vscode_fake(&host, b"publisher.extension\n");
    host.run_ok(&vscode_extension_step(&["publisher.extension"]));
    let log = host.log();
    assert_eq!(log.matches("code <--version>").count(), 1, "{log}");
    assert_eq!(log.matches("code <--list-extensions>").count(), 1, "{log}");
    assert!(!log.contains("--install-extension"), "{log}");
}

#[test]
fn vscode_operation_canonicalizes_direct_input_and_cli_state() {
    let operation =
        operations::VsCodeExtensionOperation::new(vec!["Publisher.Extension-Name".into()]).unwrap();
    assert_eq!(
        operations::Operation::VsCodeExtensionSet(operation.clone()).display_args(),
        ["vscode-extension-set", "publisher.extension-name"]
    );

    let present = Host::new();
    configure_vscode_fake(&present, b"publisher.extension-name\n");
    present.run_ok(&Step::workflow(operations::Operation::VsCodeExtensionSet(
        operation.clone(),
    )));
    assert!(!present.log().contains("--install-extension"));

    let absent = Host::new();
    configure_vscode_fake(&absent, b"");
    absent.run_ok(&Step::workflow(operations::Operation::VsCodeExtensionSet(
        operation,
    )));
    assert!(
        absent
            .log()
            .contains("code <--install-extension> <publisher.extension-name>"),
        "{}",
        absent.log()
    );

    for invalid in ["_publisher.extension", "publisher.ext_name", ".extension"] {
        assert!(operations::VsCodeExtensionOperation::new(vec![invalid.into()]).is_err());
    }
    assert!(operations::VsCodeExtensionOperation::new(vec![
        "Publisher.Extension".into(),
        "publisher.extension".into(),
    ])
    .is_err());
}

#[test]
fn vscode_rejects_malformed_and_case_fold_duplicate_cli_state() {
    for state in [
        b"_publisher.extension\n".as_slice(),
        b"publisher.ext_name\n",
        b"Publisher.Extension\npublisher.extension\n",
    ] {
        let host = Host::new();
        configure_vscode_fake(&host, state);
        assert!(!host
            .run(&vscode_extension_step(&["publisher.extension"]))
            .status
            .success());
        assert!(!host.log().contains("--install-extension"));
    }
}

#[test]
fn vscode_state_install_and_postcondition_failures_propagate() {
    for (marker, state) in [
        ("code-list-failure", b"".as_slice()),
        ("code-install-failure", b"".as_slice()),
        ("code-postcondition-failure", b"".as_slice()),
    ] {
        let host = Host::new();
        configure_vscode_fake(&host, state);
        fs::write(host._dir.path().join("tmp").join(marker), "").unwrap();
        assert!(!host
            .run(&vscode_extension_step(&["publisher.extension"]))
            .status
            .success());
    }

    for state in [b"malformed\n".as_slice(), b"publisher.extension\xff\n"] {
        let host = Host::new();
        configure_vscode_fake(&host, state);
        let output = host.run(&vscode_extension_step(&["publisher.extension"]));
        assert!(!output.status.success());
        assert!(!host.log().contains("--install-extension"));
    }
}

#[test]
fn schema_v1_cargo_ensure_installs_only_missing_packages_in_order_and_is_retry_safe() {
    let host = Host::new();
    configure_cargo_package_fakes(&host, "present v1.2.3:\n    present\n");
    let step = cargo_package_step(
        &["missing_one", "present", "missing-two"],
        operations::CargoPackageMode::EnsurePresent,
    );

    host.run_ok(&step);
    host.run_ok(&step);

    let log = host.log();
    assert_eq!(log.matches("cargo <install> <--list>").count(), 3, "{log}");
    assert!(!log.contains("cargo <install> <cargo-binstall>"), "{log}");
    assert_eq!(
        log.matches("cargo-binstall <--no-confirm> <missing_one> <missing-two>")
            .count(),
        1,
        "{log}"
    );
    assert!(
        !log.contains("cargo-binstall <--no-confirm> <present>"),
        "{log}"
    );
}

#[test]
fn schema_v1_cargo_ignores_display_oriented_sources_when_installing_missing_registry_package() {
    let host = Host::new();
    configure_cargo_package_fakes(
        &host,
        "path-probe v1.2.3 (/tmp/hermes-cargo-probe):\n    path-probe\ngit-probe v2.3.4 (https://github.com/example/repo (main)):\n    git-probe\n",
    );

    host.run_ok(&cargo_package_step(
        &["bat"],
        operations::CargoPackageMode::EnsurePresent,
    ));

    let log = host.log();
    assert!(log.contains("cargo-binstall <--no-confirm> <bat>"), "{log}");
    assert_eq!(log.matches("cargo <install> <--list>").count(), 2, "{log}");
}

#[test]
fn schema_v1_cargo_installed_ensure_is_a_single_query_noop() {
    let host = Host::new();
    configure_cargo_package_fakes(&host, "bat v0.25.0:\n    bat\nripgrep v14.1.0:\n    rg\n");
    host.run_ok(&cargo_package_step(
        &["bat", "ripgrep"],
        operations::CargoPackageMode::EnsurePresent,
    ));
    let log = host.log();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        ["cargo <install> <--list>"]
    );
}

#[test]
fn schema_v1_cargo_update_forces_exactly_the_configured_batch() {
    let host = Host::new();
    configure_cargo_package_fakes(
        &host,
        "unrelated v9.0.0:\n    unrelated\nbat v0.1.0:\n    bat\nripgrep v0.1.0:\n    rg\n",
    );
    host.fake(
        "cargo-binstall",
        "{ printf 'cargo-binstall'; printf ' <%s>' \"$@\"; printf '\\n'; } >>\"$LOG\"",
    );
    host.run_ok(&cargo_package_step(
        &["ripgrep", "bat"],
        operations::CargoPackageMode::UpdateCurrent,
    ));
    let log = host.log();
    assert!(
        log.contains("cargo-binstall <--no-confirm> <--force> <ripgrep> <bat>"),
        "{log}"
    );
    assert!(!log.contains("--force unrelated"), "{log}");
    assert_eq!(log.matches("cargo <install> <--list>").count(), 2, "{log}");
}

#[test]
fn schema_v1_cargo_rejects_invalid_duplicate_and_injection_inputs_before_execution() {
    for packages in [
        Vec::<String>::new(),
        vec!["bat".into(), "bat".into()],
        vec!["bat --locked".into()],
        vec!["bat;touch-pwned".into()],
        vec!["--force".into()],
    ] {
        assert!(operations::CargoPackageOperation::new(
            packages,
            operations::CargoPackageMode::EnsurePresent
        )
        .is_err());
    }
}

#[test]
fn schema_v1_cargo_state_failures_are_fatal() {
    for body in [
        "printf 'malformed\\n'",
        "printf 'bat v01.2.3:\\n'",
        "printf '\\377'",
        "printf 'fatal\\n' >&2; exit 61",
    ] {
        let host = Host::new();
        host.fake(
            "cargo",
            &format!(
                "if [ \"$1\" = install ] && [ \"$2\" = --list ]; then {body}; else exit 62; fi"
            ),
        );
        fs::create_dir_all(host.home.join(".cargo/bin")).unwrap();
        fs::rename(host.bin.join("cargo"), host.home.join(".cargo/bin/cargo")).unwrap();
        assert!(
            !host
                .run(&cargo_package_step(
                    &["bat"],
                    operations::CargoPackageMode::EnsurePresent,
                ))
                .status
                .success(),
            "body unexpectedly succeeded: {body}"
        );
    }
}

#[test]
fn schema_v1_cargo_requires_an_executable_cargo_without_using_real_state() {
    let host = Host::new();
    let output = host.run_with_path(
        &cargo_package_step(&["bat"], operations::CargoPackageMode::EnsurePresent),
        host.bin.display().to_string(),
    );
    assert!(!output.status.success());
}

#[test]
fn schema_v1_cargo_propagates_mutation_and_postcondition_failures() {
    for failure in ["mutation", "postcondition"] {
        let host = Host::new();
        configure_cargo_package_fakes(&host, "");
        match failure {
            "mutation" => {
                fs::write(host._dir.path().join("tmp/cargo-mutation-failure"), b"1").unwrap()
            }
            "postcondition" => {
                host.fake(
                    "cargo-binstall",
                    "printf 'cargo-binstall %s\\n' \"$*\" >>\"$LOG\"",
                );
                fs::rename(
                    host.bin.join("cargo-binstall"),
                    host.home.join(".cargo/bin/cargo-binstall"),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let output = host.run(&cargo_package_step(
            &["bat"],
            operations::CargoPackageMode::EnsurePresent,
        ));
        assert!(!output.status.success(), "{failure} unexpectedly succeeded");
    }
}

#[test]
fn schema_v1_cargo_bootstrap_must_publish_an_executable_binstall() {
    let host = Host::new();
    host.fake(
        "cargo",
        r#"if [ "$1" = install ] && [ "$2" = --list ]; then exit; fi
if [ "$1" = install ] && [ "$2" = cargo-binstall ]; then exit; fi
exit 63"#,
    );
    fs::create_dir_all(host.home.join(".cargo/bin")).unwrap();
    fs::rename(host.bin.join("cargo"), host.home.join(".cargo/bin/cargo")).unwrap();
    assert!(!host
        .run(&cargo_package_step(
            &["bat"],
            operations::CargoPackageMode::EnsurePresent,
        ))
        .status
        .success());
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
fn flatpak_flathub_absent_converges_and_second_apply_does_not_add_again() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user remotes --show-disabled --columns=name,url,options,filter')
    [ ! -f "$TMPDIR/flathub-added" ] || printf 'flathub\thttps://dl.flathub.org/repo/\tcollection-id=org.flathub.Stable\t-\n'
    ;;
  '--user remote-add flathub https://dl.flathub.org/repo/flathub.flatpakrepo')
    touch "$TMPDIR/flathub-added"
    ;;
  '--user remote-modify --use-for-deps flathub') ;;
  *) exit 42 ;;
esac"#,
    );
    let step = Step::workflow(operations::Operation::FlatpakEnsureFlathub);
    host.run_ok(&step);
    host.run_ok(&step);
    assert_eq!(
        host.log(),
        "flatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-add flathub https://dl.flathub.org/repo/flathub.flatpakrepo\nflatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-modify --use-for-deps flathub\nflatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-modify --use-for-deps flathub\nflatpak --user remotes --show-disabled --columns=name,url,options,filter\n"
    );
    assert_eq!(host.log().matches(" remote-add ").count(), 1);
}

#[test]
fn flatpak_flathub_existing_remote_explicitly_enables_dependency_use() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user remotes --show-disabled --columns=name,url,options,filter')
    printf 'flathub\thttps://dl.flathub.org/repo/\tgpg-verify-summary,collection-id=org.flathub.Stable\t-\n'
    ;;
  '--user remote-modify --use-for-deps flathub') ;;
  *) exit 42 ;;
esac"#,
    );
    host.run_ok(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
    assert_eq!(
        host.log(),
        "flatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-modify --use-for-deps flathub\nflatpak --user remotes --show-disabled --columns=name,url,options,filter\n"
    );
}

#[test]
fn flatpak_flathub_validates_state_immediately_after_add() {
    for (name, record) in [
        ("wrong-url", "flathub\thttps://example.invalid/repo\t\t-"),
        (
            "disabled",
            "flathub\thttps://dl.flathub.org/repo/\tdisabled\t-",
        ),
        (
            "no-gpg-verification",
            "flathub\thttps://dl.flathub.org/repo/\tno-gpg-verify\t-",
        ),
        (
            "no-enumeration",
            "flathub\thttps://dl.flathub.org/repo/\tno-enumerate\t-",
        ),
        (
            "filtered",
            "flathub\thttps://dl.flathub.org/repo/\t\t/etc/flatpak/flathub.filter",
        ),
    ] {
        let host = Host::new();
        host.fake(
            "flatpak",
            &format!(
                r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user remotes --show-disabled --columns=name,url,options,filter')
    [ ! -f "$TMPDIR/flathub-added" ] || printf '{record}\n'
    ;;
  '--user remote-add flathub https://dl.flathub.org/repo/flathub.flatpakrepo')
    touch "$TMPDIR/flathub-added"
    ;;
  *) exit 42 ;;
esac"#
            ),
        );
        let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
        assert!(!output.status.success(), "{name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Flathub remote mismatch"),
            "{name}: {stderr}"
        );
        assert!(stderr.contains("Repair or remove"), "{name}: {stderr}");
        assert_eq!(
            host.log(),
            "flatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-add flathub https://dl.flathub.org/repo/flathub.flatpakrepo\nflatpak --user remotes --show-disabled --columns=name,url,options,filter\n",
            "{name}"
        );
    }
}

#[test]
fn flatpak_flathub_add_success_without_publication_fails_before_modify() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user remotes --show-disabled --columns=name,url,options,filter') ;;
  '--user remote-add flathub https://dl.flathub.org/repo/flathub.flatpakrepo') ;;
  *) exit 42 ;;
esac"#,
    );
    let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("to exist after mutation"));
    assert_eq!(
        host.log(),
        "flatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-add flathub https://dl.flathub.org/repo/flathub.flatpakrepo\nflatpak --user remotes --show-disabled --columns=name,url,options,filter\n"
    );
}

#[test]
fn flatpak_flathub_concurrent_creation_fails_closed_at_add() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user remotes --show-disabled --columns=name,url,options,filter') ;;
  '--user remote-add flathub https://dl.flathub.org/repo/flathub.flatpakrepo') exit 43 ;;
  *) exit 42 ;;
esac"#,
    );
    let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Flathub remote ensure"));
    assert_eq!(
        host.log(),
        "flatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-add flathub https://dl.flathub.org/repo/flathub.flatpakrepo\n"
    );
}

#[test]
fn flatpak_flathub_remote_modify_failure_propagates_without_final_query() {
    let host = Host::new();
    host.fake(
        "flatpak",
        r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user remotes --show-disabled --columns=name,url,options,filter')
    printf 'flathub\thttps://dl.flathub.org/repo/\t\t-\n'
    ;;
  '--user remote-modify --use-for-deps flathub') exit 44 ;;
  *) exit 42 ;;
esac"#,
    );
    let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("dependency use enablement"));
    assert_eq!(
        host.log(),
        "flatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-modify --use-for-deps flathub\n"
    );
}

#[test]
fn flatpak_flathub_final_query_must_revalidate_the_remote() {
    for (name, final_query, error) in [
        ("absent", "true", "to exist after mutation"),
        (
            "wrong",
            "printf 'flathub\\thttps://example.invalid/repo\\t\\t-\\n'",
            "Flathub remote mismatch",
        ),
        (
            "fatal",
            "printf 'final query failed\\n' >&2; exit 45",
            "final query failed",
        ),
    ] {
        let host = Host::new();
        host.fake(
            "flatpak",
            &format!(
                r#"printf 'flatpak %s\n' "$*" >>"$LOG"
case "$*" in
  '--user remotes --show-disabled --columns=name,url,options,filter')
    if [ ! -f "$TMPDIR/modified" ]; then
      printf 'flathub\thttps://dl.flathub.org/repo/\t\t-\n'
    else
      {final_query}
    fi
    ;;
  '--user remote-modify --use-for-deps flathub') touch "$TMPDIR/modified" ;;
  *) exit 42 ;;
esac"#
            ),
        );
        let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
        assert!(!output.status.success(), "{name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(error), "{name}: {stderr}");
        assert_eq!(
            host.log(),
            "flatpak --user remotes --show-disabled --columns=name,url,options,filter\nflatpak --user remote-modify --use-for-deps flathub\nflatpak --user remotes --show-disabled --columns=name,url,options,filter\n",
            "{name}"
        );
    }
}

#[test]
fn flatpak_flathub_fails_before_mutation_on_bad_pre_query_state() {
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
            "printf 'flathub\\thttps://dl.flathub.org/repo/\\t-\\n'",
            "malformed per-user remote state",
        ),
        (
            "blank-record",
            "printf 'other\\thttps://example.test/repo\\t\\t-\\n\\nflathub\\thttps://dl.flathub.org/repo/\\t\\t-\\n'",
            "malformed per-user remote state",
        ),
        (
            "duplicate-name",
            "printf 'flathub\\thttps://dl.flathub.org/repo/\\t\\t-\\nflathub\\thttps://dl.flathub.org/repo/\\t\\t-\\n'",
            "duplicate per-user remote name",
        ),
        (
            "duplicate-option",
            "printf 'other\\thttps://example.test/repo\\tdisabled,disabled\\t-\\n'",
            "malformed per-user remote state",
        ),
        (
            "invalid-url",
            "printf 'other\\tnot-a-url\\t\\t-\\n'",
            "malformed per-user remote state",
        ),
    ] {
        let host = Host::new();
        host.fake(
            "flatpak",
            &format!("printf 'flatpak %s\\n' \"$*\" >>\"$LOG\"\n{body}"),
        );
        let output = host.run(&Step::workflow(operations::Operation::FlatpakEnsureFlathub));
        assert!(!output.status.success(), "{name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(error), "{name}: {stderr}");
        assert_eq!(
            host.log(),
            "flatpak --user remotes --show-disabled --columns=name,url,options,filter\n",
            "{name}"
        );
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
    assert!(publication_log.contains("sudo sync -- /etc/apt/sources.list.d"));
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

    fs::write(
        host._dir.path().join("tmp/publication-failure"),
        "parent-sync",
    )
    .unwrap();
    assert!(!host.run(&first).status.success());
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"deb [arch=amd64] https://example.test stable main\n"
    );
    assert!(destination.exists());
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

fn files_below(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
}

#[test]
fn managed_apt_migrates_both_formats_preserves_unrelated_state_and_reapplies_as_noop() {
    let host = Host::new();
    host.atomic_sudo();
    let apt = host.root.join("etc/apt");
    let keyring = host
        .root
        .join("usr/share/keyrings/ubuntu-archive-keyring.gpg");
    fs::create_dir_all(apt.join("sources.list.d")).unwrap();
    fs::create_dir_all(keyring.parent().unwrap()).unwrap();
    fs::write(&keyring, b"archive key").unwrap();
    fs::write(
        apt.join("sources.list"),
        b"# keep comment\ndeb http://archive.ubuntu.com/ubuntu noble main\ndeb-src http://archive.ubuntu.com/ubuntu noble main\n",
    )
    .unwrap();
    fs::write(
        apt.join("sources.list.d/base.sources"),
        b"Types: deb deb-src\nURIs: https://security.ubuntu.com/ubuntu\nSuites: noble-security\nComponents: main\nX-Unknown: keep\n",
    )
    .unwrap();
    let vendor = b"Types: deb\nURIs: https://vendor.example/apt\nSuites: noble\nComponents: main\nX-Repolib-Name: Vendor\n";
    fs::write(apt.join("sources.list.d/vendor.sources"), vendor).unwrap();
    let step = managed_apt_sources_step("ubuntu", "noble", Architecture::Amd64, &["main"]);

    host.run_ok(&step);
    assert_eq!(
        fs::read_to_string(apt.join("sources.list")).unwrap(),
        "# keep comment\ndeb-src http://archive.ubuntu.com/ubuntu noble main\n"
    );
    let migrated = fs::read_to_string(apt.join("sources.list.d/base.sources")).unwrap();
    assert!(migrated.contains("Types: deb-src"));
    assert!(migrated.contains("X-Unknown: keep"));
    assert_eq!(
        fs::read(apt.join("sources.list.d/vendor.sources")).unwrap(),
        vendor
    );
    let owned = fs::read_to_string(apt.join("sources.list.d/cozydot-base.sources")).unwrap();
    assert!(owned.contains("URIs: https://archive.ubuntu.com/ubuntu"));
    assert!(owned.contains("URIs: https://security.ubuntu.com/ubuntu"));
    assert!(owned.contains("Architectures: amd64"));

    let backups = files_below(&host.root.join("var/lib/cozydot/apt-source-backups"));
    assert_eq!(backups.len(), 2, "{backups:?}");
    assert!(backups
        .iter()
        .all(|path| { fs::metadata(path).unwrap().permissions().mode() & 0o777 == 0o600 }));
    let owned_renames_before = host
        .log()
        .lines()
        .filter(|line| line.starts_with("sudo mv ") && line.ends_with(OWNED_APT_SOURCE))
        .count();
    host.run_ok(&step);
    let owned_renames_after = host
        .log()
        .lines()
        .filter(|line| line.starts_with("sudo mv ") && line.ends_with(OWNED_APT_SOURCE))
        .count();
    assert_eq!(owned_renames_before, 1);
    assert_eq!(owned_renames_after, owned_renames_before);
}

const OWNED_APT_SOURCE: &str = "/etc/apt/sources.list.d/cozydot-base.sources";

#[test]
fn managed_apt_partial_multi_file_failure_keeps_backups_and_retry_converges() {
    let host = Host::new();
    host.atomic_sudo();
    let apt = host.root.join("etc/apt");
    let keyring = host
        .root
        .join("usr/share/keyrings/debian-archive-keyring.gpg");
    fs::create_dir_all(apt.join("sources.list.d")).unwrap();
    fs::create_dir_all(keyring.parent().unwrap()).unwrap();
    fs::write(&keyring, b"archive key").unwrap();
    let base = b"deb https://deb.debian.org/debian trixie main\n";
    fs::write(apt.join("sources.list"), base).unwrap();
    fs::write(apt.join("sources.list.d/second.list"), base).unwrap();
    fs::write(
        host._dir.path().join("tmp/publication-failure"),
        "managed-second-rewrite",
    )
    .unwrap();
    let step = managed_apt_sources_step("debian", "trixie", Architecture::Amd64, &["main"]);

    assert!(!host.run(&step).status.success());
    assert_eq!(fs::read(apt.join("sources.list")).unwrap(), b"");
    assert_eq!(
        fs::read(apt.join("sources.list.d/second.list")).unwrap(),
        base
    );
    assert!(!apt.join("sources.list.d/cozydot-base.sources").exists());
    assert_eq!(
        files_below(&host.root.join("var/lib/cozydot/apt-source-backups")).len(),
        2
    );

    fs::remove_file(host._dir.path().join("tmp/publication-failure")).unwrap();
    host.run_ok(&step);
    assert_eq!(
        fs::read(apt.join("sources.list.d/second.list")).unwrap(),
        b""
    );
    assert!(apt.join("sources.list.d/cozydot-base.sources").is_file());
}

#[test]
fn managed_apt_keyring_preflight_fails_before_source_inspection_or_backup() {
    for keyring_kind in ["missing", "symlink"] {
        let host = Host::new();
        host.atomic_sudo();
        let apt = host.root.join("etc/apt");
        fs::create_dir_all(apt.join("sources.list.d")).unwrap();
        fs::write(
            apt.join("sources.list"),
            b"deb https://deb.debian.org/debian trixie main\n",
        )
        .unwrap();
        if keyring_kind == "symlink" {
            let directory = host.root.join("usr/share/keyrings");
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("foreign-keyring"), b"foreign").unwrap();
            symlink(
                "foreign-keyring",
                directory.join("debian-archive-keyring.gpg"),
            )
            .unwrap();
        }
        let step = managed_apt_sources_step("debian", "trixie", Architecture::Amd64, &["main"]);

        assert!(!host.run(&step).status.success(), "{keyring_kind}");
        assert_eq!(
            fs::read(apt.join("sources.list")).unwrap(),
            b"deb https://deb.debian.org/debian trixie main\n"
        );
        assert!(!host.root.join("var/lib/cozydot").exists());
        assert!(!host.log().contains("sudo find"), "{}", host.log());
    }
}

#[test]
fn apt_repository_converges_records_completion_and_reapplies_without_publication() {
    let host = Host::new();
    configure_key_fakes(&host);
    fs::write(host._dir.path().join("tmp/key-input"), "binary").unwrap();
    let operation = apt_repository_operation(
        "https://example.test/key",
        "https://example.test/apt/",
        "stable",
    );

    host.execute_operation_as(&operation, "tester").unwrap();
    let key = host.root.join("etc/apt/keyrings/cozydot-vendor-name.gpg");
    let source = host
        .root
        .join("etc/apt/sources.list.d/cozydot-vendor-name.list");
    let record = host
        .home
        .join(".local/state/cozydot/apt-repositories/vendor-name.json");
    assert_eq!(fs::read(&key).unwrap(), b"normalized-binary");
    assert_eq!(
        fs::read(&source).unwrap(),
        b"deb [arch=amd64 signed-by=/etc/apt/keyrings/cozydot-vendor-name.gpg] https://example.test/apt/ stable main\n"
    );
    assert!(fs::read_to_string(&record)
        .unwrap()
        .contains("\"status\":\"completed\""));
    assert_eq!(
        fs::metadata(&record).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let initial_links = host.log().matches("sudo ln --").count();
    assert_eq!(initial_links, 2, "{}", host.log());
    let record_inode = fs::metadata(&record).unwrap().ino();

    host.execute_operation_as(&operation, "tester").unwrap();
    assert_eq!(host.log().matches("sudo ln --").count(), initial_links);
    assert_eq!(fs::metadata(&record).unwrap().ino(), record_inode);
    assert_eq!(
        operations::Operation::AptRepository(match operation.clone() {
            operations::Operation::AptRepository(operation) => operation,
            _ => unreachable!(),
        })
        .display_args(),
        [
            "apt-repository",
            "Vendor_Name",
            "/etc/apt/keyrings/cozydot-vendor-name.gpg",
            "/etc/apt/sources.list.d/cozydot-vendor-name.list"
        ]
    );
}

#[test]
fn apt_repository_refuses_every_unmanaged_destination_kind_before_download() {
    for (destination_kind, kind) in [
        ("key", "file"),
        ("key", "directory"),
        ("key", "symlink"),
        ("key", "dangling-symlink"),
        ("source", "file"),
        ("source", "directory"),
        ("source", "symlink"),
        ("source", "dangling-symlink"),
    ] {
        let host = Host::new();
        configure_key_fakes(&host);
        fs::write(host._dir.path().join("tmp/key-input"), "binary").unwrap();
        let destination = if destination_kind == "key" {
            host.root.join("etc/apt/keyrings/cozydot-vendor-name.gpg")
        } else {
            host.root
                .join("etc/apt/sources.list.d/cozydot-vendor-name.list")
        };
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        match kind {
            "file" => fs::write(&destination, b"foreign").unwrap(),
            "directory" => fs::create_dir(&destination).unwrap(),
            "symlink" => {
                fs::write(destination.parent().unwrap().join("foreign"), b"foreign").unwrap();
                symlink("foreign", &destination).unwrap();
            }
            "dangling-symlink" => symlink("missing", &destination).unwrap(),
            _ => unreachable!(),
        }
        let operation = apt_repository_operation(
            "https://example.test/key",
            "https://example.test/apt/",
            "stable",
        );
        assert!(
            host.execute_operation_as(&operation, "tester").is_err(),
            "{destination_kind} {kind}"
        );
        assert!(
            !host.log().contains("curl "),
            "{destination_kind} {kind}: {}",
            host.log()
        );
        assert!(
            !host.log().contains("gpg "),
            "{destination_kind} {kind}: {}",
            host.log()
        );
        assert!(!host
            .home
            .join(".local/state/cozydot/apt-repositories/vendor-name.json")
            .exists());
    }
}

#[test]
fn apt_repository_pending_retries_updates_and_never_completes_early() {
    let host = Host::new();
    configure_key_fakes(&host);
    let input = host._dir.path().join("tmp/key-input");
    let record = host
        .home
        .join(".local/state/cozydot/apt-repositories/vendor-name.json");
    let first = apt_repository_operation(
        "https://example.test/key",
        "https://example.test/apt/",
        "stable",
    );

    fs::write(&input, "interrupted").unwrap();
    assert!(host.execute_operation_as(&first, "tester").is_err());
    assert!(fs::read_to_string(&record)
        .unwrap()
        .contains("\"status\":\"pending_initial\""));
    assert!(!host
        .root
        .join("etc/apt/keyrings/cozydot-vendor-name.gpg")
        .exists());

    fs::write(&input, "binary").unwrap();
    fs::write(
        host._dir
            .path()
            .join("tmp/repository-postcondition-failure"),
        "/etc/apt/keyrings/cozydot-vendor-name.gpg",
    )
    .unwrap();
    assert!(host.execute_operation_as(&first, "tester").is_err());
    assert_eq!(
        fs::read(host.root.join("etc/apt/keyrings/cozydot-vendor-name.gpg")).unwrap(),
        b"normalized-binary"
    );
    assert!(!host
        .root
        .join("etc/apt/sources.list.d/cozydot-vendor-name.list")
        .exists());
    assert!(fs::read_to_string(&record)
        .unwrap()
        .contains("\"status\":\"pending_initial\""));
    fs::remove_file(
        host._dir
            .path()
            .join("tmp/repository-postcondition-failure"),
    )
    .unwrap();
    host.execute_operation_as(&first, "tester").unwrap();
    let old_source = fs::read(
        host.root
            .join("etc/apt/sources.list.d/cozydot-vendor-name.list"),
    )
    .unwrap();
    let update = apt_repository_operation(
        "https://example.test/key-v2",
        "https://example.test/apt-v2/",
        "testing",
    );
    fs::write(&input, "binary2").unwrap();
    fs::write(
        host._dir.path().join("tmp/publication-failure"),
        "source-stage",
    )
    .unwrap();
    assert!(host.execute_operation_as(&update, "tester").is_err());
    assert!(fs::read_to_string(&record)
        .unwrap()
        .contains("\"status\":\"pending_update\""));
    assert_eq!(
        fs::read(host.root.join("etc/apt/keyrings/cozydot-vendor-name.gpg")).unwrap(),
        b"normalized-binary-2"
    );
    assert_eq!(
        fs::read(
            host.root
                .join("etc/apt/sources.list.d/cozydot-vendor-name.list")
        )
        .unwrap(),
        old_source
    );

    fs::remove_file(host._dir.path().join("tmp/publication-failure")).unwrap();
    host.execute_operation_as(&update, "tester").unwrap();
    assert_eq!(
        fs::read(host.root.join("etc/apt/keyrings/cozydot-vendor-name.gpg")).unwrap(),
        b"normalized-binary-2"
    );
    assert!(fs::read_to_string(
        host.root
            .join("etc/apt/sources.list.d/cozydot-vendor-name.list")
    )
    .unwrap()
    .contains("https://example.test/apt-v2/ testing main"));
    assert!(fs::read_to_string(&record)
        .unwrap()
        .contains("\"status\":\"completed\""));
}

#[test]
fn apt_repository_retries_after_source_publication_and_exact_postcondition_failure() {
    let host = Host::new();
    configure_key_fakes(&host);
    fs::write(host._dir.path().join("tmp/key-input"), "binary").unwrap();
    let first = apt_repository_operation(
        "https://example.test/key",
        "https://example.test/apt/",
        "stable",
    );
    host.execute_operation_as(&first, "tester").unwrap();
    let update = apt_repository_operation(
        "https://example.test/key",
        "https://example.test/apt/",
        "testing",
    );
    let record = host
        .home
        .join(".local/state/cozydot/apt-repositories/vendor-name.json");
    let source = host
        .root
        .join("etc/apt/sources.list.d/cozydot-vendor-name.list");

    fs::write(
        host._dir.path().join("tmp/publication-failure"),
        "parent-sync",
    )
    .unwrap();
    assert!(host.execute_operation_as(&update, "tester").is_err());
    assert!(fs::read_to_string(&source)
        .unwrap()
        .contains(" testing main"));
    assert!(fs::read_to_string(&record)
        .unwrap()
        .contains("\"status\":\"pending_update\""));
    fs::remove_file(host._dir.path().join("tmp/publication-failure")).unwrap();
    host.execute_operation_as(&update, "tester").unwrap();

    let next = apt_repository_operation(
        "https://example.test/key",
        "https://example.test/apt/",
        "next",
    );
    fs::write(
        host._dir
            .path()
            .join("tmp/repository-postcondition-failure"),
        "/etc/apt/sources.list.d/cozydot-vendor-name.list",
    )
    .unwrap();
    assert!(host.execute_operation_as(&next, "tester").is_err());
    assert!(fs::read_to_string(&record)
        .unwrap()
        .contains("\"status\":\"pending_update\""));
    fs::remove_file(
        host._dir
            .path()
            .join("tmp/repository-postcondition-failure"),
    )
    .unwrap();
    host.execute_operation_as(&next, "tester").unwrap();
    assert!(fs::read_to_string(&source).unwrap().contains(" next main"));
}

#[test]
fn apt_repository_fails_closed_for_ambiguous_or_malformed_managed_records() {
    for kind in [
        "corrupt",
        "duplicate",
        "nested-duplicate",
        "unknown-record",
        "unknown-declaration",
        "unknown-layout",
        "unsupported-version",
        "noncanonical",
        "symlink",
        "directory",
        "wrong-mode",
        "hardlink",
    ] {
        let host = Host::new();
        configure_key_fakes(&host);
        fs::write(host._dir.path().join("tmp/key-input"), "interrupted").unwrap();
        let first = apt_repository_operation(
            "https://example.test/key",
            "https://example.test/apt/",
            "stable",
        );
        assert!(host.execute_operation_as(&first, "tester").is_err());
        let record = host
            .home
            .join(".local/state/cozydot/apt-repositories/vendor-name.json");
        match kind {
            "corrupt" => fs::write(&record, b"not json").unwrap(),
            "duplicate" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(
                    &record,
                    text.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
                )
                .unwrap();
            }
            "nested-duplicate" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(
                    &record,
                    text.replacen(
                        "\"suite\":\"stable\"",
                        "\"suite\":\"stable\",\"suite\":\"testing\"",
                        1,
                    ),
                )
                .unwrap();
            }
            "unknown-record" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(&record, text.replacen('{', "{\"unknown\":true,", 1)).unwrap();
            }
            "unknown-declaration" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(
                    &record,
                    text.replacen(
                        "\"name\":\"Vendor_Name\"",
                        "\"name\":\"Vendor_Name\",\"unknown\":true",
                        1,
                    ),
                )
                .unwrap();
            }
            "unknown-layout" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(
                    &record,
                    text.replacen(
                        "\"suite\":\"stable\"",
                        "\"suite\":\"stable\",\"unknown\":true",
                        1,
                    ),
                )
                .unwrap();
            }
            "unsupported-version" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(&record, text.replacen("\"version\":1", "\"version\":2", 1)).unwrap();
            }
            "noncanonical" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(
                    &record,
                    text.replacen(
                        "\"architecture\":\"amd64\"",
                        "\"architecture\":\"AMD64\"",
                        1,
                    ),
                )
                .unwrap();
            }
            "symlink" => {
                fs::remove_file(&record).unwrap();
                symlink("missing", &record).unwrap();
            }
            "directory" => {
                fs::remove_file(&record).unwrap();
                fs::create_dir(&record).unwrap();
            }
            "wrong-mode" => {
                fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap()
            }
            "hardlink" => {
                fs::hard_link(&record, record.with_extension("linked")).unwrap();
            }
            _ => unreachable!(),
        }
        fs::write(host.log.as_path(), b"").unwrap();
        fs::write(host._dir.path().join("tmp/key-input"), "binary").unwrap();
        assert!(
            host.execute_operation_as(&first, "tester").is_err(),
            "{kind}"
        );
        assert!(host.log().is_empty(), "{kind}: {}", host.log());
    }

    let host = Host::new();
    configure_key_fakes(&host);
    fs::write(host._dir.path().join("tmp/key-input"), "interrupted").unwrap();
    let first = apt_repository_operation(
        "https://example.test/key",
        "https://example.test/apt/",
        "stable",
    );
    assert!(host.execute_operation_as(&first, "tester").is_err());
    fs::write(host.log.as_path(), b"").unwrap();
    let different = apt_repository_operation(
        "https://example.test/key",
        "https://example.test/other/",
        "stable",
    );
    assert!(host.execute_operation_as(&different, "tester").is_err());
    assert!(host.log().is_empty());
}

#[test]
fn apt_repository_initial_publication_races_preserve_foreign_bytes_and_pending_state() {
    for logical in [
        "/etc/apt/keyrings/cozydot-vendor-name.gpg",
        "/etc/apt/sources.list.d/cozydot-vendor-name.list",
    ] {
        let host = Host::new();
        configure_key_fakes(&host);
        fs::write(host._dir.path().join("tmp/key-input"), "binary").unwrap();
        fs::write(
            host._dir.path().join("tmp/repository-inject-before-link"),
            logical,
        )
        .unwrap();
        let operation = apt_repository_operation(
            "https://example.test/key",
            "https://example.test/apt/",
            "stable",
        );

        assert!(host.execute_operation_as(&operation, "tester").is_err());
        let destination = host.root.join(logical.trim_start_matches('/'));
        assert_eq!(
            fs::read(destination).unwrap(),
            b"foreign-race-bytes",
            "{logical}"
        );
        let record = fs::read_to_string(
            host.home
                .join(".local/state/cozydot/apt-repositories/vendor-name.json"),
        )
        .unwrap();
        assert!(record.contains("\"status\":\"pending_initial\""));
        assert!(!record.contains("\"status\":\"completed\""));
    }
}

#[test]
fn apt_repository_rejects_symlinked_and_writable_state_hierarchy_components() {
    for kind in [
        "root-symlink",
        "cozydot-symlink",
        "repositories-symlink",
        "root-writable",
        "cozydot-writable",
        "cozydot-group-writable",
        "repositories-writable",
    ] {
        let host = Host::new();
        configure_key_fakes(&host);
        fs::write(host._dir.path().join("tmp/key-input"), "binary").unwrap();
        let state_home = host._dir.path().join("managed-state");
        let target = host._dir.path().join("state-target");
        match kind {
            "root-symlink" => {
                fs::create_dir(&target).unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
                symlink(&target, &state_home).unwrap();
            }
            "cozydot-symlink" => {
                fs::create_dir(&state_home).unwrap();
                fs::set_permissions(&state_home, fs::Permissions::from_mode(0o700)).unwrap();
                fs::create_dir(&target).unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
                symlink(&target, state_home.join("cozydot")).unwrap();
            }
            "repositories-symlink" => {
                fs::create_dir_all(state_home.join("cozydot")).unwrap();
                fs::set_permissions(&state_home, fs::Permissions::from_mode(0o700)).unwrap();
                fs::set_permissions(
                    state_home.join("cozydot"),
                    fs::Permissions::from_mode(0o700),
                )
                .unwrap();
                fs::create_dir(&target).unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
                symlink(&target, state_home.join("cozydot/apt-repositories")).unwrap();
            }
            "root-writable"
            | "cozydot-writable"
            | "cozydot-group-writable"
            | "repositories-writable" => {
                fs::create_dir_all(state_home.join("cozydot/apt-repositories")).unwrap();
                for path in [
                    state_home.as_path(),
                    state_home.join("cozydot").as_path(),
                    state_home.join("cozydot/apt-repositories").as_path(),
                ] {
                    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                }
                let unsafe_path = match kind {
                    "root-writable" => state_home.clone(),
                    "cozydot-writable" | "cozydot-group-writable" => state_home.join("cozydot"),
                    _ => state_home.join("cozydot/apt-repositories"),
                };
                let mode = if kind == "cozydot-group-writable" {
                    0o770
                } else {
                    0o777
                };
                fs::set_permissions(unsafe_path, fs::Permissions::from_mode(mode)).unwrap();
            }
            _ => unreachable!(),
        }
        let operation = apt_repository_operation(
            "https://example.test/key",
            "https://example.test/apt/",
            "stable",
        );
        assert!(
            host.execute_operation_as_with_state_home(&operation, &state_home)
                .is_err(),
            "{kind}"
        );
        assert!(host.log().is_empty(), "{kind}: {}", host.log());
    }
}

#[test]
fn apt_repository_state_root_ignores_unrelated_sticky_ancestor_and_creates_mode_0700() {
    const CHILD: &str = "COZYDOT_TEST_PERMISSIVE_STATE_UMASK";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new("sh")
            .arg("-c")
            .arg("umask 000; exec \"$@\"")
            .arg("sh")
            .arg(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(
                "apt_repository_state_root_ignores_unrelated_sticky_ancestor_and_creates_mode_0700",
            )
            .arg("--nocapture")
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let host = Host::new();
    configure_key_fakes(&host);
    fs::write(host._dir.path().join("tmp/key-input"), "binary").unwrap();
    let unrelated = host._dir.path().join("sticky-parent");
    fs::create_dir(&unrelated).unwrap();
    fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o1777)).unwrap();
    let state_home = unrelated.join("selected-state-root");
    let operation = apt_repository_operation(
        "https://example.test/key",
        "https://example.test/apt/",
        "stable",
    );
    host.execute_operation_as_with_state_home(&operation, &state_home)
        .unwrap();
    for path in [
        state_home.clone(),
        state_home.join("cozydot"),
        state_home.join("cozydot/apt-repositories"),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[test]
fn apt_repository_rejects_unsafe_preexisting_lock_entries() {
    for kind in ["symlink", "directory", "wrong-mode", "hardlink"] {
        let host = Host::new();
        configure_key_fakes(&host);
        fs::write(host._dir.path().join("tmp/key-input"), "binary").unwrap();
        let state_home = host._dir.path().join("managed-state");
        let directory = state_home.join("cozydot/apt-repositories");
        fs::create_dir_all(&directory).unwrap();
        for path in [
            state_home.as_path(),
            state_home.join("cozydot").as_path(),
            directory.as_path(),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let lock = directory.join("vendor-name.lock");
        match kind {
            "symlink" => symlink("missing", &lock).unwrap(),
            "directory" => fs::create_dir(&lock).unwrap(),
            "wrong-mode" => {
                fs::write(&lock, b"").unwrap();
                fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
            }
            "hardlink" => {
                fs::write(&lock, b"").unwrap();
                fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
                fs::hard_link(&lock, directory.join("linked.lock")).unwrap();
            }
            _ => unreachable!(),
        }
        let operation = apt_repository_operation(
            "https://example.test/key",
            "https://example.test/apt/",
            "stable",
        );
        assert!(
            host.execute_operation_as_with_state_home(&operation, &state_home)
                .is_err(),
            "{kind}"
        );
        assert!(host.log().is_empty(), "{kind}: {}", host.log());
    }
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
    binary2) printf normalized-binary-2 >"$out" ;;
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
fn binary_fixed_url_verifies_checksum_never_queries_github_and_is_offline_when_complete() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
[ -n "$out" ]; printf '\177ELFpayload' >"$out""#,
    );
    let step = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample", "sample-cli"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&step);
    let artifact = host
        .home
        .join(".local/share/cozydot/binaries/sample.AppImage");
    assert_eq!(fs::read(&artifact).unwrap(), b"\x7fELFpayload");
    assert_eq!(
        fs::metadata(&artifact).unwrap().permissions().mode() & 0o777,
        0o755
    );
    for command in ["sample", "sample-cli"] {
        assert_eq!(
            fs::read_link(host.home.join(".local/bin").join(command)).unwrap(),
            artifact
        );
    }
    let first = host.log();
    assert!(!first.contains("api.github.com"));
    host.fake("curl", "exit 99");
    host.run_ok(&step);
    assert_eq!(host.log(), first);
}

#[test]
fn binary_fixed_url_checksum_failure_and_foreign_destinations_fail_before_mutation() {
    for conflict in ["none", "file", "symlink"] {
        let host = Host::new();
        let link = host.home.join(".local/bin/sample");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        match conflict {
            "file" => fs::write(&link, b"foreign").unwrap(),
            "symlink" => symlink("/foreign", &link).unwrap(),
            _ => {}
        }
        host.fake("curl", r#"printf 'curl\n' >>"$LOG"; out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFpayload' >"$out""#);
        let output = host.run(&binary_step(
            operations::BinaryPackageFormat::AppImage,
            &["sample"],
            fixed_source("https://example.test/sample.AppImage", &"00".repeat(32)),
            operations::BinaryPackageMode::EnsurePresent,
        ));
        assert!(!output.status.success(), "{conflict}");
        assert!(!host
            .home
            .join(".local/share/cozydot/binaries/sample.AppImage")
            .exists());
        if conflict == "none" {
            assert!(host.log().contains("curl"));
        } else {
            assert!(host.log().is_empty());
        }
    }
}

#[test]
fn binary_appimage_missing_link_repairs_offline_and_command_changes_remove_only_owned_stale_links()
{
    let host = Host::new();
    host.fake("curl", r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFpayload' >"$out""#);
    let initial = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample", "sample-old"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&initial);
    fs::remove_file(host.home.join(".local/bin/sample")).unwrap();
    host.fake("curl", "exit 98");
    host.run_ok(&initial);
    assert!(fs::symlink_metadata(host.home.join(".local/bin/sample"))
        .unwrap()
        .file_type()
        .is_symlink());
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFpayload' >"$out""#,
    );

    let changed = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample", "sample-new"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&changed);
    assert!(!host.home.join(".local/bin/sample-old").exists());
    assert!(host.home.join(".local/bin/sample-new").exists());
}

#[test]
fn binary_stale_modified_link_is_preserved_and_prevents_completion() {
    let host = Host::new();
    host.fake("curl", r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFpayload' >"$out""#);
    let initial = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample", "sample-old"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&initial);
    let stale = host.home.join(".local/bin/sample-old");
    fs::remove_file(&stale).unwrap();
    fs::write(&stale, b"foreign").unwrap();
    let changed = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample", "sample-new"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    let output = host.run(&changed);
    assert!(!output.status.success());
    assert_eq!(fs::read(&stale).unwrap(), b"foreign");
    assert!(!host.home.join(".local/bin/sample-new").exists());
}

#[test]
fn binary_github_update_uses_stable_identity_and_checksum_composition() {
    let host = Host::new();
    host.fake("curl", r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf '\177ELFpayload' >"$out"; else printf '{"draft":false,"prerelease":false,"tag_name":"v1","assets":[{"name":"sample-amd64-v1","browser_download_url":"https://example.test/sample.AppImage","digest":"sha256:f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb"}]}'; fi"#);
    let ensure = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        github_source(Some(
            "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb",
        )),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&ensure);
    fs::write(&host.log, b"").unwrap();
    let update = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        github_source(Some(
            "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb",
        )),
        operations::BinaryPackageMode::Update,
    );
    host.run_ok(&update);
    let log = host.log();
    assert_eq!(log.matches("curl ").count(), 1, "{log}");
    assert!(log.contains("api.github.com/repos/owner/repo/releases/latest"));
}

#[test]
fn binary_github_declared_api_mismatch_fails_before_download() {
    let host = Host::new();
    host.fake("curl", r#"printf 'curl %s\n' "$*" >>"$LOG"; printf '{"draft":false,"prerelease":false,"tag_name":"v1","assets":[{"name":"sample-amd64-v1","browser_download_url":"https://example.test/sample.AppImage","digest":"sha256:1111111111111111111111111111111111111111111111111111111111111111"}]}'"#);
    let output = host.run(&binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        github_source(Some(&"22".repeat(32))),
        operations::BinaryPackageMode::EnsurePresent,
    ));
    assert!(!output.status.success());
    assert_eq!(host.log().matches("curl ").count(), 1);
}

#[test]
fn binary_github_api_only_and_declaration_only_checksums_verify_downloads() {
    const PAYLOAD_SHA256: &str = "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb";
    for source in ["api", "declaration"] {
        let host = Host::new();
        let digest = if source == "api" {
            format!(r#","digest":"sha256:{PAYLOAD_SHA256}""#)
        } else {
            String::new()
        };
        host.fake(
            "curl",
            &format!(
                r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf '\177ELFpayload' >"$out"; else printf '{{"draft":false,"prerelease":false,"tag_name":"v1","assets":[{{"name":"sample-amd64-v1","browser_download_url":"https://example.test/sample.AppImage"{digest}}}]}}'; fi"#
            ),
        );
        let declared = (source == "declaration").then_some(PAYLOAD_SHA256);
        host.run_ok(&binary_step(
            operations::BinaryPackageFormat::AppImage,
            &["sample"],
            github_source(declared),
            operations::BinaryPackageMode::EnsurePresent,
        ));
        let record =
            fs::read_to_string(host.home.join(".local/state/cozydot/binaries/sample.json"))
                .unwrap();
        assert!(record.contains(&format!("\"effective_sha256\":\"{PAYLOAD_SHA256}\"")));
    }

    for source in ["api", "declaration"] {
        let host = Host::new();
        let bad_checksum = "00".repeat(32);
        let digest = if source == "api" {
            format!(r#","digest":"sha256:{bad_checksum}""#)
        } else {
            String::new()
        };
        host.fake(
            "curl",
            &format!(
                r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf '\177ELFpayload' >"$out"; else printf '{{"draft":false,"prerelease":false,"tag_name":"v1","assets":[{{"name":"sample-amd64-v1","browser_download_url":"https://example.test/sample.AppImage"{digest}}}]}}'; fi"#
            ),
        );
        let declared = (source == "declaration").then_some(bad_checksum.as_str());
        assert!(!host
            .run(&binary_step(
                operations::BinaryPackageFormat::AppImage,
                &["sample"],
                github_source(declared),
                operations::BinaryPackageMode::EnsurePresent,
            ))
            .status
            .success());
        assert_eq!(host.log().matches("curl ").count(), 2);
        assert!(!host
            .home
            .join(".local/share/cozydot/binaries/sample.AppImage")
            .exists());
    }
}

#[test]
fn binary_deb_strict_metadata_native_and_all_then_offline_ensure() {
    for architecture in ["amd64", "all"] {
        let host = Host::new();
        host.fake("curl", r#"printf 'curl\n' >>"$LOG"; out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf deb >"$out""#);
        host.fake("dpkg-deb", &format!(r#"printf 'dpkg-deb %s\n' "$*" >>"$LOG"; if [ "$1" = --field ]; then printf 'sample\n{architecture}\n'; fi"#));
        host.fake("sudo", r#"printf 'sudo %s\n' "$*" >>"$LOG"; bin=${PATH%%:*}; printf '#!/bin/sh\n' >"$bin/sample"; chmod 0755 "$bin/sample""#);
        let step = binary_step(
            operations::BinaryPackageFormat::Deb,
            &["sample"],
            fixed_source(
                "https://example.test/sample.deb",
                "9cfa1468c93fc18652e34a000f0c6614b0fa18f6f4887477ad9b0d36ca6a7eaa",
            ),
            operations::BinaryPackageMode::EnsurePresent,
        );
        host.run_ok(&step);
        let first = host.log();
        assert!(first.contains("dpkg-deb --info --"));
        assert!(first.contains("dpkg-deb --field --"));
        host.fake("curl", "exit 99");
        host.run_ok(&step);
        assert_eq!(host.log(), first);
    }
}

#[test]
fn binary_deb_rejects_wrong_or_malformed_metadata_before_sudo() {
    for fields in [
        "sample\narm64\n",
        "Bad_Name\namd64\n",
        "sample\namd64\nextra\n",
        "sample amd64\n",
    ] {
        let host = Host::new();
        host.fake("curl", r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf deb >"$out""#);
        host.fake(
            "dpkg-deb",
            &format!(
                r#"if [ "$1" = --field ]; then printf '%b' '{}'; fi"#,
                fields
            ),
        );
        host.logging_fake("sudo");
        let output = host.run(&binary_step(
            operations::BinaryPackageFormat::Deb,
            &["sample"],
            fixed_source(
                "https://example.test/sample.deb",
                "9cfa1468c93fc18652e34a000f0c6614b0fa18f6f4887477ad9b0d36ca6a7eaa",
            ),
            operations::BinaryPackageMode::EnsurePresent,
        ));
        assert!(!output.status.success(), "{fields:?}");
        assert!(!host.log().contains("sudo"), "{}", host.log());
    }
}

#[test]
fn binary_no_state_does_not_adopt_path_command_and_legacy_adapter_uses_binary_display() {
    let host = Host::new();
    host.fake("sample", "exit 0");
    host.fake("curl", r#"printf 'curl\n' >>"$LOG"; out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf deb >"$out""#);
    host.fake(
        "dpkg-deb",
        r#"if [ "$1" = --field ]; then printf 'sample\namd64\n'; fi"#,
    );
    host.logging_fake("sudo");
    let output = host.run(&binary_step(
        operations::BinaryPackageFormat::Deb,
        &["sample"],
        fixed_source(
            "https://example.test/sample.deb",
            "9cfa1468c93fc18652e34a000f0c6614b0fa18f6f4887477ad9b0d36ca6a7eaa",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    ));
    assert!(output.status.success());
    assert!(host.log().contains("curl"));
    assert!(direct_step(
        operations::DirectPackageFormat::Deb,
        &["sample"],
        operations::DirectPackageMode::EnsurePresent
    )
    .display()
    .contains("binary-package"));
}

#[test]
fn binary_unsafe_state_hierarchy_and_records_fail_before_network() {
    let host = Host::new();
    let state = host.home.join(".local/state");
    fs::create_dir_all(state.join("target")).unwrap();
    symlink("target", state.join("cozydot")).unwrap();
    host.logging_fake("curl");
    let output = host.run(&binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "f9eef27e57ba7160224b739c77d4fa1dd7169c5ca8bb7247b899a17cd4370bfb",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    ));
    assert!(!output.status.success());
    assert!(host.log().is_empty());
}

#[test]
fn binary_pending_retry_pins_checksumless_github_bytes() {
    let host = Host::new();
    fs::write(host._dir.path().join("tmp/binary-bytes"), b"deb-one").unwrap();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then cat "$TMPDIR/binary-bytes" >"$out"; else printf '{"draft":false,"prerelease":false,"tag_name":"v1","assets":[{"name":"sample-amd64-v1","browser_download_url":"https://example.test/sample-v1.deb"}]}'; fi"#,
    );
    host.fake(
        "dpkg-deb",
        r#"printf 'dpkg-deb %s\n' "$*" >>"$LOG"; if [ "$1" = --field ]; then printf 'sample\namd64\n'; fi"#,
    );
    host.fake("sudo", r#"printf 'sudo %s\n' "$*" >>"$LOG"; exit 73"#);
    let step = binary_step(
        operations::BinaryPackageFormat::Deb,
        &["sample"],
        github_source(None),
        operations::BinaryPackageMode::EnsurePresent,
    );

    assert!(!host.run(&step).status.success());
    let record = host.home.join(".local/state/cozydot/binaries/sample.json");
    let pending = fs::read_to_string(&record).unwrap();
    assert!(pending.contains("\"status\":\"pending_initial\""));
    assert!(pending.contains("418902f4c16dd75525b5b2b8af23678d8c0a1ae085e05138402702523ad4ba07"));

    fs::write(host.log.as_path(), b"").unwrap();
    fs::write(host._dir.path().join("tmp/binary-bytes"), b"deb-two").unwrap();
    assert!(!host.run(&step).status.success());
    let log = host.log();
    assert_eq!(log.matches("curl ").count(), 1, "{log}");
    assert!(!log.contains("api.github.com"), "{log}");
    assert!(!log.contains("dpkg-deb"), "{log}");
    assert!(!log.contains("sudo"), "{log}");
    assert!(fs::read_to_string(record)
        .unwrap()
        .contains("\"status\":\"pending_initial\""));
}

#[test]
fn binary_completed_deb_reinstalls_the_recorded_github_source_without_resolution() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf deb-one >"$out"; else printf '{"draft":false,"prerelease":false,"tag_name":"v1","assets":[{"name":"sample-amd64-v1","browser_download_url":"https://example.test/sample-v1.deb"}]}'; fi"#,
    );
    host.fake(
        "dpkg-deb",
        r#"printf 'dpkg-deb %s\n' "$*" >>"$LOG"; if [ "$1" = --field ]; then printf 'sample\namd64\n'; fi"#,
    );
    host.fake(
        "sudo",
        r#"printf 'sudo %s\n' "$*" >>"$LOG"; bin=${PATH%%:*}; printf '#!/bin/sh\n' >"$bin/sample"; chmod 0755 "$bin/sample""#,
    );
    let step = binary_step(
        operations::BinaryPackageFormat::Deb,
        &["sample"],
        github_source(None),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&step);
    fs::remove_file(host.bin.join("sample")).unwrap();
    fs::write(host.log.as_path(), b"").unwrap();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"; [[ " $* " != *api.github.com* ]] || exit 91; out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf deb-one >"$out""#,
    );

    host.run_ok(&step);
    let log = host.log();
    assert_eq!(log.matches("curl ").count(), 1, "{log}");
    assert!(!log.contains("api.github.com"), "{log}");
    assert!(log.contains("dpkg-deb --field"), "{log}");
    assert!(log.contains("sudo "), "{log}");
}

#[test]
fn binary_github_changed_identity_updates_appimage_bytes() {
    let host = Host::new();
    fs::write(host._dir.path().join("tmp/release-tag"), b"v1").unwrap();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
tag=$(cat "$TMPDIR/release-tag")
if [ -n "$out" ]; then if [ "$tag" = v1 ]; then printf '\177ELFone' >"$out"; else printf '\177ELFtwo' >"$out"; fi
else printf '{"draft":false,"prerelease":false,"tag_name":"%s","assets":[{"name":"sample-amd64-%s","browser_download_url":"https://example.test/sample-%s.AppImage"}]}' "$tag" "$tag" "$tag"; fi"#,
    );
    let ensure = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        github_source(None),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&ensure);
    let artifact = host
        .home
        .join(".local/share/cozydot/binaries/sample.AppImage");
    assert_eq!(fs::read(&artifact).unwrap(), b"\x7fELFone");

    fs::write(host._dir.path().join("tmp/release-tag"), b"v2").unwrap();
    fs::write(host.log.as_path(), b"").unwrap();
    let update = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        github_source(None),
        operations::BinaryPackageMode::Update,
    );
    host.run_ok(&update);
    assert_eq!(fs::read(&artifact).unwrap(), b"\x7fELFtwo");
    let log = host.log();
    assert_eq!(log.matches("curl ").count(), 2, "{log}");
    assert!(log.contains("sample-v2.AppImage"), "{log}");
    assert!(
        fs::read_to_string(host.home.join(".local/state/cozydot/binaries/sample.json"))
            .unwrap()
            .contains("\"tag\":\"v2\"")
    );
}

#[test]
fn binary_appimage_initial_and_update_races_preserve_foreign_artifacts() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"; out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf foreign-initial >"$XDG_DATA_HOME/cozydot/binaries/sample.AppImage"; printf '\177ELFone' >"$out""#,
    );
    let initial = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        fixed_source(
            "https://example.test/sample-one.AppImage",
            "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    assert!(!host.run(&initial).status.success());
    let artifact = host
        .home
        .join(".local/share/cozydot/binaries/sample.AppImage");
    assert_eq!(fs::read(&artifact).unwrap(), b"foreign-initial");
    assert!(
        fs::read_to_string(host.home.join(".local/state/cozydot/binaries/sample.json"))
            .unwrap()
            .contains("\"status\":\"pending_initial\"")
    );
    fs::write(host.log.as_path(), b"").unwrap();
    host.logging_fake("curl");
    assert!(!host.run(&initial).status.success());
    assert!(host.log().is_empty());
    assert_eq!(fs::read(&artifact).unwrap(), b"foreign-initial");

    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFone' >"$out""#,
    );
    host.run_ok(&initial);
    let artifact = host
        .home
        .join(".local/share/cozydot/binaries/sample.AppImage");
    host.fake(
        "curl",
        r#"printf 'curl %s\n' "$*" >>"$LOG"; out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; rm -f "$XDG_DATA_HOME/cozydot/binaries/sample.AppImage"; printf foreign-update >"$XDG_DATA_HOME/cozydot/binaries/sample.AppImage"; printf '\177ELFtwo' >"$out""#,
    );
    let changed = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        fixed_source(
            "https://example.test/sample-two.AppImage",
            "733b31227555fba7435fae977758297e9787f01201757b7a8b9c4bbbd75635b5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    assert!(!host.run(&changed).status.success());
    assert_eq!(fs::read(&artifact).unwrap(), b"foreign-update");
    assert!(
        fs::read_to_string(host.home.join(".local/state/cozydot/binaries/sample.json"))
            .unwrap()
            .contains("\"status\":\"pending_update\"")
    );
    fs::write(host.log.as_path(), b"").unwrap();
    host.logging_fake("curl");
    assert!(!host.run(&changed).status.success());
    assert!(host.log().is_empty());
    assert_eq!(fs::read(&artifact).unwrap(), b"foreign-update");
}

#[test]
fn binary_new_command_conflict_precedes_download_and_artifact_replacement() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFone' >"$out""#,
    );
    let initial = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        fixed_source(
            "https://example.test/sample-one.AppImage",
            "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&initial);
    let artifact = host
        .home
        .join(".local/share/cozydot/binaries/sample.AppImage");
    fs::write(host.home.join(".local/bin/sample-new"), b"foreign").unwrap();
    fs::write(host.log.as_path(), b"").unwrap();
    host.logging_fake("curl");
    let changed = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample", "sample-new"],
        fixed_source(
            "https://example.test/sample-two.AppImage",
            "733b31227555fba7435fae977758297e9787f01201757b7a8b9c4bbbd75635b5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );

    assert!(!host.run(&changed).status.success());
    assert!(host.log().is_empty());
    assert_eq!(fs::read(&artifact).unwrap(), b"\x7fELFone");
    assert_eq!(
        fs::read(host.home.join(".local/bin/sample-new")).unwrap(),
        b"foreign"
    );
    assert!(
        fs::read_to_string(host.home.join(".local/state/cozydot/binaries/sample.json"))
            .unwrap()
            .contains("\"status\":\"completed\"")
    );
}

#[test]
fn binary_stale_cleanup_retry_accepts_absence_only_from_pending_update() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFone' >"$out""#,
    );
    let initial = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample", "sample-old"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&initial);
    let artifact = host
        .home
        .join(".local/share/cozydot/binaries/sample.AppImage");
    let stale = host.home.join(".local/bin/sample-old");
    fs::remove_file(&stale).unwrap();
    fs::write(host.log.as_path(), b"").unwrap();
    host.fake("curl", "exit 99");
    let changed = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample", "sample-new"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    assert!(!host.run(&changed).status.success());
    assert!(host.log().is_empty());
    assert!(!host.home.join(".local/bin/sample-new").exists());

    symlink(&artifact, &stale).unwrap();
    let record = host.home.join(".local/state/cozydot/binaries/sample.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    let previous = serde_json::json!({
        "declaration": value["declaration"].clone(),
        "resolved": value["resolved"].clone(),
    });
    value["status"] = serde_json::json!("pending_update");
    value["declaration"]["commands"] = serde_json::json!(["sample", "sample-new"]);
    value["previous"] = previous;
    fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::remove_file(&stale).unwrap();
    fs::write(&stale, b"foreign-after-cleanup").unwrap();
    assert!(!host.run(&changed).status.success());
    assert_eq!(fs::read(&stale).unwrap(), b"foreign-after-cleanup");
    assert!(host.log().is_empty());

    fs::remove_file(&stale).unwrap();
    host.run_ok(&changed);
    assert!(!stale.exists());
    assert_eq!(
        fs::read_link(host.home.join(".local/bin/sample-new")).unwrap(),
        artifact
    );
    assert!(fs::read_to_string(record)
        .unwrap()
        .contains("\"status\":\"completed\""));
    assert!(host.log().is_empty());
}

#[test]
fn binary_rejects_unsafe_data_and_command_roots_before_network() {
    for kind in [
        "data-symlink",
        "bin-symlink",
        "data-writable",
        "bin-writable",
    ] {
        let host = Host::new();
        let local = host.home.join(".local");
        let data = local.join("share");
        let bin = local.join("bin");
        fs::create_dir_all(&local).unwrap();
        let target = host._dir.path().join(format!("{kind}-target"));
        match kind {
            "data-symlink" => {
                fs::create_dir(&target).unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
                symlink(&target, &data).unwrap();
            }
            "bin-symlink" => {
                fs::create_dir(&target).unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
                symlink(&target, &bin).unwrap();
            }
            "data-writable" => {
                fs::create_dir(&data).unwrap();
                fs::set_permissions(&data, fs::Permissions::from_mode(0o777)).unwrap();
            }
            "bin-writable" => {
                fs::create_dir(&bin).unwrap();
                fs::set_permissions(&bin, fs::Permissions::from_mode(0o777)).unwrap();
            }
            _ => unreachable!(),
        }
        host.logging_fake("curl");
        let output = host.run(&binary_step(
            operations::BinaryPackageFormat::AppImage,
            &["sample"],
            fixed_source(
                "https://example.test/sample.AppImage",
                "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
            ),
            operations::BinaryPackageMode::EnsurePresent,
        ));
        assert!(!output.status.success(), "{kind}");
        assert!(host.log().is_empty(), "{kind}: {}", host.log());
    }

    let host = Host::new();
    host.logging_fake("curl");
    let operation = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    for (data, bin) in [
        (PathBuf::from("relative-data"), host.home.join("bin-one")),
        (host.home.join("data-two"), PathBuf::from("relative-bin")),
    ] {
        assert!(host
            .execute_operation_with_xdg_roots(operation.operation(), &data, &bin)
            .is_err());
        assert!(host.log().is_empty());
    }
}

#[test]
fn binary_created_roots_are_mode_0700_under_permissive_umask() {
    const CHILD: &str = "COZYDOT_TEST_BINARY_PERMISSIVE_UMASK";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new("sh")
            .arg("-c")
            .arg("umask 000; exec \"$@\"")
            .arg("sh")
            .arg(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("binary_created_roots_are_mode_0700_under_permissive_umask")
            .arg("--nocapture")
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }

    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFone' >"$out""#,
    );
    host.run_ok(&binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    ));
    for path in [
        host.home.join(".local/share"),
        host.home.join(".local/share/cozydot"),
        host.home.join(".local/share/cozydot/binaries"),
        host.home.join(".local/bin"),
        host.home.join(".local/state"),
        host.home.join(".local/state/cozydot"),
        host.home.join(".local/state/cozydot/binaries"),
    ] {
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700,
            "{}",
            path.display()
        );
    }
}

#[test]
fn binary_rejects_hostile_records_and_hardlinked_artifacts_before_network() {
    for kind in [
        "duplicate",
        "unknown",
        "unsupported-version",
        "noncanonical-url",
        "mismatched-checksum",
        "invalid-status",
        "symlink",
        "directory",
        "wrong-mode",
        "hardlink",
    ] {
        let host = Host::new();
        host.fake(
            "curl",
            r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFone' >"$out""#,
        );
        let step = binary_step(
            operations::BinaryPackageFormat::AppImage,
            &["sample"],
            fixed_source(
                "https://example.test/sample.AppImage",
                "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
            ),
            operations::BinaryPackageMode::EnsurePresent,
        );
        host.run_ok(&step);
        let record = host.home.join(".local/state/cozydot/binaries/sample.json");
        match kind {
            "duplicate" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(
                    &record,
                    text.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
                )
                .unwrap();
            }
            "unknown" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(&record, text.replacen('{', "{\"unknown\":true,", 1)).unwrap();
            }
            "unsupported-version" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(&record, text.replacen("\"version\":1", "\"version\":2", 1)).unwrap();
            }
            "noncanonical-url" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(
                    &record,
                    text.replacen("https://example.test", "https://EXAMPLE.test", 1),
                )
                .unwrap();
            }
            "mismatched-checksum" => {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
                value["resolved"]["effective_sha256"] = serde_json::json!("00".repeat(32));
                fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "invalid-status" => {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
                value["status"] = serde_json::json!("pending_update");
                fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "symlink" => {
                fs::remove_file(&record).unwrap();
                symlink("missing", &record).unwrap();
            }
            "directory" => {
                fs::remove_file(&record).unwrap();
                fs::create_dir(&record).unwrap();
            }
            "wrong-mode" => {
                fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
            }
            "hardlink" => {
                fs::hard_link(&record, record.with_extension("linked")).unwrap();
            }
            _ => unreachable!(),
        }
        fs::write(host.log.as_path(), b"").unwrap();
        host.logging_fake("curl");
        assert!(!host.run(&step).status.success(), "{kind}");
        assert!(host.log().is_empty(), "{kind}: {}", host.log());
    }

    let host = Host::new();
    host.fake(
        "curl",
        r#"out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done; printf '\177ELFone' >"$out""#,
    );
    let step = binary_step(
        operations::BinaryPackageFormat::AppImage,
        &["sample"],
        fixed_source(
            "https://example.test/sample.AppImage",
            "d4923526ab32944a1a0ffd7c71764d647911e5701a016abf69c370d1da8b0ff5",
        ),
        operations::BinaryPackageMode::EnsurePresent,
    );
    host.run_ok(&step);
    let artifact = host
        .home
        .join(".local/share/cozydot/binaries/sample.AppImage");
    fs::hard_link(&artifact, artifact.with_extension("linked")).unwrap();
    fs::write(host.log.as_path(), b"").unwrap();
    host.logging_fake("curl");
    assert!(!host.run(&step).status.success());
    assert!(host.log().is_empty());
}

#[test]
fn schema_v1_npm_ensure_uses_selected_fnm_node_and_installs_ordered_missing_subset() {
    let host = Host::new();
    configure_npm_package_fakes(
        &host,
        b"v22.14.0\n",
        br#"{"dependencies":{"present":{"version":"1.0.0"}}}"#,
        br#"{"dependencies":{"present":{"version":"1.0.0"},"missing-one":{"version":"2.0.0"},"@scope/tool":{"version":"3.0.0"}}}"#,
    );
    let step = npm_package_step(
        &["missing-one", "present", "@scope/tool"],
        operations::NpmPackageMode::EnsurePresent,
    );

    host.run_ok(&step);
    host.run_ok(&step);

    let log = host.log();
    assert_eq!(log.matches("fnm <default>").count(), 2, "{log}");
    assert_eq!(
        log.matches(
            "fnm <exec> <--using> <v22.14.0> <--> <npm> <list> <--global> <--depth=0> <--json>"
        )
        .count(),
        3,
        "{log}"
    );
    assert_eq!(
        log.matches("fnm <exec> <--using> <v22.14.0> <--> <npm> <install> <--global> <--> <missing-one> <@scope/tool>")
            .count(),
        1,
        "{log}"
    );
    assert!(!log.contains("ambient-npm"), "{log}");
}

#[test]
fn schema_v1_npm_empty_global_root_installs_then_becomes_query_only() {
    let host = Host::new();
    configure_npm_package_fakes(
        &host,
        b"v22.14.0\n",
        br#"{"name":"lib"}"#,
        br#"{"name":"lib","dependencies":{"tool":{"version":"1.0.0"}}}"#,
    );
    let step = npm_package_step(&["tool"], operations::NpmPackageMode::EnsurePresent);

    host.run_ok(&step);
    host.run_ok(&step);

    let log = host.log();
    assert_eq!(
        log.matches("<list> <--global> <--depth=0> <--json>")
            .count(),
        3,
        "{log}"
    );
    assert_eq!(
        log.matches("<install> <--global> <--> <tool>").count(),
        1,
        "{log}"
    );
    assert!(!log.contains("ambient-npm"), "{log}");
}

#[test]
fn schema_v1_npm_installed_ensure_is_a_single_state_query_noop() {
    let host = Host::new();
    let state = br#"{"dependencies":{"opencode-ai":{"version":"1.0.0"},"@scope/tool":{"version":"2.0.0"}}}"#;
    configure_npm_package_fakes(&host, b"v20.11.1\n", state, state);
    host.run_ok(&npm_package_step(
        &["opencode-ai", "@scope/tool"],
        operations::NpmPackageMode::EnsurePresent,
    ));
    let log = host.log();
    assert_eq!(log.matches("<list>").count(), 1, "{log}");
    assert!(!log.contains("<install>"), "{log}");
    assert!(!log.contains("<update>"), "{log}");
    assert!(!log.contains("ambient-npm"), "{log}");
}

#[test]
fn schema_v1_npm_dependency_error_prevents_ensure_noop_and_mutation() {
    let host = Host::new();
    let state = br#"{"dependencies":{"tool":{"version":"1.0.0","error":{"code":"EFAIL"}}}}"#;
    configure_npm_package_fakes(&host, b"v22.14.0\n", state, state);

    assert!(!host
        .run(&npm_package_step(
            &["tool"],
            operations::NpmPackageMode::EnsurePresent,
        ))
        .status
        .success());
    assert_eq!(
        host.log(),
        concat!(
            "fnm <default>\n",
            "fnm <exec> <--using> <v22.14.0> <--> <npm> <list> <--global> <--depth=0> <--json>\n"
        )
    );
    assert!(!host.log().contains("ambient-npm"), "{}", host.log());
}

#[test]
fn schema_v1_npm_dependency_problem_states_prevent_ensure_noop_and_mutation() {
    for (name, state) in [
        (
            "problems",
            br#"{"dependencies":{"tool":{"version":"1.0.0","problems":["broken"]}}}"#.as_slice(),
        ),
        (
            "invalid",
            br#"{"dependencies":{"tool":{"version":"1.0.0","invalid":true}}}"#.as_slice(),
        ),
        (
            "missing",
            br#"{"dependencies":{"tool":{"version":"1.0.0","missing":true}}}"#.as_slice(),
        ),
    ] {
        let host = Host::new();
        configure_npm_package_fakes(&host, b"v22.14.0\n", state, state);

        assert!(
            !host
                .run(&npm_package_step(
                    &["tool"],
                    operations::NpmPackageMode::EnsurePresent,
                ))
                .status
                .success(),
            "dependency {name} state unexpectedly accepted"
        );
        assert_eq!(
            host.log(),
            concat!(
                "fnm <default>\n",
                "fnm <exec> <--using> <v22.14.0> <--> <npm> <list> <--global> <--depth=0> <--json>\n"
            ),
            "dependency {name} state"
        );
        assert!(!host.log().contains("ambient-npm"), "{}", host.log());
    }
}

#[test]
fn schema_v1_npm_update_installs_existing_and_missing_without_targeting_unrelated_package() {
    let host = Host::new();
    let state =
        br#"{"dependencies":{"unrelated":{"version":"9.0.0"},"tool-two":{"version":"1.0.0"}}}"#;
    let post_state = br#"{"dependencies":{"unrelated":{"version":"9.0.0"},"tool-two":{"version":"2.0.0"},"@scope/tool":{"version":"3.0.0"}}}"#;
    configure_npm_package_fakes(&host, b"v24.1.0\n", state, post_state);
    host.run_ok(&npm_package_step(
        &["@scope/tool", "tool-two"],
        operations::NpmPackageMode::UpdateCurrent,
    ));
    let log = host.log();
    assert!(
        log.contains("fnm <exec> <--using> <v24.1.0> <--> <npm> <install> <--global> <--> <@scope/tool> <tool-two>"),
        "{log}"
    );
    assert!(
        !log.contains("<install> <--global> <--> <unrelated>"),
        "{log}"
    );
    assert!(!log.contains("<update>"), "{log}");
    assert_eq!(
        log.matches("<list> <--global> <--depth=0> <--json>")
            .count(),
        2,
        "{log}"
    );
    assert!(!log.contains("ambient-npm"), "{log}");
}

#[test]
fn schema_v1_npm_rejects_invalid_duplicate_and_injection_inputs_before_execution() {
    for packages in [
        Vec::<String>::new(),
        vec!["tool".into(), "tool".into()],
        vec!["Tool".into()],
        vec!["tool@latest".into()],
        vec!["tool;touch-pwned".into()],
        vec!["@scope/".into()],
        vec!["--force".into()],
    ] {
        assert!(operations::NpmPackageOperation::new(
            packages,
            operations::NpmPackageMode::EnsurePresent
        )
        .is_err());
    }
}

#[test]
fn schema_v1_npm_rejects_absent_or_invalid_default_node() {
    let absent = Host::new();
    assert!(!absent
        .run_with_path(
            &npm_package_step(&["tool"], operations::NpmPackageMode::EnsurePresent,),
            absent.bin.display().to_string(),
        )
        .status
        .success());

    for version in [
        b"none\n".as_slice(),
        b"default\n".as_slice(),
        b"22.1.0\n".as_slice(),
        b"v22.1\n".as_slice(),
        b"v022.1.0\n".as_slice(),
        b"v22.1.0\nv20.0.0\n".as_slice(),
        b"v22.1.0\r\n".as_slice(),
        b"\xff\n".as_slice(),
    ] {
        let host = Host::new();
        configure_npm_package_fakes(
            &host,
            version,
            br#"{"dependencies":{}}"#,
            br#"{"dependencies":{"tool":{"version":"1.0.0"}}}"#,
        );
        assert!(
            !host
                .run(&npm_package_step(
                    &["tool"],
                    operations::NpmPackageMode::EnsurePresent,
                ))
                .status
                .success(),
            "invalid version unexpectedly accepted: {version:?}"
        );
    }
}

#[test]
fn schema_v1_npm_uses_the_accepted_xdg_fnm_path() {
    let host = Host::new();
    let state = br#"{"dependencies":{"tool":{"version":"1.0.0"}}}"#;
    configure_npm_package_fakes(&host, b"v22.1.0\n", state, state);
    host.run_ok(&npm_package_step(
        &["tool"],
        operations::NpmPackageMode::EnsurePresent,
    ));
    assert!(!host.log().contains("ambient-npm"), "{}", host.log());
}

#[test]
fn schema_v1_npm_query_failures_and_bad_json_are_fatal() {
    let cases = [
        br#"not-json"#.as_slice(),
        br#"[]"#.as_slice(),
        br#"{"dependencies":null}"#.as_slice(),
        br#"{"dependencies":[]}"#.as_slice(),
        br#"{"dependencies":{"BAD":{}}}"#.as_slice(),
        br#"{"dependencies":{"tool":{}}}"#.as_slice(),
        br#"{"dependencies":{"tool":null}}"#.as_slice(),
        br#"{"dependencies":{},"problems":["missing: tool"]}"#.as_slice(),
        br#"{"dependencies":{},"error":{"code":"EFAIL"}}"#.as_slice(),
        b"\xff".as_slice(),
    ];
    for state in cases {
        let host = Host::new();
        configure_npm_package_fakes(
            &host,
            b"v22.1.0\n",
            state,
            br#"{"dependencies":{"tool":{"version":"1.0.0"}}}"#,
        );
        assert!(
            !host
                .run(&npm_package_step(
                    &["tool"],
                    operations::NpmPackageMode::EnsurePresent,
                ))
                .status
                .success(),
            "bad npm state unexpectedly accepted: {state:?}"
        );
    }

    let host = Host::new();
    configure_npm_package_fakes(
        &host,
        b"v22.1.0\n",
        br#"{"dependencies":{}}"#,
        br#"{"dependencies":{"tool":{"version":"1.0.0"}}}"#,
    );
    fs::write(host._dir.path().join("tmp/npm-query-failure"), b"1").unwrap();
    assert!(!host
        .run(&npm_package_step(
            &["tool"],
            operations::NpmPackageMode::EnsurePresent,
        ))
        .status
        .success());
}

#[test]
fn schema_v1_npm_propagates_default_mutation_and_postcondition_failures() {
    let default_failure = Host::new();
    default_failure.fake("fnm", "[ \"$1\" != default ] || exit 71");
    assert!(!default_failure
        .run(&npm_package_step(
            &["tool"],
            operations::NpmPackageMode::EnsurePresent,
        ))
        .status
        .success());

    for failure in ["mutation", "postcondition"] {
        let host = Host::new();
        configure_npm_package_fakes(
            &host,
            b"v22.1.0\n",
            br#"{"dependencies":{}}"#,
            if failure == "postcondition" {
                br#"{"dependencies":{}}"#
            } else {
                br#"{"dependencies":{"tool":{"version":"1.0.0"}}}"#
            },
        );
        if failure == "mutation" {
            fs::write(host._dir.path().join("tmp/npm-mutation-failure"), b"1").unwrap();
        }
        assert!(
            !host
                .run(&npm_package_step(
                    &["tool"],
                    operations::NpmPackageMode::EnsurePresent,
                ))
                .status
                .success(),
            "{failure} unexpectedly succeeded"
        );
        assert!(!host.log().contains("ambient-npm"), "{}", host.log());
    }
}

#[test]
fn schema_v1_package_operation_display_forms_include_typed_modes() {
    assert_eq!(
        cargo_package_step(
            &["bat", "ripgrep"],
            operations::CargoPackageMode::UpdateCurrent,
        )
        .display(),
        "workflow cargo-package-set update-current bat ripgrep"
    );
    assert_eq!(
        npm_package_step(&["opencode-ai"], operations::NpmPackageMode::EnsurePresent,).display(),
        "workflow npm-package-set ensure-present opencode-ai"
    );
}

#[test]
fn schema_v1_rust_toolchain_ensure_is_retry_safe_and_update_refreshes_moving_channel() {
    let host = Host::new();
    configure_rust_toolchain_fake(&host);
    let ensure = rust_toolchain_step(operations::ToolMutationMode::EnsurePresent);

    host.run_ok(&ensure);
    host.run_ok(&ensure);
    assert_eq!(
        host.log()
            .matches("rustup <toolchain> <install> <1.90.0-x86_64-unknown-linux-gnu>")
            .count(),
        1,
        "{}",
        host.log()
    );
    assert_eq!(
        host.log()
            .matches("rustup <default> <1.90.0-x86_64-unknown-linux-gnu>")
            .count(),
        1,
        "{}",
        host.log()
    );

    fs::write(host._dir.path().join("tmp/rust-release"), b"1.91.0").unwrap();
    fs::write(host._dir.path().join("tmp/rust-date"), b"2026-02-01").unwrap();
    host.run_ok(&rust_toolchain_step(
        operations::ToolMutationMode::UpdateMoving,
    ));
    assert!(
        host.log()
            .contains("rustup <toolchain> <install> <1.91.0-x86_64-unknown-linux-gnu>"),
        "{}",
        host.log()
    );
}

#[test]
fn schema_v1_node_toolchain_uses_managed_alias_without_shell_evaluation() {
    let host = Host::new();
    configure_node_toolchain_fake(&host);
    let ensure = node_toolchain_step(operations::ToolMutationMode::EnsurePresent);

    host.run_ok(&ensure);
    host.run_ok(&ensure);
    let log = host.log();
    assert_eq!(
        log.matches("fnm <list-remote> <--latest> <--lts>").count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.matches("fnm <install> <v22.14.0> <--progress> <never>")
            .count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.matches("fnm <alias> <v22.14.0> <cozydot-lts>").count(),
        1,
        "{log}"
    );
    assert!(
        log.contains("fnm <exec> <--using> <cozydot-lts> <--> <node> <--version>"),
        "{log}"
    );
    assert!(!log.contains("bash"), "{log}");
}

#[test]
fn schema_v1_node_update_replaces_only_the_moving_selector_alias() {
    let host = Host::new();
    configure_node_toolchain_fake(&host);
    host.run_ok(&node_toolchain_step(
        operations::ToolMutationMode::EnsurePresent,
    ));
    fs::write(host._dir.path().join("tmp/fnm-remote"), b"v24.4.1\n").unwrap();

    host.run_ok(&node_toolchain_step(
        operations::ToolMutationMode::UpdateMoving,
    ));

    let log = host.log();
    assert!(log.contains("fnm <unalias> <cozydot-lts>"), "{log}");
    assert!(log.contains("fnm <alias> <v24.4.1> <cozydot-lts>"), "{log}");
    assert!(log.contains("fnm <default> <v24.4.1>"), "{log}");
}

#[test]
fn schema_v1_uv_python_uses_managed_state_and_is_retry_safe() {
    let host = Host::new();
    configure_python_toolchain_fake(&host);
    let step = python_toolchain_step("3.13");

    host.run_ok(&step);
    host.run_ok(&step);

    let log = host.log();
    assert_eq!(
        log.matches(
            "uv <python> <install> <--no-config> <--managed-python> <--no-progress> <--default> <3.13.7>"
        )
        .count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.matches(
            "uv <python> <find> <--no-project> <--managed-python> <--show-version> <3.13.7>"
        )
        .count(),
        4,
        "{log}"
    );
    assert_eq!(log.matches("uv <python> <list> <3.13>").count(), 1, "{log}");
}

#[test]
fn schema_v1_partial_tool_selectors_pin_across_failed_mutation_retries() {
    let rust = Host::new();
    configure_rust_toolchain_fake(&rust);
    let rust_step = rust_toolchain_selector_step(
        operations::RustToolchainSelector::Version("1.90".into()),
        operations::ToolMutationMode::EnsurePresent,
    );
    fs::write(rust._dir.path().join("tmp/rust-install-failure"), b"1").unwrap();
    assert!(!rust.run(&rust_step).status.success());
    let rust_record = rust.home.join(".local/state/cozydot/tools/rust.json");
    assert!(fs::read_to_string(&rust_record)
        .unwrap()
        .contains("\"status\":\"pending\""));
    fs::remove_file(rust._dir.path().join("tmp/rust-install-failure")).unwrap();
    fs::write(rust._dir.path().join("tmp/rust-release"), b"1.90.1").unwrap();
    fs::write(rust.log.as_path(), b"").unwrap();
    rust.run_ok(&rust_step);
    let log = rust.log();
    assert!(!log.contains("curl"), "{log}");
    assert!(
        log.contains("rustup <toolchain> <install> <1.90.0-x86_64-unknown-linux-gnu>"),
        "{log}"
    );
    assert!(fs::read_to_string(rust_record)
        .unwrap()
        .contains("\"resolved\":\"1.90.0\""));

    let node = Host::new();
    configure_node_toolchain_fake(&node);
    let node_step = node_toolchain_selector_step(
        operations::NodeToolchainSelector::Version("22".into()),
        operations::ToolMutationMode::EnsurePresent,
    );
    fs::write(node._dir.path().join("tmp/fnm-install-failure"), b"1").unwrap();
    assert!(!node.run(&node_step).status.success());
    fs::remove_file(node._dir.path().join("tmp/fnm-install-failure")).unwrap();
    fs::write(node._dir.path().join("tmp/fnm-remote"), b"v22.15.0\n").unwrap();
    fs::write(node.log.as_path(), b"").unwrap();
    node.run_ok(&node_step);
    let log = node.log();
    assert!(!log.contains("list-remote"), "{log}");
    assert!(log.contains("fnm <install> <v22.14.0>"), "{log}");

    let python = Host::new();
    configure_python_toolchain_fake(&python);
    let python_step = python_toolchain_step("3.13");
    fs::write(python._dir.path().join("tmp/python-install-failure"), b"1").unwrap();
    assert!(!python.run(&python_step).status.success());
    fs::remove_file(python._dir.path().join("tmp/python-install-failure")).unwrap();
    fs::write(python._dir.path().join("tmp/python-remote"), b"3.13.8").unwrap();
    fs::write(python.log.as_path(), b"").unwrap();
    python.run_ok(&python_step);
    let log = python.log();
    assert!(!log.contains("<list>"), "{log}");
    assert!(
        log.contains(
            "<install> <--no-config> <--managed-python> <--no-progress> <--default> <3.13.7>"
        ),
        "{log}"
    );
}

#[test]
fn schema_v1_tool_state_is_strict_and_precedes_manager_invocation() {
    for kind in [
        "duplicate",
        "unknown",
        "unsupported",
        "architecture",
        "wrong-mode",
    ] {
        let host = Host::new();
        configure_node_toolchain_fake(&host);
        let step = node_toolchain_step(operations::ToolMutationMode::EnsurePresent);
        host.run_ok(&step);
        let record = host.home.join(".local/state/cozydot/tools/node.json");
        match kind {
            "duplicate" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(
                    &record,
                    text.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
                )
                .unwrap();
            }
            "unknown" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(&record, text.replacen('{', "{\"unknown\":true,", 1)).unwrap();
            }
            "unsupported" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(&record, text.replacen("\"version\":1", "\"version\":2", 1)).unwrap();
            }
            "architecture" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(&record, text.replacen("\"amd64\"", "\"AMD64\"", 1)).unwrap();
            }
            "wrong-mode" => {
                fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
            }
            _ => unreachable!(),
        }
        fs::write(host.log.as_path(), b"").unwrap();
        assert!(!host.run(&step).status.success(), "{kind}");
        assert!(host.log().is_empty(), "{kind}: {}", host.log());
    }
}

#[test]
fn schema_v1_ambient_managers_never_satisfy_or_redirect_managed_operations() {
    let host = Host::new();
    for manager in ["rustup", "fnm", "uv", "cargo", "cargo-binstall"] {
        host.fake(
            manager,
            &format!("printf 'ambient-{manager}\\n' >>\"$LOG\""),
        );
    }
    for step in [
        rust_toolchain_step(operations::ToolMutationMode::EnsurePresent),
        node_toolchain_step(operations::ToolMutationMode::EnsurePresent),
        python_toolchain_step("3.13"),
        cargo_package_step(&["bat"], operations::CargoPackageMode::EnsurePresent),
        npm_package_step(&["tool"], operations::NpmPackageMode::EnsurePresent),
    ] {
        assert!(!host.run(&step).status.success(), "{}", step.display());
    }
    assert!(host.log().is_empty(), "{}", host.log());
}

#[test]
fn schema_v1_cargo_binstall_bootstraps_without_rust_or_cargo_and_is_offline_when_complete() {
    let host = Host::new();
    host.fake(
        "curl",
        r#"{ printf 'curl'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
out=''; while [ "$#" -gt 0 ]; do case "$1" in --output) out=$2; shift 2 ;; *) shift ;; esac; done
if [ -n "$out" ]; then printf archive >"$out"; else printf '{"draft":false,"prerelease":false,"tag_name":"v1.21.0","assets":[{"name":"cargo-binstall-x86_64-unknown-linux-musl.tgz","browser_download_url":"https://github.com/cargo-bins/cargo-binstall/releases/download/v1.21.0/cargo-binstall-x86_64-unknown-linux-musl.tgz","digest":"sha256:0eb3e36bfb24dcd9bb1d1bece1531216b59539a8fde17ee80224af0653c92aa3"}]}'; fi"#,
    );
    host.fake(
        "tar",
        r#"{ printf 'tar'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
for argument in "$@"; do [ "$argument" != --list ] || { printf 'LICENSE\ncargo-binstall\n'; exit; }; done
while [ "$#" -gt 0 ]; do
  if [ "$1" = --directory ]; then
    cat >"$2/cargo-binstall" <<'BINSTALL'
#!/bin/sh
[ "$1" = -V ] || exit 71
printf '1.21.0\n'
BINSTALL
    chmod 0755 "$2/cargo-binstall"
    exit
  fi
  shift
done
exit 72"#,
    );
    host.fake("cargo", "printf 'ambient-cargo\\n' >>\"$LOG\"; exit 90");
    host.fake("rustup", "printf 'ambient-rustup\\n' >>\"$LOG\"; exit 90");
    let step = cargo_binstall_bootstrap_step(Architecture::Amd64);

    host.run_ok(&step);
    let first = host.log();
    assert_eq!(first.matches("curl <").count(), 2, "{first}");
    assert_eq!(first.matches("tar <").count(), 2, "{first}");
    assert!(!first.contains("ambient-"), "{first}");
    assert_eq!(step.display(), "workflow cargo-binstall-bootstrap amd64");
    host.fake("curl", "exit 99");
    host.fake("tar", "exit 99");
    host.run_ok(&step);
    assert_eq!(host.log(), first);
}

#[test]
fn schema_v1_cargo_binstall_rejects_foreign_destination_before_resolution() {
    let host = Host::new();
    let destination = host.home.join(".cargo/bin/cargo-binstall");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"foreign").unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o755)).unwrap();
    host.logging_fake("curl");

    assert!(!host
        .run(&cargo_binstall_bootstrap_step(Architecture::Amd64))
        .status
        .success());
    assert!(host.log().is_empty());
    assert_eq!(fs::read(destination).unwrap(), b"foreign");
}

#[test]
fn schema_v1_tool_operation_display_forms_are_typed() {
    assert_eq!(
        rust_toolchain_step(operations::ToolMutationMode::UpdateMoving).display(),
        "workflow rust-toolchain update-moving stable x86_64-unknown-linux-gnu"
    );
    assert_eq!(
        node_toolchain_step(operations::ToolMutationMode::EnsurePresent).display(),
        "workflow node-toolchain ensure-present lts amd64"
    );
    assert_eq!(
        python_toolchain_step("3.13").display(),
        "workflow python-toolchain 3.13 amd64"
    );
}

#[test]
fn schema_v1_manager_bootstraps_publish_fixed_executables_once() {
    let host = Host::new();
    for manager in ["rustup", "fnm", "uv"] {
        host.fake(
            manager,
            &format!("printf 'ambient-{manager}\\n' >>\"$LOG\""),
        );
    }
    host.fake(
        "curl",
        r#"{ printf 'curl'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
out=''
while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then out=$2; shift 2; else shift; fi; done
[ -n "$out" ] || exit 40
case "$out" in
  *rustup*) cat >"$out" <<'INSTALL'
#!/bin/sh
mkdir -p "$HOME/.cargo/bin"
printf '#!/bin/sh\n' >"$HOME/.cargo/bin/rustup"
chmod +x "$HOME/.cargo/bin/rustup"
INSTALL
    ;;
  *fnm-install*) cat >"$out" <<'INSTALL'
#!/bin/sh
mkdir -p "$XDG_DATA_HOME/fnm"
printf '#!/bin/sh\n' >"$XDG_DATA_HOME/fnm/fnm"
chmod +x "$XDG_DATA_HOME/fnm/fnm"
INSTALL
    ;;
  *uv-install*) cat >"$out" <<'INSTALL'
#!/bin/sh
printf '#!/bin/sh\n' >"$UV_UNMANAGED_INSTALL/uv"
chmod +x "$UV_UNMANAGED_INSTALL/uv"
INSTALL
    ;;
  *) exit 41 ;;
esac"#,
    );
    let steps = [
        bootstrap_step(operations::Operation::RustupBootstrap),
        bootstrap_step(operations::Operation::FnmBootstrap),
        bootstrap_step(operations::Operation::UvBootstrap),
    ];

    for step in &steps {
        host.run_ok(step);
        host.run_ok(step);
    }

    assert!(host.home.join(".cargo/bin/rustup").is_file());
    assert!(host.home.join(".local/share/fnm/fnm").is_file());
    assert!(host.home.join(".local/bin/uv").is_file());
    let log = host.log();
    assert_eq!(log.matches("curl <").count(), 3, "{log}");
    assert!(!log.contains("ambient-"), "{log}");
    assert_eq!(steps[0].display(), "workflow rustup-bootstrap");
    assert_eq!(steps[1].display(), "workflow fnm-bootstrap");
    assert_eq!(steps[2].display(), "workflow uv-bootstrap");
}

#[test]
fn schema_v1_nerd_fonts_publish_user_local_files_and_verify_fontconfig_state() {
    let host = Host::new();
    host.fake(
        "fc-list",
        r#"{ printf 'fc-list'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ -f "$TMPDIR/font-cached" ]; then printf 'GeistMono Nerd Font,GeistMono Nerd Font Mono\n'; fi"#,
    );
    host.fake(
        "curl",
        r#"{ printf 'curl'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
while [ "$#" -gt 0 ]; do if [ "$1" = --output ]; then : >"$2"; exit; fi; shift; done
exit 40"#,
    );
    host.fake(
        "tar",
        r#"{ printf 'tar'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
for argument in "$@"; do [ "$argument" != --list ] || { printf 'GeistMonoNerdFont-Regular.ttf\n'; exit; }; done
while [ "$#" -gt 0 ]; do
  if [ "$1" = --directory ]; then mkdir -p "$2"; printf 'font' >"$2/GeistMonoNerdFont-Regular.ttf"; exit; fi
  shift
done
exit 41"#,
    );
    host.fake(
        "fc-cache",
        "{ printf 'fc-cache'; printf ' <%s>' \"$@\"; printf '\n'; } >>\"$LOG\"; touch \"$TMPDIR/font-cached\"",
    );
    let step = nerd_fonts_step(&["GeistMono"]);

    host.run_ok(&step);
    host.run_ok(&step);

    let installed = host
        .home
        .join(".local/share/fonts/cozydot/GeistMono/GeistMonoNerdFont-Regular.ttf");
    assert_eq!(fs::read(installed).unwrap(), b"font");
    let log = host.log();
    assert_eq!(log.matches("fc-cache <--force>").count(), 1, "{log}");
    assert_eq!(log.matches("curl <").count(), 1, "{log}");
    assert!(
        log.contains(
            "https://github.com/ryanoasis/nerd-fonts/releases/latest/download/GeistMono.tar.xz"
        ),
        "{log}"
    );
}

#[test]
fn schema_v1_dotfiles_back_up_conflicts_before_stow_and_are_retry_safe() {
    let host = Host::new();
    let root = host._dir.path().join("dotfiles");
    fs::create_dir_all(root.join("bash")).unwrap();
    fs::write(root.join("bash/.bashrc"), b"managed\n").unwrap();
    fs::write(host.home.join(".bashrc"), b"user-owned\n").unwrap();
    host.fake(
        "stow",
        r#"{ printf 'stow'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
root=''; target=''; package=''
while [ "$#" -gt 0 ]; do
  case "$1" in --dir) root=$2; shift 2 ;; --target) target=$2; shift 2 ;; --stow|--) shift ;; *) package=$1; shift ;; esac
done
[ -L "$target/.bashrc" ] || ln -s "$root/$package/.bashrc" "$target/.bashrc""#,
    );
    let step = dotfiles_step(&root, &["bash"]);

    host.run_ok(&step);
    host.run_ok(&step);

    assert_eq!(
        fs::canonicalize(host.home.join(".bashrc")).unwrap(),
        fs::canonicalize(root.join("bash/.bashrc")).unwrap()
    );
    let backups = host.home.join(".local/state/cozydot/dotfile-backups");
    let runs = fs::read_dir(backups)
        .unwrap()
        .collect::<std::io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        fs::read(runs[0].path().join("bash/.bashrc")).unwrap(),
        b"user-owned\n"
    );
    let log = host.log();
    assert_eq!(log.matches("stow <").count(), 2, "{log}");
    assert!(log.contains("<--stow> <--> <bash>"), "{log}");
}

#[test]
fn schema_v1_local_state_operation_display_forms_are_typed() {
    assert_eq!(
        nerd_fonts_step(&["GeistMono", "JetBrainsMono"]).display(),
        "workflow nerd-fonts GeistMono JetBrainsMono"
    );
    assert_eq!(
        dotfiles_step(Path::new("/dotfiles"), &["bash", "starship"]).display(),
        "workflow dotfiles-backup-stow bash starship"
    );
}

#[test]
fn schema_v1_desktop_settings_use_target_schemas_and_verify_exact_state() {
    let host = Host::new();
    host.fake(
        "gsettings",
        r#"{ printf 'gsettings'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
state="$TMPDIR/gsettings-${2//./_}-$3"
if [ "$1" = get ]; then if [ -f "$state" ]; then cat "$state"; else printf "'initial'\n"; fi; exit; fi
if [ "$1" = set ]; then printf '%s\n' "$4" >"$state"; exit; fi
exit 40"#,
    );
    host.fake("wezterm", "exit 0");
    let steps = [
        desktop_setting_step(
            operations::DesktopEnvironment::Gnome,
            operations::DesktopSetting::Theme(operations::DesktopTheme::Dark),
        ),
        desktop_setting_step(
            operations::DesktopEnvironment::Cinnamon,
            operations::DesktopSetting::Terminal("wezterm".into()),
        ),
        desktop_setting_step(
            operations::DesktopEnvironment::Gnome,
            operations::DesktopSetting::IdleTimeoutSeconds(900),
        ),
        desktop_setting_step(
            operations::DesktopEnvironment::Cinnamon,
            operations::DesktopSetting::IdleDim(false),
        ),
    ];
    for step in &steps {
        host.run_ok(step);
        host.run_ok(step);
    }

    let log = host.log();
    assert_eq!(log.matches("gsettings <set>").count(), 5, "{log}");
    assert!(
        log.contains("<org.gnome.desktop.interface> <color-scheme> <'prefer-dark'>"),
        "{log}"
    );
    assert!(
        log.contains("<org.cinnamon.desktop.default-applications.terminal> <exec> <'wezterm'>"),
        "{log}"
    );
    assert!(
        log.contains("<org.gnome.desktop.session> <idle-delay> <uint32 900>"),
        "{log}"
    );
    assert!(
        log.contains("<org.cinnamon.settings-daemon.plugins.power> <idle-dim> <false>"),
        "{log}"
    );
}

#[test]
fn schema_v1_gnome_extensions_install_enable_and_requery_without_shells() {
    let host = Host::new();
    host.fake(
        "gnome-extensions",
        r#"{ printf 'gnome-extensions'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
case "$1" in
  list) if [ "${2:-}" = --enabled ]; then [ ! -f "$TMPDIR/gnome-enabled" ] || cat "$TMPDIR/gnome-enabled"; else [ ! -f "$TMPDIR/gnome-installed" ] || cat "$TMPDIR/gnome-installed"; fi ;;
  install) printf 'blur-my-shell@aunetx\n' >"$TMPDIR/gnome-installed" ;;
  enable) printf '%s\n' "$2" >"$TMPDIR/gnome-enabled" ;;
  *) exit 40 ;;
esac"#,
    );
    host.fake("gnome-shell", "printf 'GNOME Shell 48.4\n'");
    host.fake(
        "curl",
        r#"{ printf 'curl'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ "$1" = -fsSL ]; then printf '{"shell_version_map":{"48":{"version":13}}}\n'; exit; fi
while [ "$#" -gt 0 ]; do if [ "$1" = -o ]; then : >"$2"; exit; fi; shift; done
exit 41"#,
    );
    let step = gnome_extensions_step(&["blur-my-shell@aunetx"]);

    host.run_ok(&step);
    host.run_ok(&step);

    let log = host.log();
    assert_eq!(
        log.matches("gnome-extensions <install> <--force>").count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.matches("gnome-extensions <enable> <blur-my-shell@aunetx>")
            .count(),
        1,
        "{log}"
    );
    assert!(
        log.contains("blur-my-shellaunetx.v13.shell-extension.zip"),
        "{log}"
    );
    assert!(!log.contains("bash"), "{log}");
}

#[test]
fn schema_v1_gnome_dconf_layouts_write_once_and_verify_every_apply() {
    for step in [gnome_dock_step(), gnome_rounded_corners_step()] {
        let host = Host::new();
        host.fake(
            "dconf",
            r#"{ printf 'dconf'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
name=$(printf '%s' "$2" | tr -c 'A-Za-z0-9' '_')
state="$TMPDIR/dconf-$name"
if [ "$1" = read ]; then [ ! -f "$state" ] || cat "$state"; exit; fi
if [ "$1" = write ]; then printf '%s\n' "$3" >"$state"; exit; fi
exit 40"#,
        );

        host.run_ok(&step);
        host.run_ok(&step);

        let log = host.log();
        let expected_writes = if step.display().contains("rounded") {
            1
        } else {
            9
        };
        assert_eq!(
            log.matches("dconf <write>").count(),
            expected_writes,
            "{log}"
        );
        assert_eq!(
            log.matches("dconf <read>").count(),
            expected_writes * 4,
            "{log}"
        );
    }
}

#[test]
fn schema_v1_desktop_operation_display_forms_are_typed() {
    assert_eq!(
        desktop_setting_step(
            operations::DesktopEnvironment::Gnome,
            operations::DesktopSetting::IdleTimeoutSeconds(900)
        )
        .display(),
        "workflow desktop-setting gnome idle-timeout-seconds 900"
    );
    assert_eq!(
        gnome_extensions_step(&["blur-my-shell@aunetx"]).display(),
        "workflow gnome-extensions blur-my-shell@aunetx"
    );
    assert_eq!(gnome_dock_step().display(), "workflow gnome-dock");
    assert_eq!(
        gnome_rounded_corners_step().display(),
        "workflow gnome-rounded-corners"
    );
}

#[test]
fn schema_v1_ensure_admin_adds_effective_user_once_and_verifies_membership() {
    let host = Host::new();
    let uid = rustix::process::geteuid().as_raw();
    host.fake(
        "getent",
        &format!(
            r#"{{ printf 'getent'; printf ' <%s>' "$@"; printf '\n'; }} >>"$LOG"
if [ "$1" = passwd ] && [ "$2" = {uid} ]; then printf 'tester:x:{uid}:1000:Tester:/home/tester:/bin/bash\n'; exit; fi
if [ "$1" = group ] && [ "$2" = sudo ]; then printf 'sudo:x:27:\n'; exit; fi
exit 2"#
        ),
    );
    host.fake(
        "id",
        r#"{ printf 'id'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
if [ -f "$TMPDIR/admin-added" ]; then printf '1000 27\n'; else printf '1000\n'; fi"#,
    );
    host.fake(
        "sudo",
        r#"{ printf 'sudo'; printf ' <%s>' "$@"; printf '\n'; } >>"$LOG"
[ "$1" = usermod ] && [ "$2" = -aG ] && [ "$3" = sudo ] && [ "$4" = -- ] && [ "$5" = tester ] || exit 40
touch "$TMPDIR/admin-added""#,
    );
    let step = ensure_admin_step();

    host.run_ok(&step);
    host.run_ok(&step);

    let log = host.log();
    assert_eq!(
        log.matches("sudo <usermod> <-aG> <sudo> <--> <tester>")
            .count(),
        1,
        "{log}"
    );
    assert_eq!(step.display(), "workflow ensure-admin");
}

#[test]
fn schema_v1_unattended_upgrades_converge_enabled_and_disabled_state() {
    let host = Host::new();
    configure_system_state_fakes(&host);
    let enabled = unattended_upgrades_step(true);
    let disabled = unattended_upgrades_step(false);

    host.run_ok(&enabled);
    host.run_ok(&enabled);
    assert!(host
        ._dir
        .path()
        .join("tmp/package-unattended-upgrades")
        .is_file());
    assert_eq!(
        fs::read(host.root.join("etc/apt/apt.conf.d/20auto-upgrades")).unwrap(),
        b"APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n"
    );

    host.run_ok(&disabled);
    host.run_ok(&disabled);
    assert!(!host
        ._dir
        .path()
        .join("tmp/package-unattended-upgrades")
        .exists());
    assert_eq!(
        fs::read(host.root.join("etc/apt/apt.conf.d/20auto-upgrades")).unwrap(),
        b"APT::Periodic::Update-Package-Lists \"0\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n"
    );
    let log = host.log();
    assert_eq!(log.matches("apt-get <install>").count(), 1, "{log}");
    assert_eq!(log.matches("apt-get <purge>").count(), 1, "{log}");
    assert_eq!(
        log.matches("systemctl <enable> <--now> <unattended-upgrades.service>")
            .count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.matches("systemctl <disable> <--now> <unattended-upgrades.service>")
            .count(),
        1,
        "{log}"
    );
}

#[test]
fn schema_v1_ubuntu_snap_disable_and_reenable_converge_owned_state() {
    let host = Host::new();
    configure_system_state_fakes(&host);
    let tmp = host._dir.path().join("tmp");
    fs::write(tmp.join("package-snapd"), b"").unwrap();
    fs::write(tmp.join("snap-firefox"), b"").unwrap();
    fs::write(tmp.join("systemd-snapd_socket-enabled"), b"").unwrap();
    fs::write(tmp.join("systemd-snapd_socket-active"), b"").unwrap();
    for directory in ["snap", "var/snap", "var/lib/snapd"] {
        fs::create_dir_all(host.root.join(directory)).unwrap();
    }
    fs::create_dir_all(host.home.join("snap")).unwrap();
    let disabled = ubuntu_snap_step(false);

    host.run_ok(&disabled);
    host.run_ok(&disabled);
    assert!(!tmp.join("package-snapd").exists());
    assert!(!tmp.join("snap-firefox").exists());
    assert_eq!(
        fs::read(host.root.join("etc/apt/preferences.d/cozydot-no-snap.pref")).unwrap(),
        b"Package: snapd\nPin: release a=*\nPin-Priority: -10\n"
    );
    for directory in ["snap", "var/snap", "var/lib/snapd"] {
        assert!(!host.root.join(directory).exists());
    }
    assert!(!host.home.join("snap").exists());

    host.run_ok(&ubuntu_snap_step(true));
    assert!(tmp.join("package-snapd").is_file());
    assert!(!host
        .root
        .join("etc/apt/preferences.d/cozydot-no-snap.pref")
        .exists());
    let log = host.log();
    assert_eq!(
        log.matches("snap <remove> <--purge> <firefox>").count(),
        1,
        "{log}"
    );
    assert_eq!(
        log.matches("systemctl <enable> <--now> <snapd.socket>")
            .count(),
        1,
        "{log}"
    );
}
