mod binary;

pub(crate) mod privileged_file {
    use super::{Host, TempPath};
    use anyhow::{bail, Context, Result};
    use std::{ffi::OsStr, fs, io::Write, path::Path};

    pub(crate) fn publish_bytes(host: &Host<'_>, destination: &Path, contents: &[u8], operation: &str) -> Result<()> {
        publish_bytes_with_mode(host, destination, contents, operation, "0644")
    }

    pub(crate) fn publish_bytes_with_mode(
        host: &Host<'_>,
        destination: &Path,
        contents: &[u8],
        operation: &str,
        mode: &str,
    ) -> Result<()> {
        publish_bytes_with_mode_and_policy(host, destination, contents, operation, mode, false)
    }

    pub(super) fn publish_bytes_with_policy(
        host: &Host<'_>,
        destination: &Path,
        contents: &[u8],
        operation: &str,
        no_replace: bool,
    ) -> Result<()> {
        publish_bytes_with_mode_and_policy(host, destination, contents, operation, "0644", no_replace)
    }

    fn publish_bytes_with_mode_and_policy(
        host: &Host<'_>,
        destination: &Path,
        contents: &[u8],
        operation: &str,
        mode: &str,
        no_replace: bool,
    ) -> Result<()> {
        if !matches!(mode, "0600" | "0644") {
            bail!("unsupported privileged publication mode");
        }
        let local = TempPath::new(host, "privileged-publication")?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(local.path())
            .context("open local publication staging file")?;
        file.write_all(contents)
            .context("write local publication staging file")?;
        file.sync_all().context("sync local publication staging file")?;
        drop(file);
        let parent = destination.parent().context("publication destination has no parent")?;
        let file_name = destination
            .file_name()
            .context("publication destination has no filename")?
            .to_string_lossy();
        let nonce = local
            .path()
            .file_name()
            .context("publication staging file has no filename")?
            .to_string_lossy();
        let staged = parent.join(format!(".{file_name}.{nonce}.tmp"));
        let parent_arg = parent.as_os_str();
        let local_arg = local.path().as_os_str();
        let staged_arg = staged.as_os_str();
        let destination_arg = destination.as_os_str();
        host.require(
            operation,
            "sudo",
            [
                OsStr::new("install"),
                OsStr::new("-d"),
                OsStr::new("-o"),
                OsStr::new("root"),
                OsStr::new("-g"),
                OsStr::new("root"),
                OsStr::new("-m"),
                OsStr::new("0755"),
                OsStr::new("--"),
                parent_arg,
            ],
        )?;
        let result = (|| {
            host.require(
                operation,
                "sudo",
                [
                    OsStr::new("install"),
                    OsStr::new("-o"),
                    OsStr::new("root"),
                    OsStr::new("-g"),
                    OsStr::new("root"),
                    OsStr::new("-m"),
                    OsStr::new(mode),
                    OsStr::new("--"),
                    local_arg,
                    staged_arg,
                ],
            )?;
            host.require(operation, "sudo", [OsStr::new("sync"), OsStr::new("--"), staged_arg])?;
            if no_replace {
                // `link(2)` is an atomic no-replace publication here: both names are in the
                // destination directory, and an existing destination makes `ln` fail rather
                // than report a skipped move as success. The staging name is removed only
                // after the destination link exists.
                host.require(
                    operation,
                    "sudo",
                    [OsStr::new("ln"), OsStr::new("--"), staged_arg, destination_arg],
                )?;
                host.require(
                    operation,
                    "sudo",
                    [OsStr::new("rm"), OsStr::new("-f"), OsStr::new("--"), staged_arg],
                )?;
            } else {
                host.require(
                    operation,
                    "sudo",
                    [OsStr::new("test"), OsStr::new("!"), OsStr::new("-d"), destination_arg],
                )?;
                host.require(
                    operation,
                    "sudo",
                    [
                        OsStr::new("mv"),
                        OsStr::new("-fT"),
                        OsStr::new("--"),
                        staged_arg,
                        destination_arg,
                    ],
                )?;
            }
            sync_parent(host, destination, operation)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = host.run(
                "sudo",
                [OsStr::new("rm"), OsStr::new("-f"), OsStr::new("--"), staged_arg],
            );
        }
        result
    }

    pub(crate) fn sync_parent(host: &Host<'_>, destination: &Path, operation: &str) -> Result<()> {
        let parent = destination.parent().context("publication destination has no parent")?;
        host.require(
            operation,
            "sudo",
            [OsStr::new("sync"), OsStr::new("--"), parent.as_os_str()],
        )?;
        Ok(())
    }
}

mod repository;
mod system;
mod tools;

pub use apt::AptUpgradePolicy;
pub use binary::cargo_binstall::CargoBinstallBootstrapOperation;
pub use binary::{
    BinaryPackageFormat, BinaryPackageMode, BinaryPackageOperation, BinaryPackageSelector, BinarySha256,
    BinarySourceOperation, GithubRepository,
};
pub use packages::cargo::{CargoPackageMode, CargoPackageOperation};
pub use packages::dotfiles::DotfilesOperation;
pub use packages::fonts::{NerdFontsMode, NerdFontsOperation};
pub use packages::npm::{NpmPackageMode, NpmPackageOperation};
pub use repository::managed_apt::ManagedAptSourcesOperation;
pub use repository::{AptRepositoryOperation, AptRepositoryPath, AptRepositorySourceLayout, AptRepositoryToken};
pub use system::{
    DesktopEnvironment, DesktopSetting, DesktopSettingOperation, DesktopTheme, GnomeDockOperation,
    GnomeExtensionsOperation, GnomeRoundedCornersOperation,
};
pub use system::{DockerLocalLogOperation, VsCodeExtensionOperation};
pub use system::{EnsureAdminOperation, UbuntuSnapOperation, UnattendedUpgradesOperation};
pub use tools::{
    GoToolchainOperation, GoToolchainSelector, NodeToolchainOperation, NodeToolchainSelector, PythonToolchainOperation,
    RustToolchainOperation, RustToolchainSelector, ToolMutationMode,
};

use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::Duration,
};

const COZYDOT_RUNTIME_DIRECTORY: &str = "/run/cozydot";
const DOCKER_LOCK: &str = "/run/cozydot/docker-daemon.lock";
const EXECUTABLE_FILE_BUSY: i32 = 26;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    AptBootstrapPackages { packages: Vec<String> },
    AptMetadataRefresh,
    AptRepository(AptRepositoryOperation),
    ManagedAptSources(ManagedAptSourcesOperation),
    AptPackages { packages: Vec<String> },
    AptPurge { packages: Vec<String> },
    AptUpgrade { policy: AptUpgradePolicy },
    DockerGroup,
    DockerLocalLog(DockerLocalLogOperation),
    DesktopSetting(DesktopSettingOperation),
    BinaryPackage(BinaryPackageOperation),
    Dotfiles(DotfilesOperation),
    FlatpakEnsureFlathub,
    FlatpakEnsureApps { refs: Vec<String> },
    FlatpakUpdateApps { refs: Vec<String> },
    FnmBootstrap,
    EnsureAdmin(EnsureAdminOperation),
    GnomeExtensions(GnomeExtensionsOperation),
    GnomeDock(GnomeDockOperation),
    GnomeRoundedCorners(GnomeRoundedCornersOperation),
    GoToolchain(GoToolchainOperation),
    NerdFonts(NerdFontsOperation),
    RustupBootstrap,
    CargoBinstallBootstrap(CargoBinstallBootstrapOperation),
    RustToolchain(RustToolchainOperation),
    CargoPackageSet(CargoPackageOperation),
    NodeToolchain(NodeToolchainOperation),
    NpmPackageSet(NpmPackageOperation),
    UbuntuSnap(UbuntuSnapOperation),
    UnattendedUpgrades(UnattendedUpgradesOperation),
    UvBootstrap,
    PythonToolchain(PythonToolchainOperation),
    VirtualBoxGroup,
    VsCodeExtensionSet(VsCodeExtensionOperation),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationOutcome {
    Completed,
    LoginRequired,
}

impl Operation {
    pub fn display_args(&self) -> Vec<String> {
        match self {
            Self::AptBootstrapPackages { packages } => std::iter::once("apt-bootstrap-packages".into())
                .chain(packages.clone())
                .collect(),
            Self::AptMetadataRefresh => vec!["apt-metadata-refresh".into()],
            Self::AptRepository(operation) => operation.display_args(),
            Self::ManagedAptSources(operation) => operation.display_args(),
            Self::AptPackages { packages } => std::iter::once("apt-packages".into()).chain(packages.clone()).collect(),
            Self::AptPurge { packages } => std::iter::once("apt-purge".into()).chain(packages.clone()).collect(),
            Self::AptUpgrade { policy } => vec![
                "apt-upgrade".into(),
                match policy {
                    AptUpgradePolicy::Standard => "standard",
                    AptUpgradePolicy::Full => "full",
                }
                .into(),
            ],
            Self::DockerGroup => vec!["docker-group".into()],
            Self::DockerLocalLog(operation) => operation.display_args(),
            Self::DesktopSetting(operation) => operation.display_args(),
            Self::BinaryPackage(package) => package.display_args(),
            Self::Dotfiles(operation) => operation.display_args(),
            Self::FlatpakEnsureFlathub => vec!["flatpak-ensure-flathub".into()],
            Self::FlatpakEnsureApps { refs } => std::iter::once("flatpak-ensure-apps".into())
                .chain(refs.clone())
                .collect(),
            Self::FlatpakUpdateApps { refs } => std::iter::once("flatpak-update-apps".into())
                .chain(refs.clone())
                .collect(),
            Self::FnmBootstrap => vec!["fnm-bootstrap".into()],
            Self::EnsureAdmin(operation) => operation.display_args(),
            Self::GnomeExtensions(operation) => operation.display_args(),
            Self::GnomeDock(operation) => operation.display_args(),
            Self::GnomeRoundedCorners(operation) => operation.display_args(),
            Self::GoToolchain(operation) => operation.display_args(),
            Self::NerdFonts(operation) => operation.display_args(),
            Self::RustupBootstrap => vec!["rustup-bootstrap".into()],
            Self::CargoBinstallBootstrap(operation) => operation.display_args(),
            Self::RustToolchain(operation) => operation.display_args(),
            Self::CargoPackageSet(operation) => operation.display_args(),
            Self::NodeToolchain(operation) => operation.display_args(),
            Self::NpmPackageSet(operation) => operation.display_args(),
            Self::UbuntuSnap(operation) => operation.display_args(),
            Self::UnattendedUpgrades(operation) => operation.display_args(),
            Self::UvBootstrap => vec!["uv-bootstrap".into()],
            Self::PythonToolchain(operation) => operation.display_args(),
            Self::VirtualBoxGroup => vec!["virtualbox-group".into()],
            Self::VsCodeExtensionSet(operation) => operation.display_args(),
        }
    }
}

pub(crate) fn execute(operation: &Operation, env: &[(OsString, OsString)]) -> Result<OperationOutcome> {
    execute_on_host(operation, Host::new(env, Path::new(DOCKER_LOCK))?)
}

fn execute_on_host(operation: &Operation, host: Host<'_>) -> Result<OperationOutcome> {
    match operation {
        Operation::AptBootstrapPackages { packages } => completed(apt::bootstrap_packages(&host, packages)),
        Operation::AptMetadataRefresh => completed(apt::metadata_refresh(&host)),
        Operation::AptRepository(operation) => completed(repository::execute(&host, operation)),
        Operation::ManagedAptSources(operation) => completed(repository::managed_apt::execute(&host, operation)),
        Operation::AptPackages { packages } => completed(apt::packages(&host, packages)),
        Operation::AptPurge { packages } => completed(apt::purge(&host, packages)),
        Operation::AptUpgrade { policy } => completed(apt::upgrade(&host, *policy)),
        Operation::DockerGroup => completed(system::docker_group(&host)),
        Operation::DockerLocalLog(operation) => completed(system::docker_local_log(&host, operation)),
        Operation::DesktopSetting(operation) => completed(system::desktop_setting(&host, operation)),
        Operation::BinaryPackage(package) => completed(binary::execute(&host, package)),
        Operation::Dotfiles(operation) => completed(packages::dotfiles::execute(&host, operation)),
        Operation::FlatpakEnsureFlathub => completed(packages::flatpak::ensure_flathub(&host)),
        Operation::FlatpakEnsureApps { refs } => completed(packages::flatpak::ensure_apps(&host, refs)),
        Operation::FlatpakUpdateApps { refs } => completed(packages::flatpak::update_apps(&host, refs)),
        Operation::FnmBootstrap => completed(languages::fnm_bootstrap(&host)),
        Operation::EnsureAdmin(operation) => completed(system::ensure_admin(&host, operation)),
        Operation::GnomeExtensions(operation) => system::gnome_extensions(&host, operation),
        Operation::GnomeDock(operation) => system::gnome_dock(&host, operation),
        Operation::GnomeRoundedCorners(operation) => system::gnome_rounded_corners(&host, operation),
        Operation::GoToolchain(operation) => completed(tools::execute_go(&host, operation)),
        Operation::NerdFonts(operation) => completed(packages::fonts::execute(&host, operation)),
        Operation::RustupBootstrap => completed(languages::rustup(&host)),
        Operation::CargoBinstallBootstrap(operation) => completed(binary::cargo_binstall::execute(&host, operation)),
        Operation::RustToolchain(operation) => completed(tools::execute_rust(&host, operation)),
        Operation::CargoPackageSet(operation) => completed(packages::cargo::execute(&host, operation)),
        Operation::NodeToolchain(operation) => completed(tools::execute_node(&host, operation)),
        Operation::NpmPackageSet(operation) => completed(packages::npm::execute(&host, operation)),
        Operation::UbuntuSnap(operation) => completed(system::ubuntu_snap(&host, operation)),
        Operation::UnattendedUpgrades(operation) => completed(system::unattended_upgrades(&host, operation)),
        Operation::UvBootstrap => completed(languages::uv_bootstrap(&host)),
        Operation::PythonToolchain(operation) => completed(tools::execute_python(&host, operation)),
        Operation::VirtualBoxGroup => completed(system::virtualbox_group(&host)),
        Operation::VsCodeExtensionSet(operation) => completed(system::vscode_extensions(&host, operation)),
    }
}

fn completed(result: Result<()>) -> Result<OperationOutcome> {
    result.map(|()| OperationOutcome::Completed)
}

pub(crate) struct Host<'a> {
    env: &'a [(OsString, OsString)],
    docker_lock_open_path: &'a Path,
    home: PathBuf,
}

impl<'a> Host<'a> {
    fn new(env: &'a [(OsString, OsString)], docker_lock_open_path: &'a Path) -> Result<Self> {
        let home = resolve_home(env, std::env::var_os("HOME"))?;
        Ok(Self {
            env,
            docker_lock_open_path,
            home,
        })
    }

    pub fn run<I, S>(&self, program: &str, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .collect::<Vec<_>>();
        let mut command = Command::new(program);
        command.args(&args);
        for (key, value) in self.env {
            command.env(key, value);
        }
        retry_executable_busy(|| command.output())
            .with_context(|| format!("{program} operation: start {}", display(program, &args)))
    }

    pub fn require<I, S>(&self, operation: &str, program: &str, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run(program, args)?;
        if !output.status.success() {
            bail!(
                "{operation}: {program} failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(output)
    }

    pub fn require_input<I, S>(&self, operation: &str, program: &str, args: I, input: &[u8]) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .collect::<Vec<_>>();
        let mut command = Command::new(program);
        command.args(&args).stdin(Stdio::piped());
        for (key, value) in self.env {
            command.env(key, value);
        }
        let mut child = retry_executable_busy(|| command.spawn())
            .with_context(|| format!("{operation}: start {}", display(program, &args)))?;
        child
            .stdin
            .take()
            .context("command stdin unavailable")?
            .write_all(input)?;
        let status = child.wait()?;
        if !status.success() {
            bail!("{operation}: {program} failed ({status})");
        }
        Ok(())
    }

    pub fn home(&self) -> PathBuf {
        self.home.clone()
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.value("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"))
    }

    pub fn value(&self, name: &str) -> Option<OsString> {
        self.env
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var_os(name))
    }

    pub fn acquire_docker_lock(&self) -> Result<File> {
        self.require(
            "Docker transaction lock directory symlink check",
            "sudo",
            ["test", "!", "-L", COZYDOT_RUNTIME_DIRECTORY],
        )?;
        self.require(
            "Docker transaction lock directory",
            "sudo",
            [
                "install",
                "-d",
                "-o",
                "root",
                "-g",
                "root",
                "-m",
                "0755",
                "--",
                COZYDOT_RUNTIME_DIRECTORY,
            ],
        )?;
        self.require(
            "Docker transaction lock creation",
            "sudo",
            [
                "cp",
                "--no-clobber",
                "--no-target-directory",
                "--",
                "/dev/null",
                DOCKER_LOCK,
            ],
        )?;
        let kind = self.require(
            "Docker transaction lock type check",
            "sudo",
            ["stat", "--format=%f", "--", DOCKER_LOCK],
        )?;
        let mode = std::str::from_utf8(&kind.stdout)
            .context("Docker transaction lock stat returned non-UTF-8 output")?
            .trim_end();
        let mode =
            u32::from_str_radix(mode, 16).context("Docker transaction lock stat returned malformed mode output")?;
        if mode & 0o170000 != 0o100000 {
            bail!("Docker transaction lock is not a regular file");
        }
        self.require(
            "Docker transaction lock ownership",
            "sudo",
            ["chown", "--no-dereference", "root:root", "--", DOCKER_LOCK],
        )?;
        self.require(
            "Docker transaction lock permissions",
            "sudo",
            ["chmod", "0644", "--", DOCKER_LOCK],
        )?;
        let state = self.require(
            "Docker transaction lock state check",
            "sudo",
            ["stat", "--format=%f:%u:%g", "--", DOCKER_LOCK],
        )?;
        let state = std::str::from_utf8(&state.stdout)
            .context("Docker transaction lock state returned non-UTF-8 output")?
            .trim_end();
        let mut fields = state.split(':');
        let mode = fields.next().and_then(|value| u32::from_str_radix(value, 16).ok());
        let uid = fields.next().and_then(|value| value.parse::<u32>().ok());
        let gid = fields.next().and_then(|value| value.parse::<u32>().ok());
        if fields.next().is_some()
            || mode.is_none_or(|mode| mode & 0o170000 != 0o100000 || mode & 0o7777 != 0o0644)
            || uid != Some(0)
            || gid != Some(0)
        {
            bail!("Docker transaction lock has mismatched type, ownership, or permissions");
        }
        let lock: File = rustix::fs::open(
            self.docker_lock_open_path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .context("Docker transaction lock: open fixed regular lock file without following links")?
        .into();
        if !lock
            .metadata()
            .context("Docker transaction lock: inspect opened lock file")?
            .file_type()
            .is_file()
        {
            bail!("Docker transaction lock opened inode is not a regular file");
        }
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)
            .context("Docker transaction lock: acquire exclusive flock")?;
        Ok(lock)
    }
}

pub(crate) struct TempDir(tempfile::TempDir);

impl TempDir {
    pub fn new(host: &Host<'_>, stem: &str) -> Result<Self> {
        Self::new_in(&host.temp_dir(), stem)
    }

    pub fn new_in(parent: &Path, stem: &str) -> Result<Self> {
        tempfile::Builder::new()
            .prefix(stem)
            .tempdir_in(parent)
            .map(Self)
            .context("create operation temporary directory")
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

pub(crate) struct TempPath(tempfile::TempPath);

impl TempPath {
    pub fn new(host: &Host<'_>, stem: &str) -> Result<Self> {
        Self::new_with_suffix(host, stem, "")
    }

    pub fn new_with_suffix(host: &Host<'_>, stem: &str, suffix: &str) -> Result<Self> {
        tempfile::Builder::new()
            .prefix(stem)
            .suffix(suffix)
            .tempfile_in(host.temp_dir())
            .map(|file| Self(file.into_temp_path()))
            .context("create operation temporary file")
    }

    pub fn path(&self) -> &Path {
        self.0.as_ref()
    }
}

fn display(program: &str, args: &[OsString]) -> String {
    std::iter::once(OsStr::new(program))
        .chain(args.iter().map(OsString::as_os_str))
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

fn retry_executable_busy<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for delay_ms in [1, 2, 4, 8, 16] {
        match operation() {
            Err(error) if error.raw_os_error() == Some(EXECUTABLE_FILE_BUSY) => {
                thread::sleep(Duration::from_millis(delay_ms));
            }
            result => return result,
        }
    }
    operation()
}

fn resolve_home(env: &[(OsString, OsString)], process_home: Option<OsString>) -> Result<PathBuf> {
    env.iter()
        .rev()
        .find(|(key, _)| key == "HOME")
        .map(|(_, value)| PathBuf::from(value))
        .or_else(|| process_home.map(PathBuf::from))
        .context("HOME is not set")
}

pub(crate) mod apt {
    use crate::operations::Host;
    use anyhow::{Context, Result};
    use std::collections::BTreeSet;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AptUpgradePolicy {
        Standard,
        Full,
    }

    pub fn metadata_refresh(host: &Host<'_>) -> Result<()> {
        host.require("APT metadata refresh", "sudo", ["apt-get", "update", "-qq"])?;
        Ok(())
    }

    pub fn bootstrap_packages(host: &Host<'_>, packages: &[String]) -> Result<()> {
        if packages.is_empty() {
            anyhow::bail!("APT bootstrap package sequence must not be empty");
        }
        let missing = select_packages(host, packages, false)?;
        if missing.is_empty() {
            return Ok(());
        }
        host.require("APT bootstrap metadata refresh", "sudo", ["apt-get", "update", "-qq"])?;
        install(host, "APT bootstrap package installation", missing)
    }

    pub fn packages(host: &Host<'_>, packages: &[String]) -> Result<()> {
        let missing = select_packages(host, packages, false)?;
        if missing.is_empty() {
            return Ok(());
        }
        install(host, "APT package installation", missing)
    }

    fn install(host: &Host<'_>, operation: &str, packages: Vec<String>) -> Result<()> {
        let mut args = vec![
            "DEBIAN_FRONTEND=noninteractive".to_owned(),
            "apt-get".to_owned(),
            "install".into(),
            "-y".into(),
            "-qq".into(),
            "--".into(),
        ];
        args.extend(packages.into_iter().map(|package| format!("{package}+")));
        host.require(operation, "sudo", args)?;
        Ok(())
    }

    pub fn purge(host: &Host<'_>, packages: &[String]) -> Result<()> {
        let installed = select_packages(host, packages, true)?;
        if installed.is_empty() {
            return Ok(());
        }
        let mut args = vec![
            "DEBIAN_FRONTEND=noninteractive".to_owned(),
            "apt-get".to_owned(),
            "purge".into(),
            "-y".into(),
            "-qq".into(),
            "--".into(),
        ];
        args.extend(installed.into_iter().map(|package| format!("{package}-")));
        host.require("APT package purge", "sudo", args)?;
        Ok(())
    }

    pub fn upgrade(host: &Host<'_>, policy: AptUpgradePolicy) -> Result<()> {
        match policy {
            AptUpgradePolicy::Standard => {
                host.require(
                    "APT standard upgrade",
                    "sudo",
                    [
                        "DEBIAN_FRONTEND=noninteractive",
                        "apt-get",
                        "upgrade",
                        "-y",
                        "-qq",
                        "--",
                    ],
                )?;
            }
            AptUpgradePolicy::Full => {
                host.require(
                    "APT full upgrade",
                    "sudo",
                    [
                        "DEBIAN_FRONTEND=noninteractive",
                        "apt-get",
                        "full-upgrade",
                        "-y",
                        "-qq",
                        "--",
                    ],
                )?;
                host.require(
                    "APT purge autoremove",
                    "sudo",
                    [
                        "DEBIAN_FRONTEND=noninteractive",
                        "apt-get",
                        "autoremove",
                        "--purge",
                        "-y",
                        "-qq",
                        "--",
                    ],
                )?;
            }
        }
        Ok(())
    }

    fn select_packages(host: &Host<'_>, packages: &[String], select_installed: bool) -> Result<Vec<String>> {
        if packages.is_empty() {
            return Ok(Vec::new());
        }
        let mut requested = BTreeSet::new();
        for package in packages {
            validate_package_name(package)?;
            if !requested.insert(package.as_str()) {
                anyhow::bail!("APT package state query has duplicate requested package: {package:?}");
            }
        }
        let mut args = vec![
            "-W".to_owned(),
            "-f=${Package}\\t${db:Status-Abbrev}\\n".into(),
            "--".into(),
        ];
        args.extend(packages.iter().cloned());
        let output = host.run("dpkg-query", args)?;
        if !output.status.success() && output.status.code() != Some(1) {
            anyhow::bail!(
                "APT package state query: dpkg-query failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let installed = installed_packages(&output.stdout, &requested, output.status.success())?;
        Ok(packages
            .iter()
            .filter(|package| installed.contains(package.as_str()) == select_installed)
            .cloned()
            .collect())
    }

    fn validate_package_name(package: &str) -> Result<()> {
        let mut bytes = package.bytes();
        let valid = bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'.' | b'-'));
        if !valid {
            anyhow::bail!("invalid canonical Debian package name: {package:?}");
        }
        Ok(())
    }

    fn installed_packages<'a>(
        output: &'a [u8],
        requested: &BTreeSet<&str>,
        require_complete: bool,
    ) -> Result<BTreeSet<&'a str>> {
        let output = std::str::from_utf8(output).context("dpkg-query returned non-UTF-8 package state")?;
        let mut installed = BTreeSet::new();
        let mut returned = BTreeSet::new();
        for line in output.lines().filter(|line| !line.is_empty()) {
            let Some((package, status)) = line.split_once('\t') else {
                anyhow::bail!("dpkg-query returned malformed package state: {line:?}");
            };
            let status = status.as_bytes();
            if package.is_empty()
                || status.len() != 3
                || !matches!(status[0], b'u' | b'i' | b'h' | b'r' | b'p')
                || !matches!(status[1], b'n' | b'c' | b'H' | b'U' | b'F' | b'W' | b't' | b'i')
                || !matches!(status[2], b' ' | b'R')
            {
                anyhow::bail!("dpkg-query returned malformed package state: {line:?}");
            }
            if !requested.contains(package) {
                anyhow::bail!("dpkg-query returned unrequested package record: {package:?}");
            }
            if !returned.insert(package) {
                anyhow::bail!("dpkg-query returned duplicate package record: {package:?}");
            }
            if status[1] == b'i' {
                installed.insert(package);
            }
        }
        if require_complete && returned.len() != requested.len() {
            let missing = requested
                .iter()
                .filter(|package| !returned.contains(**package))
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("dpkg-query returned incomplete package state; missing records for: {missing}");
        }
        Ok(installed)
    }
}

pub(crate) mod languages {
    use anyhow::{bail, Context, Result};
    use std::{os::unix::fs::PermissionsExt, path::PathBuf};

    use crate::operations::{Host, TempPath};

    pub fn fnm_bootstrap(host: &Host<'_>) -> Result<()> {
        let data_home = host
            .value("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".local/share"));
        if !data_home.is_absolute() {
            bail!("FNM managed data directory must be absolute");
        }
        let installed = data_home.join("fnm/fnm");
        if executable_file(&installed) {
            return Ok(());
        }
        let installer = TempPath::new(host, "fnm-install")?;
        host.require(
            "FNM bootstrap download",
            "curl",
            [
                "-fsSL",
                "-o",
                &installer.path().to_string_lossy(),
                "https://fnm.vercel.app/install",
            ],
        )?;
        host.require(
            "FNM bootstrap",
            "bash",
            [&installer.path().to_string_lossy(), "--skip-shell"],
        )?;
        if !executable_file(&installed) {
            bail!("FNM bootstrap did not publish executable {}", installed.display());
        }
        Ok(())
    }

    pub fn uv_bootstrap(host: &Host<'_>) -> Result<()> {
        let install_dir = host
            .value("UV_INSTALL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".local/bin"));
        if !install_dir.is_absolute() {
            bail!("UV managed install directory must be absolute");
        }
        let installed = install_dir.join("uv");
        if executable_file(&installed) {
            return Ok(());
        }
        let installer = TempPath::new(host, "uv-install")?;
        host.require(
            "UV bootstrap download",
            "curl",
            [
                "-LsSf",
                "-o",
                &installer.path().to_string_lossy(),
                "https://astral.sh/uv/install.sh",
            ],
        )?;
        std::fs::create_dir_all(&install_dir).context("UV bootstrap: create install directory")?;
        host.require(
            "UV bootstrap",
            "env",
            vec![
                format!("UV_UNMANAGED_INSTALL={}", install_dir.display()),
                "sh".into(),
                installer.path().to_string_lossy().into_owned(),
            ],
        )?;
        if !executable_file(&installed) {
            bail!("UV bootstrap did not publish executable {}", installed.display());
        }
        Ok(())
    }

    fn executable_file(path: &std::path::Path) -> bool {
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0)
    }

    pub fn rustup(host: &Host<'_>) -> Result<()> {
        let cargo_home = host
            .value("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".cargo"));
        if !cargo_home.is_absolute() {
            bail!("rustup managed CARGO_HOME must be absolute");
        }
        if executable_file(&cargo_home.join("bin/rustup")) {
            return Ok(());
        }
        let installer = TempPath::new(host, "rustup")?;
        host.require(
            "rustup bootstrap download",
            "curl",
            [
                "--proto",
                "=https",
                "--tlsv1.2",
                "-sSf",
                "-o",
                &installer.path().to_string_lossy(),
                "https://sh.rustup.rs",
            ],
        )?;
        host.require("rustup bootstrap", "sh", [installer.path().as_os_str(), "-y".as_ref()])?;
        if !executable_file(&cargo_home.join("bin/rustup")) {
            bail!("rustup bootstrap did not publish the managed rustup executable");
        }
        Ok(())
    }
}

pub(crate) mod managed_state {
    use crate::operations::Host;
    use anyhow::{bail, Context, Result};
    use serde::{
        de::{DeserializeOwned, MapAccess, SeqAccess, Visitor},
        Deserialize, Deserializer,
    };
    use std::{
        fmt,
        fs::{self, File},
        io::{Read, Write},
        os::unix::fs::MetadataExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn parse_strict_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
        let strict: StrictValue = serde_json::from_slice(bytes)?;
        serde_json::from_value(strict.0).context("deserialize strict JSON value")
    }

    struct StrictValue(serde_json::Value);

    impl<'de> Deserialize<'de> for StrictValue {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(StrictValueVisitor)
        }
    }

    struct StrictValueVisitor;

    impl<'de> Visitor<'de> for StrictValueVisitor {
        type Value = StrictValue;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("JSON without duplicate object keys")
        }

        fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
            Ok(StrictValue(value.into()))
        }
        fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
            Ok(StrictValue(value.into()))
        }
        fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
            Ok(StrictValue(value.into()))
        }
        fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            serde_json::Number::from_f64(value)
                .map(|number| StrictValue(number.into()))
                .ok_or_else(|| E::custom("invalid JSON number"))
        }
        fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
            Ok(StrictValue(value.into()))
        }
        fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
            Ok(StrictValue(value.into()))
        }
        fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(StrictValue(serde_json::Value::Null))
        }
        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(StrictValue(serde_json::Value::Null))
        }
        fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut values = Vec::new();
            while let Some(value) = sequence.next_element::<StrictValue>()? {
                values.push(value.0);
            }
            Ok(StrictValue(values.into()))
        }
        fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut values = serde_json::Map::new();
            while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
                if values.insert(key.clone(), value.0).is_some() {
                    return Err(serde::de::Error::custom(format!("duplicate JSON key {key:?}")));
                }
            }
            Ok(StrictValue(values.into()))
        }
    }

    pub(crate) struct ManagedState {
        directory: File,
        record_name: String,
        lock_name: String,
        label: &'static str,
    }

    impl ManagedState {
        pub(crate) fn open(host: &Host<'_>, component: &str, stem: &str, label: &'static str) -> Result<Self> {
            validate_component(component)?;
            validate_stem(stem)?;
            let state_home = host
                .value("XDG_STATE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| host.home().join(".local/state"));
            if !state_home.is_absolute() {
                bail!("{label} state directory must be absolute");
            }
            let root_existed = fs::symlink_metadata(&state_home).is_ok();
            fs::create_dir_all(&state_home).context("create selected managed-state root")?;
            let mut directory = open_directory_path(&state_home, "selected managed-state root")?;
            if !root_existed {
                rustix::fs::fchmod(&directory, rustix::fs::Mode::from_bits_truncate(0o700))
                    .context("restrict selected managed-state root")?;
            }
            validate_state_directory(&directory, "selected managed-state root")?;
            directory = open_or_create_state_directory(&directory, "cozydot")?;
            directory = open_or_create_state_directory(&directory, component)?;
            Ok(Self {
                directory,
                record_name: format!("{stem}.json"),
                lock_name: format!("{stem}.lock"),
                label,
            })
        }

        pub(crate) fn acquire_lock(&self) -> Result<File> {
            let flags = rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC;
            let (lock, created): (File, bool) = match rustix::fs::openat(
                &self.directory,
                self.lock_name.as_str(),
                flags,
                rustix::fs::Mode::from_bits_truncate(0o600),
            ) {
                Ok(lock) => (lock.into(), true),
                Err(rustix::io::Errno::EXIST) => (
                    rustix::fs::openat(
                        &self.directory,
                        self.lock_name.as_str(),
                        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
                        rustix::fs::Mode::empty(),
                    )
                    .with_context(|| {
                        format!(
                            "open existing {} managed-state lock without following links",
                            self.label
                        )
                    })?
                    .into(),
                    false,
                ),
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("create {} managed-state lock without following links", self.label))
                }
            };
            if created {
                rustix::fs::fchmod(&lock, rustix::fs::Mode::from_bits_truncate(0o600))?;
            }
            validate_state_file(&lock, &format!("{} managed-state lock", self.label))?;
            rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)
                .with_context(|| format!("lock {} managed state", self.label))?;
            self.validate_lock_entry(&lock)?;
            Ok(lock)
        }

        pub(crate) fn validate_lock_entry(&self, lock: &File) -> Result<()> {
            validate_state_file(lock, &format!("{} managed-state lock", self.label))?;
            let entry = rustix::fs::statat(
                &self.directory,
                self.lock_name.as_str(),
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .with_context(|| format!("reinspect {} managed-state lock entry", self.label))?;
            let metadata = lock.metadata()?;
            if entry.st_dev != metadata.dev() || entry.st_ino != metadata.ino() {
                bail!("{} managed-state lock entry was replaced", self.label);
            }
            Ok(())
        }

        pub(crate) fn read(&self) -> Result<Option<Vec<u8>>> {
            let descriptor = match rustix::fs::openat(
                &self.directory,
                self.record_name.as_str(),
                rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            ) {
                Ok(value) => value,
                Err(rustix::io::Errno::NOENT) => return Ok(None),
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("open {} managed record without following links", self.label))
                }
            };
            read_validated_file(descriptor.into(), &format!("{} managed record", self.label)).map(Some)
        }

        pub(crate) fn publish(&self, bytes: &[u8]) -> Result<()> {
            let temporary_name = format!(
                ".{}.{}.{}.tmp",
                self.record_name,
                std::process::id(),
                TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
            );
            let mut temporary: File = rustix::fs::openat(
                &self.directory,
                temporary_name.as_str(),
                rustix::fs::OFlags::WRONLY
                    | rustix::fs::OFlags::CREATE
                    | rustix::fs::OFlags::EXCL
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::from_bits_truncate(0o600),
            )
            .with_context(|| format!("create {} managed-record staging file", self.label))?
            .into();
            let result = (|| {
                rustix::fs::fchmod(&temporary, rustix::fs::Mode::from_bits_truncate(0o600))?;
                validate_state_file(&temporary, &format!("{} managed-record staging file", self.label))?;
                temporary.write_all(bytes)?;
                temporary.sync_all()?;
                rustix::fs::renameat(
                    &self.directory,
                    temporary_name.as_str(),
                    &self.directory,
                    self.record_name.as_str(),
                )
                .with_context(|| format!("publish {} managed record", self.label))?;
                self.directory
                    .sync_all()
                    .with_context(|| format!("sync {} managed-state directory", self.label))?;
                Ok(())
            })();
            if result.is_err() {
                let _ = rustix::fs::unlinkat(&self.directory, temporary_name.as_str(), rustix::fs::AtFlags::empty());
            }
            result
        }
    }

    fn read_validated_file(mut file: File, label: &str) -> Result<Vec<u8>> {
        validate_state_file(&file, label)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .with_context(|| format!("read {label} descriptor"))?;
        Ok(bytes)
    }

    fn validate_component(value: &str) -> Result<()> {
        if value.is_empty()
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            bail!("invalid managed-state component");
        }
        Ok(())
    }
    fn validate_stem(value: &str) -> Result<()> {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
            bail!("invalid managed-state stem");
        }
        Ok(())
    }
    fn open_directory_path(path: &std::path::Path, label: &str) -> Result<File> {
        Ok(rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .with_context(|| format!("open {label} without following links"))?
        .into())
    }
    fn open_or_create_state_directory(parent: &File, name: &str) -> Result<File> {
        let created = match rustix::fs::mkdirat(parent, name, rustix::fs::Mode::from_bits_truncate(0o700)) {
            Ok(()) => true,
            Err(rustix::io::Errno::EXIST) => false,
            Err(error) => return Err(error).with_context(|| format!("create managed-state {name}")),
        };
        let directory: File = rustix::fs::openat(
            parent,
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .with_context(|| format!("open managed-state {name} without following links"))?
        .into();
        if created {
            rustix::fs::fchmod(&directory, rustix::fs::Mode::from_bits_truncate(0o700))?;
        }
        validate_state_directory(&directory, &format!("managed-state {name}"))?;
        Ok(directory)
    }
    fn validate_state_directory(directory: &File, label: &str) -> Result<()> {
        let metadata = directory.metadata().with_context(|| format!("inspect {label}"))?;
        if !metadata.file_type().is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o022 != 0
        {
            bail!("{label} has unsafe type, owner, or permissions");
        }
        Ok(())
    }
    fn validate_state_file(file: &File, label: &str) -> Result<()> {
        let metadata = file.metadata().with_context(|| format!("inspect {label}"))?;
        if !metadata.file_type().is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o7777 != 0o600
            || metadata.nlink() != 1
        {
            bail!("{label} has unsafe type, owner, permissions, or link count");
        }
        Ok(())
    }
}

pub(crate) mod packages {

    pub(crate) mod cargo {
        use anyhow::{bail, Context, Result};
        use semver::Version;
        use std::{
            collections::BTreeSet,
            os::unix::fs::PermissionsExt,
            path::{Path, PathBuf},
        };

        use super::super::Host;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum CargoPackageMode {
            EnsurePresent,
            UpdateCurrent,
        }

        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct CargoPackageOperation {
            packages: Vec<String>,
            mode: CargoPackageMode,
        }

        impl CargoPackageOperation {
            pub fn new(packages: Vec<String>, mode: CargoPackageMode) -> Result<Self> {
                validate_packages(&packages)?;
                Ok(Self { packages, mode })
            }

            pub(crate) fn display_args(&self) -> Vec<String> {
                std::iter::once("cargo-package-set".into())
                    .chain(std::iter::once(
                        match self.mode {
                            CargoPackageMode::EnsurePresent => "ensure-present",
                            CargoPackageMode::UpdateCurrent => "update-current",
                        }
                        .into(),
                    ))
                    .chain(self.packages.iter().cloned())
                    .collect()
            }
        }

        pub(crate) fn execute(host: &Host<'_>, operation: &CargoPackageOperation) -> Result<()> {
            validate_packages(&operation.packages).context("validate Cargo package operation")?;
            let cargo_home = host
                .value("CARGO_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| host.home().join(".cargo"));
            if !cargo_home.is_absolute() {
                bail!("Cargo package operation requires an absolute CARGO_HOME");
            }
            let cargo = resolve_cargo(host, &cargo_home)?;
            let installed = inspect_installed(host, &cargo)?;
            let selected = match operation.mode {
                CargoPackageMode::EnsurePresent => operation
                    .packages
                    .iter()
                    .filter(|package| !installed.contains(package.as_str()))
                    .cloned()
                    .collect::<Vec<_>>(),
                CargoPackageMode::UpdateCurrent => operation.packages.clone(),
            };
            if selected.is_empty() {
                return Ok(());
            }

            let binstall = resolve_binstall(&cargo_home)?
                .context("Cargo package operation: managed cargo-binstall is unavailable after bootstrap")?;
            let mut args = vec!["--no-confirm".to_owned()];
            if operation.mode == CargoPackageMode::UpdateCurrent {
                args.push("--force".into());
            }
            args.extend(selected);
            host.require("Cargo package mutation", &binstall, args)?;

            let installed = inspect_installed(host, &cargo)?;
            require_packages(&operation.packages, &installed)
        }

        fn resolve_cargo(_host: &Host<'_>, cargo_home: &Path) -> Result<String> {
            let managed = cargo_home.join("bin/cargo");
            if executable_file(&managed) {
                return path_program(&managed, "Cargo executable path");
            }
            bail!("Cargo package operation: managed Cargo is unavailable after Rust bootstrap")
        }

        fn resolve_binstall(cargo_home: &Path) -> Result<Option<String>> {
            let managed = cargo_home.join("bin/cargo-binstall");
            if executable_file(&managed) {
                return path_program(&managed, "cargo-binstall executable path").map(Some);
            }
            Ok(None)
        }

        fn inspect_installed(host: &Host<'_>, cargo: &str) -> Result<BTreeSet<String>> {
            let output = host.require("Cargo installed package query", cargo, ["install", "--list"])?;
            installed_packages(&output.stdout)
        }

        fn installed_packages(output: &[u8]) -> Result<BTreeSet<String>> {
            let output = std::str::from_utf8(output).context("cargo returned non-UTF-8 installed package state")?;
            let mut installed = BTreeSet::new();
            for line in output.lines().filter(|line| !line.is_empty()) {
                if line.starts_with(char::is_whitespace) {
                    continue;
                }
                let Some((package, version_and_source)) = line.split_once(" v") else {
                    bail!("cargo returned malformed installed package state: {line:?}");
                };
                validate_package(package)
                    .map_err(|_| anyhow::anyhow!("cargo returned malformed installed package state: {line:?}"))?;
                let Some(record) = version_and_source.strip_suffix(':') else {
                    bail!("cargo returned malformed installed package state: {line:?}");
                };
                let (version, source) = record
                    .split_once(" (")
                    .map_or((record, None), |parts| (parts.0, parts.1.strip_suffix(')')));
                if Version::parse(version).is_err()
                    || record.contains(" (") && source.is_none()
                    || source.is_some_and(|source| !valid_display_source(source))
                {
                    bail!("cargo returned malformed installed package state: {line:?}");
                }
                if source.is_none() && !installed.insert(package.to_owned()) {
                    bail!("cargo returned duplicate installed registry package: {package:?}");
                }
            }
            Ok(installed)
        }

        fn valid_display_source(source: &str) -> bool {
            if source.is_empty() || source.chars().any(char::is_control) {
                return false;
            }
            let mut depth = 0_u32;
            for character in source.chars() {
                match character {
                    '(' => depth += 1,
                    ')' => {
                        let Some(next) = depth.checked_sub(1) else {
                            return false;
                        };
                        depth = next;
                    }
                    _ => {}
                }
            }
            depth == 0
        }

        fn require_packages(packages: &[String], installed: &BTreeSet<String>) -> Result<()> {
            let missing = packages
                .iter()
                .filter(|package| !installed.contains(package.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                bail!(
                    "Cargo package mutation did not install configured packages: {}",
                    missing.join(", ")
                );
            }
            Ok(())
        }

        fn validate_packages(packages: &[String]) -> Result<()> {
            if packages.is_empty() {
                bail!("Cargo package sequence must not be empty");
            }
            let mut seen = BTreeSet::new();
            for package in packages {
                validate_package(package)?;
                if !seen.insert(package.as_str()) {
                    bail!("duplicate Cargo package name: {package:?}");
                }
            }
            Ok(())
        }

        fn validate_package(package: &str) -> Result<()> {
            let mut bytes = package.bytes();
            let valid = bytes.next().is_some_and(|byte| byte.is_ascii_alphanumeric())
                && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
            if !valid {
                bail!("invalid unversioned Cargo package name: {package:?}");
            }
            Ok(())
        }

        fn executable_file(path: &Path) -> bool {
            std::fs::metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        }

        fn path_program(path: &Path, description: &str) -> Result<String> {
            path.to_str()
                .map(str::to_owned)
                .with_context(|| format!("{description} is not UTF-8: {}", path.display()))
        }
    }

    pub(crate) mod npm {
        use anyhow::{bail, Context, Result};
        use serde_json::{Map, Value};
        use std::{
            collections::BTreeSet,
            os::unix::fs::PermissionsExt,
            path::{Path, PathBuf},
        };

        use super::super::{managed_state::parse_strict_json, Host};

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum NpmPackageMode {
            EnsurePresent,
            UpdateCurrent,
        }

        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct NpmPackageOperation {
            packages: Vec<String>,
            mode: NpmPackageMode,
        }

        impl NpmPackageOperation {
            pub fn new(packages: Vec<String>, mode: NpmPackageMode) -> Result<Self> {
                validate_packages(&packages)?;
                Ok(Self { packages, mode })
            }

            pub(crate) fn display_args(&self) -> Vec<String> {
                std::iter::once("npm-package-set".into())
                    .chain(std::iter::once(
                        match self.mode {
                            NpmPackageMode::EnsurePresent => "ensure-present",
                            NpmPackageMode::UpdateCurrent => "update-current",
                        }
                        .into(),
                    ))
                    .chain(self.packages.iter().cloned())
                    .collect()
            }
        }

        pub(crate) fn execute(host: &Host<'_>, operation: &NpmPackageOperation) -> Result<()> {
            validate_packages(&operation.packages).context("validate npm package operation")?;
            let fnm = resolve_fnm(host)?;
            let version = selected_version(host, &fnm)?;
            let installed = inspect_installed(host, &fnm, &version)?;
            let selected = match operation.mode {
                NpmPackageMode::EnsurePresent => operation
                    .packages
                    .iter()
                    .filter(|package| !installed.contains(package.as_str()))
                    .cloned()
                    .collect::<Vec<_>>(),
                NpmPackageMode::UpdateCurrent => operation.packages.clone(),
            };
            if selected.is_empty() {
                return Ok(());
            }

            let mut npm_args = vec!["install".to_owned(), "--global".into(), "--".into()];
            npm_args.extend(selected);
            run_npm_required(host, &fnm, &version, "npm package mutation", npm_args)?;

            let installed = inspect_installed(host, &fnm, &version)?;
            require_packages(&operation.packages, &installed)
        }

        fn resolve_fnm(host: &Host<'_>) -> Result<String> {
            let data_home = host
                .value("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| host.home().join(".local/share"));
            if !data_home.is_absolute() {
                bail!("npm package operation requires an absolute managed FNM data directory");
            }
            let managed = data_home.join("fnm/fnm");
            if executable_file(&managed) {
                return managed
                    .to_str()
                    .map(str::to_owned)
                    .context("managed fnm executable path is not UTF-8");
            }
            bail!("npm package operation: managed fnm is unavailable after bootstrap")
        }

        fn selected_version(host: &Host<'_>, fnm: &str) -> Result<String> {
            let output = host.require("fnm default Node query", fnm, ["default"])?;
            let output = std::str::from_utf8(&output.stdout).context("fnm returned non-UTF-8 default Node version")?;
            let version = output.strip_suffix('\n').unwrap_or(output);
            if version.contains(['\n', '\r']) || !valid_node_version(version) {
                bail!("fnm returned invalid default Node version: {version:?}");
            }
            Ok(version.to_owned())
        }

        fn inspect_installed(host: &Host<'_>, fnm: &str, version: &str) -> Result<BTreeSet<String>> {
            let output = run_npm_required(
                host,
                fnm,
                version,
                "npm global package query",
                ["list", "--global", "--depth=0", "--json"],
            )?;
            installed_packages(&output.stdout)
        }

        fn run_npm_required<I, S>(
            host: &Host<'_>,
            fnm: &str,
            version: &str,
            operation: &str,
            npm_args: I,
        ) -> Result<std::process::Output>
        where
            I: IntoIterator<Item = S>,
            S: AsRef<str>,
        {
            let mut args = vec![
                "exec".to_owned(),
                "--using".into(),
                version.to_owned(),
                "--".into(),
                "npm".into(),
            ];
            args.extend(npm_args.into_iter().map(|arg| arg.as_ref().to_owned()));
            host.require(operation, fnm, args)
        }

        fn installed_packages(output: &[u8]) -> Result<BTreeSet<String>> {
            let output = std::str::from_utf8(output).context("npm returned non-UTF-8 global package state")?;
            let root: Value = parse_strict_json(output.as_bytes()).context("npm returned malformed JSON state")?;
            let root = root
                .as_object()
                .context("npm global package state must be a JSON object")?;
            reject_problem_state(root, "npm global package state")?;
            if root.contains_key("error") {
                bail!("npm global package state reported an error");
            }
            let dependencies = match root.get("dependencies") {
                Some(dependencies) => dependencies
                    .as_object()
                    .context("npm global package state dependencies must be a JSON object")?,
                None => return Ok(BTreeSet::new()),
            };
            let mut installed = BTreeSet::new();
            for (package, metadata) in dependencies {
                validate_package(package)
                    .map_err(|_| anyhow::anyhow!("npm returned invalid global package name: {package:?}"))?;
                let metadata = metadata
                    .as_object()
                    .with_context(|| format!("npm global package metadata for {package:?} must be a JSON object"))?;
                reject_problem_state(metadata, &format!("npm global package {package:?}"))?;
                if metadata.contains_key("error") {
                    bail!("npm global package {package:?} reported an error");
                }
                let version = metadata
                    .get("version")
                    .and_then(Value::as_str)
                    .with_context(|| format!("npm global package {package:?} must report a string version"))?;
                if version.is_empty() || version.chars().any(char::is_control) {
                    bail!("npm global package {package:?} reported an invalid version");
                }
                for flag in ["invalid", "missing"] {
                    if metadata.get(flag).is_some_and(|value| value != &Value::Bool(false)) {
                        bail!("npm global package {package:?} reported {flag} state");
                    }
                }
                installed.insert(package.clone());
            }
            Ok(installed)
        }

        fn reject_problem_state(object: &Map<String, Value>, description: &str) -> Result<()> {
            if let Some(problems) = object.get("problems") {
                let problems = problems
                    .as_array()
                    .with_context(|| format!("{description} problems must be a JSON array"))?;
                if !problems.is_empty() {
                    bail!("{description} reported problems");
                }
            }
            Ok(())
        }

        fn require_packages(packages: &[String], installed: &BTreeSet<String>) -> Result<()> {
            let missing = packages
                .iter()
                .filter(|package| !installed.contains(package.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                bail!(
                    "npm package mutation did not install configured packages: {}",
                    missing.join(", ")
                );
            }
            Ok(())
        }

        fn validate_packages(packages: &[String]) -> Result<()> {
            if packages.is_empty() {
                bail!("npm package sequence must not be empty");
            }
            let mut seen = BTreeSet::new();
            for package in packages {
                validate_package(package)?;
                if !seen.insert(package.as_str()) {
                    bail!("duplicate npm package name: {package:?}");
                }
            }
            Ok(())
        }

        fn validate_package(package: &str) -> Result<()> {
            let valid_part = |part: &str| {
                let mut bytes = part.bytes();
                bytes
                    .next()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                    && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte))
            };
            let valid = if let Some(scoped) = package.strip_prefix('@') {
                let mut parts = scoped.split('/');
                valid_part(parts.next().unwrap_or_default())
                    && valid_part(parts.next().unwrap_or_default())
                    && parts.next().is_none()
            } else {
                !package.contains('/') && valid_part(package)
            };
            if !valid {
                bail!("invalid unversioned lowercase npm package name: {package:?}");
            }
            Ok(())
        }

        fn valid_node_version(version: &str) -> bool {
            let Some(version) = version.strip_prefix('v') else {
                return false;
            };
            let parts = version.split('.').collect::<Vec<_>>();
            parts.len() == 3
                && parts.iter().all(|part| {
                    !part.is_empty()
                        && part.bytes().all(|byte| byte.is_ascii_digit())
                        && (*part == "0" || !part.starts_with('0'))
                })
        }

        fn executable_file(path: &Path) -> bool {
            std::fs::metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        }
    }

    pub(crate) mod flatpak {
        use super::super::Host;
        use anyhow::{Context, Result};
        use std::collections::{BTreeMap, BTreeSet};

        const FLATHUB_NAME: &str = "flathub";
        const FLATHUB_DESCRIPTOR_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
        const FLATHUB_REPOSITORY_URL: &str = "https://dl.flathub.org/repo/";

        pub fn ensure_flathub(host: &Host<'_>) -> Result<()> {
            if !validate_flathub(&inspect_user_remotes(host)?)? {
                host.require(
                    "Flathub remote ensure",
                    "flatpak",
                    ["--user", "remote-add", FLATHUB_NAME, FLATHUB_DESCRIPTOR_URL],
                )?;
                require_flathub(&inspect_user_remotes(host)?)?;
            }
            host.require(
                "Flathub dependency use enablement",
                "flatpak",
                ["--user", "remote-modify", "--use-for-deps", FLATHUB_NAME],
            )?;
            require_flathub(&inspect_user_remotes(host)?)?;
            Ok(())
        }

        pub fn ensure_apps(host: &Host<'_>, refs: &[String]) -> Result<()> {
            validate_refs(refs)?;
            let output = host.run("flatpak", ["--user", "list", "--app", "--columns=application"])?;
            if !output.status.success() {
                anyhow::bail!(
                    "Flatpak installed application query: flatpak failed ({}): {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let installed = installed_apps(&output.stdout)?;
            let missing = refs
                .iter()
                .filter(|app| !installed.contains(app.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            if missing.is_empty() {
                return Ok(());
            }
            let mut args = vec![
                "--user".to_owned(),
                "install".into(),
                "--app".into(),
                "--noninteractive".into(),
                "-y".into(),
                "flathub".into(),
                "--".into(),
            ];
            args.extend(missing);
            host.require("Flatpak application installation", "flatpak", args)?;
            Ok(())
        }

        pub fn update_apps(host: &Host<'_>, refs: &[String]) -> Result<()> {
            validate_refs(refs)?;
            let mut args = vec![
                "--user".to_owned(),
                "update".into(),
                "--app".into(),
                "--noninteractive".into(),
                "-y".into(),
                "--".into(),
            ];
            args.extend(refs.iter().cloned());
            host.require("Flatpak configured application update", "flatpak", args)?;
            Ok(())
        }

        fn validate_refs(refs: &[String]) -> Result<()> {
            if refs.is_empty() {
                anyhow::bail!("Flatpak application sequence must not be empty");
            }
            let mut unique = BTreeSet::new();
            for app in refs {
                validate_app_id(app)?;
                if !unique.insert(app.as_str()) {
                    anyhow::bail!("duplicate Flatpak application ID: {app:?}");
                }
            }
            Ok(())
        }

        fn installed_apps(output: &[u8]) -> Result<BTreeSet<&str>> {
            let output =
                std::str::from_utf8(output).context("flatpak returned non-UTF-8 installed application state")?;
            let mut installed = BTreeSet::new();
            for app in output.lines() {
                validate_app_id(app)
                    .map_err(|_| anyhow::anyhow!("flatpak returned malformed installed application ID: {app:?}"))?;
                installed.insert(app);
            }
            Ok(installed)
        }

        struct UserRemote {
            url: String,
            options: BTreeSet<String>,
            filter: String,
        }

        fn inspect_user_remotes(host: &Host<'_>) -> Result<BTreeMap<String, UserRemote>> {
            let output = host.run(
                "flatpak",
                [
                    "--user",
                    "remotes",
                    "--show-disabled",
                    "--columns=name,url,options,filter",
                ],
            )?;
            if !output.status.success() {
                anyhow::bail!(
                    "Flathub remote query: flatpak failed ({}): {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            user_remotes(&output.stdout)
        }

        fn require_flathub(remotes: &BTreeMap<String, UserRemote>) -> Result<()> {
            if !validate_flathub(remotes)? {
                anyhow::bail!(
                    "Flathub remote mismatch: expected the per-user {FLATHUB_NAME:?} remote to exist after mutation"
                );
            }
            Ok(())
        }

        fn validate_flathub(remotes: &BTreeMap<String, UserRemote>) -> Result<bool> {
            let Some(remote) = remotes.get(FLATHUB_NAME) else {
                return Ok(false);
            };
            let insecure = remote
                .options
                .iter()
                .find(|option| matches!(option.as_str(), "disabled" | "no-gpg-verify" | "no-enumerate"));
            if remote.url != FLATHUB_REPOSITORY_URL || insecure.is_some() || remote.filter != "-" {
                let options = remote.options.iter().cloned().collect::<Vec<_>>().join(",");
                anyhow::bail!(
                    "Flathub remote mismatch: expected URL {FLATHUB_REPOSITORY_URL:?} with GPG verification and enumeration enabled and no local filter; found URL {:?}, options {options:?}, and filter {:?}. Repair or remove the per-user {FLATHUB_NAME:?} remote and retry",
                    remote.url,
                    remote.filter
                );
            }
            Ok(true)
        }

        fn user_remotes(output: &[u8]) -> Result<BTreeMap<String, UserRemote>> {
            let output = std::str::from_utf8(output).context("flatpak returned non-UTF-8 per-user remote state")?;
            if output.trim().is_empty() {
                return Ok(BTreeMap::new());
            }
            let mut remotes = BTreeMap::new();
            for line in output.lines() {
                let mut fields = line.split('\t');
                let (Some(name), Some(url), Some(options), Some(filter), None) = (
                    fields.next(),
                    fields.next(),
                    fields.next(),
                    fields.next(),
                    fields.next(),
                ) else {
                    anyhow::bail!("flatpak returned malformed per-user remote state: {line:?}");
                };
                if name.is_empty() || url.is_empty() || filter.is_empty() || url::Url::parse(url).is_err() {
                    anyhow::bail!("flatpak returned malformed per-user remote state: {line:?}");
                }
                let options = if options.is_empty() {
                    BTreeSet::new()
                } else {
                    let parsed = options.split(',').collect::<BTreeSet<_>>();
                    if parsed.contains("") || parsed.len() != options.split(',').count() {
                        anyhow::bail!("flatpak returned malformed per-user remote state: {line:?}");
                    }
                    parsed.into_iter().map(str::to_owned).collect()
                };
                if remotes
                    .insert(
                        name.to_owned(),
                        UserRemote {
                            url: url.to_owned(),
                            options,
                            filter: filter.to_owned(),
                        },
                    )
                    .is_some()
                {
                    anyhow::bail!("flatpak returned duplicate per-user remote name: {name:?}");
                }
            }
            Ok(remotes)
        }

        fn validate_app_id(app: &str) -> Result<()> {
            let mut count = 0;
            for segment in app.split('.') {
                count += 1;
                let mut bytes = segment.bytes();
                let valid = bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
                    && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                if !valid {
                    anyhow::bail!("invalid canonical Flatpak application ID: {app:?}");
                }
            }
            if count < 3 {
                anyhow::bail!("invalid canonical Flatpak application ID: {app:?}");
            }
            Ok(())
        }
    }

    pub(crate) mod fonts {
        use anyhow::{bail, Context, Result};
        use std::{collections::BTreeSet, ffi::OsStr, fs, path::Path};
        use url::Url;

        use super::super::{Host, TempDir, TempPath};

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum NerdFontsMode {
            EnsurePresent,
            Update,
        }

        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct NerdFontsOperation {
            families: Vec<String>,
            mode: NerdFontsMode,
        }

        impl NerdFontsOperation {
            pub fn new(families: Vec<String>, mode: NerdFontsMode) -> Result<Self> {
                validate_families(&families)?;
                Ok(Self { families, mode })
            }

            pub(crate) fn display_args(&self) -> Vec<String> {
                [
                    "nerd-fonts".into(),
                    match self.mode {
                        NerdFontsMode::EnsurePresent => "ensure-present".into(),
                        NerdFontsMode::Update => "update".into(),
                    },
                ]
                .into_iter()
                .chain(self.families.iter().cloned())
                .collect()
            }
        }

        pub(crate) fn execute(host: &Host<'_>, operation: &NerdFontsOperation) -> Result<()> {
            validate_families(&operation.families).context("validate Nerd Fonts operation")?;
            for family in &operation.families {
                if operation.mode == NerdFontsMode::Update || !font_present(host, family)? {
                    install_family(host, family)?;
                }
            }
            Ok(())
        }

        fn install_family(host: &Host<'_>, family: &str) -> Result<()> {
            let data_home = host
                .value("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| host.home().join(".local/share"));
            if !data_home.is_absolute() {
                bail!("Nerd Fonts XDG data directory must be absolute");
            }
            let parent = data_home.join("fonts/cozydot");
            fs::create_dir_all(&parent).context("create Nerd Fonts destination directory")?;
            let destination = parent.join(family);
            let archive = TempPath::new_with_suffix(host, "nerd-font", ".tar.xz")?;
            let mut url =
                Url::parse("https://github.com/ryanoasis/nerd-fonts/releases/latest/download/placeholder.tar.xz")?;
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Nerd Fonts URL cannot be a base"))?
                .pop()
                .push(&format!("{family}.tar.xz"));
            host.require(
                "Nerd Font archive download",
                "curl",
                [
                    "--proto".as_ref(),
                    "=https".as_ref(),
                    "--location".as_ref(),
                    "--fail".as_ref(),
                    "--silent".as_ref(),
                    "--show-error".as_ref(),
                    "--retry".as_ref(),
                    "3".as_ref(),
                    "--retry-all-errors".as_ref(),
                    "--output".as_ref(),
                    archive.path().as_os_str(),
                    "--".as_ref(),
                    url.as_str().as_ref(),
                ],
            )?;
            let listing = host.require(
                "Nerd Font archive preflight",
                "tar",
                ["--list", "--xz", "--file", &archive.path().to_string_lossy()],
            )?;
            validate_archive_listing(&listing.stdout)?;
            let stage = TempDir::new_in(&data_home, ".cozydot-font-stage")?;
            host.require(
                "Nerd Font archive extraction",
                "tar",
                [
                    "--extract",
                    "--xz",
                    "--directory",
                    &stage.path().to_string_lossy(),
                    "--file",
                    &archive.path().to_string_lossy(),
                ],
            )?;
            validate_extracted_tree(stage.path())?;
            let replacing = match fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.file_type().is_dir() => true,
                Ok(_) => bail!("Nerd Font destination conflict at {}", destination.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error).context("inspect Nerd Font destination"),
            };
            publish_family(stage.path(), &destination, replacing)?;
            let postcondition = refresh_and_verify(host, family, stage.path(), &destination);
            if let Err(error) = postcondition {
                rollback_family(stage.path(), &destination, replacing)
                    .with_context(|| format!("Nerd Font mutation failed and rollback failed: {error:#}"))?;
                refresh_cache(host, "Nerd Font rollback cache refresh", &parent)
                    .with_context(|| format!("Nerd Font mutation failed: {error:#}"))?;
                return Err(error);
            }
            Ok(())
        }

        fn publish_family(stage: &Path, destination: &Path, replacing: bool) -> Result<()> {
            let flags = if replacing {
                rustix::fs::RenameFlags::EXCHANGE
            } else {
                rustix::fs::RenameFlags::NOREPLACE
            };
            rustix::fs::renameat_with(rustix::fs::CWD, stage, rustix::fs::CWD, destination, flags)
                .context("atomically publish Nerd Font family")
        }

        fn rollback_family(stage: &Path, destination: &Path, replacing: bool) -> Result<()> {
            let flags = if replacing {
                rustix::fs::RenameFlags::EXCHANGE
            } else {
                rustix::fs::RenameFlags::NOREPLACE
            };
            rustix::fs::renameat_with(rustix::fs::CWD, destination, rustix::fs::CWD, stage, flags)
                .context("atomically restore previous Nerd Font family")?;
            sync_publication_directories(stage, destination)
        }

        fn refresh_and_verify(host: &Host<'_>, family: &str, stage: &Path, destination: &Path) -> Result<()> {
            sync_publication_directories(stage, destination)?;
            refresh_cache(
                host,
                "Nerd Font cache refresh",
                destination.parent().context("Nerd Font destination has no parent")?,
            )?;
            if !font_present(host, family)? {
                bail!("Nerd Font mutation did not publish family {family:?}");
            }
            Ok(())
        }

        fn refresh_cache(host: &Host<'_>, operation: &str, directory: &Path) -> Result<()> {
            host.require(operation, "fc-cache", [OsStr::new("--force"), directory.as_os_str()])?;
            Ok(())
        }

        fn sync_publication_directories(stage: &Path, destination: &Path) -> Result<()> {
            let stage_parent = stage.parent().context("Nerd Font stage has no parent")?;
            let destination_parent = destination.parent().context("Nerd Font destination has no parent")?;
            fs::File::open(stage_parent)?
                .sync_all()
                .context("sync Nerd Font staging directory")?;
            fs::File::open(destination_parent)?
                .sync_all()
                .context("sync Nerd Font destination directory")
        }

        fn font_present(host: &Host<'_>, family: &str) -> Result<bool> {
            let expected = format!("{family} Nerd Font");
            let pattern = format!(":family={expected}");
            let output = host.require(
                "Nerd Font state query",
                "fc-list",
                ["--format=%{family}\\n", "--", &pattern],
            )?;
            let output = std::str::from_utf8(&output.stdout).context("fc-list returned non-UTF-8 font state")?;
            if output.chars().any(|character| character == '\r' || character == '\0') {
                bail!("fc-list returned malformed font state");
            }
            Ok(output
                .lines()
                .flat_map(|line| line.split(','))
                .any(|installed| installed == expected))
        }

        fn validate_archive_listing(output: &[u8]) -> Result<()> {
            let output = std::str::from_utf8(output).context("Nerd Font archive listing is not UTF-8")?;
            if output.is_empty() {
                bail!("Nerd Font archive is empty");
            }
            for entry in output.lines() {
                let path = Path::new(entry);
                if entry.is_empty()
                    || path.is_absolute()
                    || path.components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir
                                | std::path::Component::RootDir
                                | std::path::Component::Prefix(_)
                        )
                    })
                    || entry.chars().any(char::is_control)
                {
                    bail!("Nerd Font archive contains an unsafe path");
                }
            }
            Ok(())
        }

        fn validate_extracted_tree(root: &Path) -> Result<()> {
            let mut directories = vec![root.to_path_buf()];
            let mut fonts = 0_u32;
            while let Some(directory) = directories.pop() {
                for entry in fs::read_dir(directory)? {
                    let entry = entry?;
                    let path = entry.path();
                    let metadata = fs::symlink_metadata(&path)?;
                    if metadata.file_type().is_dir() {
                        directories.push(path);
                    } else if metadata.file_type().is_file() {
                        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
                        if matches!(extension, "ttf" | "otf") && metadata.len() > 0 {
                            fonts += 1;
                        }
                    } else {
                        bail!(
                            "Nerd Font archive contains an unsupported file type at {}",
                            path.display()
                        );
                    }
                }
            }
            if fonts == 0 {
                bail!("Nerd Font archive contains no non-empty TTF or OTF files");
            }
            Ok(())
        }

        fn validate_families(families: &[String]) -> Result<()> {
            if families.is_empty() {
                bail!("Nerd Font family sequence must not be empty");
            }
            let mut seen = BTreeSet::new();
            for family in families {
                let bytes = family.as_bytes();
                if bytes.first().is_none_or(|byte| !byte.is_ascii_alphanumeric())
                    || bytes.last().is_none_or(|byte| !byte.is_ascii_alphanumeric())
                    || !bytes
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(byte))
                {
                    bail!("invalid Nerd Font family name {family:?}");
                }
                if !seen.insert(family) {
                    bail!("duplicate Nerd Font family {family:?}");
                }
            }
            Ok(())
        }
    }

    pub(crate) mod dotfiles {
        use anyhow::{bail, Context, Result};
        use std::{
            collections::BTreeSet,
            fs,
            path::{Component, Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        use super::super::Host;

        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct DotfilesOperation {
            root: PathBuf,
            packages: Vec<String>,
        }

        impl DotfilesOperation {
            pub fn new(root: PathBuf, packages: Vec<String>) -> Result<Self> {
                validate_packages(&packages)?;
                if root.as_os_str().is_empty() {
                    bail!("dotfiles root must not be empty");
                }
                Ok(Self { root, packages })
            }

            pub(crate) fn display_args(&self) -> Vec<String> {
                std::iter::once("dotfiles-backup-stow".into())
                    .chain(self.packages.iter().cloned())
                    .collect()
            }
        }

        pub(crate) fn execute(host: &Host<'_>, operation: &DotfilesOperation) -> Result<()> {
            validate_packages(&operation.packages).context("validate dotfiles operation")?;
            let root = fs::canonicalize(&operation.root)
                .with_context(|| format!("dotfiles operation: canonicalize root {}", operation.root.display()))?;
            if !fs::symlink_metadata(&root)?.file_type().is_dir() {
                bail!("dotfiles root is not a directory: {}", root.display());
            }
            for package in &operation.packages {
                apply_package(host, &root, package)?;
            }
            Ok(())
        }

        fn apply_package(host: &Host<'_>, root: &Path, package: &str) -> Result<()> {
            let source = root.join(package);
            let metadata = fs::symlink_metadata(&source)
                .with_context(|| format!("dotfiles package {package:?} does not exist"))?;
            if !metadata.file_type().is_dir() {
                bail!("dotfiles package {package:?} is not a real directory");
            }
            let mut conflicts = Vec::new();
            collect_conflicts(&source, host.home(), &mut conflicts)?;
            if !conflicts.is_empty() {
                backup_conflicts(host, package, &conflicts)?;
            }
            host.require(
                "dotfiles Stow mutation",
                "stow",
                [
                    "--dir".as_ref(),
                    root.as_os_str(),
                    "--target".as_ref(),
                    host.home().as_os_str(),
                    "--stow".as_ref(),
                    "--".as_ref(),
                    package.as_ref(),
                ],
            )?;
            verify_tree(&source, host.home()).with_context(|| format!("dotfiles package {package:?} postcondition"))
        }

        fn collect_conflicts(source: &Path, target: PathBuf, conflicts: &mut Vec<PathBuf>) -> Result<()> {
            let source_metadata = fs::symlink_metadata(source)
                .with_context(|| format!("inspect dotfiles source {}", source.display()))?;
            if source_metadata.file_type().is_dir() {
                match fs::symlink_metadata(&target) {
                    Ok(metadata) if metadata.file_type().is_dir() => {
                        let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
                        entries.sort_by_key(|entry| entry.file_name());
                        for entry in entries {
                            collect_conflicts(&entry.path(), target.join(entry.file_name()), conflicts)?;
                        }
                    }
                    Ok(_) if !resolves_to(&target, source) => conflicts.push(target),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("inspect dotfiles target"),
                }
            } else if source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
                match fs::symlink_metadata(&target) {
                    Ok(_) if !resolves_to(&target, source) => conflicts.push(target),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("inspect dotfiles target"),
                }
            } else {
                bail!("unsupported dotfiles source type at {}", source.display());
            }
            Ok(())
        }

        fn backup_conflicts(host: &Host<'_>, package: &str, conflicts: &[PathBuf]) -> Result<()> {
            let state_home = host
                .value("XDG_STATE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| host.home().join(".local/state"));
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("dotfiles backup timestamp is before the Unix epoch")?
                .as_nanos();
            let backup_root = state_home
                .join("cozydot/dotfile-backups")
                .join(format!("{timestamp}-{}", std::process::id()))
                .join(package);
            for conflict in conflicts {
                let relative = conflict
                    .strip_prefix(host.home())
                    .with_context(|| format!("dotfiles conflict escaped the home directory: {}", conflict.display()))?;
                let backup = backup_root.join(relative);
                let parent = backup.parent().context("dotfiles backup has no parent")?;
                fs::create_dir_all(parent).context("create dotfiles backup directory")?;
                host.require(
                    "dotfiles conflict backup",
                    "mv",
                    [
                        "--no-clobber".as_ref(),
                        "--".as_ref(),
                        conflict.as_os_str(),
                        backup.as_os_str(),
                    ],
                )?;
                if fs::symlink_metadata(conflict).is_ok() || fs::symlink_metadata(&backup).is_err() {
                    bail!(
                        "dotfiles conflict backup did not move {} to {}",
                        conflict.display(),
                        backup.display()
                    );
                }
            }
            Ok(())
        }

        fn verify_tree(source: &Path, target: PathBuf) -> Result<()> {
            let source_metadata = fs::symlink_metadata(source)?;
            if source_metadata.file_type().is_dir() {
                let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
                entries.sort_by_key(|entry| entry.file_name());
                for entry in entries {
                    verify_tree(&entry.path(), target.join(entry.file_name()))?;
                }
            } else if !resolves_to(&target, source) {
                bail!("Stow did not link {} to {}", target.display(), source.display());
            }
            Ok(())
        }

        fn resolves_to(target: &Path, source: &Path) -> bool {
            fs::canonicalize(target)
                .and_then(|target| fs::canonicalize(source).map(|source| target == source))
                .unwrap_or(false)
        }

        fn validate_packages(packages: &[String]) -> Result<()> {
            if packages.is_empty() {
                bail!("dotfiles package sequence must not be empty");
            }
            let mut seen = BTreeSet::new();
            for package in packages {
                let mut components = Path::new(package).components();
                if !matches!(components.next(), Some(Component::Normal(_)))
                    || components.next().is_some()
                    || package.contains(['\n', '\r'])
                {
                    bail!("invalid dotfiles package directory name {package:?}");
                }
                if !seen.insert(package) {
                    bail!("duplicate dotfiles package {package:?}");
                }
            }
            Ok(())
        }
    }
}

pub(super) fn latest_go(input: &str, requested: &str, arch: &str) -> anyhow::Result<(String, String, String)> {
    use anyhow::Context;
    let value: serde_json::Value = serde_json::from_str(input).context("parse Go release JSON")?;
    let releases = value.as_array().context("Go metadata must be an array")?;
    let version = releases
        .iter()
        .filter_map(|release| release["version"].as_str())
        .filter(|v| stable_go_version(v))
        .map(|v| v.trim_start_matches("go"))
        .find(|v| {
            requested == "latest"
                || *v == requested
                || v.strip_prefix(requested).is_some_and(|rest| rest.starts_with('.'))
        })
        .context("Go metadata has no matching stable release")?;
    let filename = format!("go{version}.linux-{arch}.tar.gz");
    let checksum = releases
        .iter()
        .find(|release| release["version"].as_str() == Some(&format!("go{version}")))
        .and_then(|release| release["files"].as_array())
        .and_then(|files| files.iter().find(|file| file["filename"].as_str() == Some(&filename)))
        .and_then(|file| file["sha256"].as_str())
        .context("Go metadata has no matching archive checksum")?;
    Ok((version.to_owned(), filename, checksum.to_owned()))
}

pub(super) fn gnome_version(input: &str, shell_version: &str) -> anyhow::Result<u64> {
    use anyhow::{bail, Context};
    let value: serde_json::Value = serde_json::from_str(input).context("parse GNOME extension JSON")?;
    let versions = value["shell_version_map"]
        .as_object()
        .context("GNOME response has no shell_version_map")?;
    let mut candidate = shell_version;
    loop {
        if let Some(version) = versions.get(candidate).and_then(|entry| entry["version"].as_u64()) {
            return Ok(version);
        }
        let Some((parent, _)) = candidate.rsplit_once('.') else {
            bail!("GNOME response has no extension version for shell {shell_version}");
        };
        candidate = parent;
    }
}

pub(super) fn gnome_shell_version(input: &str) -> anyhow::Result<String> {
    use anyhow::Context;
    input
        .split_whitespace()
        .map(|part| part.trim_matches(|character: char| !character.is_ascii_digit() && character != '.'))
        .find(|part| {
            !part.is_empty()
                && part
                    .split('.')
                    .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .map(str::to_owned)
        .context("GNOME Shell version output has no numeric version")
}

fn stable_go_version(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("go") else {
        return false;
    };
    let parts = rest.split('.').collect::<Vec<_>>();
    (parts.len() == 2 || parts.len() == 3)
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}
