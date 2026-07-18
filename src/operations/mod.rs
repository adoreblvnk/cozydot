mod apt;
mod binary;
mod cargo;
mod cargo_binstall;
mod desktop;
mod dotfiles;
mod flatpak;
mod fonts;
mod integrations;
mod languages;
mod managed_apt;
mod managed_state;
mod npm;
mod privileged_file;
mod provisioning;
mod repository;
mod system;
mod tools;

#[cfg(test)]
mod behavior_tests;

pub use apt::AptUpgradePolicy;
pub use binary::{
    BinaryPackageFormat, BinaryPackageMode, BinaryPackageOperation, BinaryPackageSelector,
    BinarySha256, BinarySourceOperation, GithubRepository,
};
pub use cargo::{CargoPackageMode, CargoPackageOperation};
pub use cargo_binstall::CargoBinstallBootstrapOperation;
pub use desktop::{
    DesktopEnvironment, DesktopSetting, DesktopSettingOperation, DesktopTheme, GnomeDockOperation,
    GnomeExtensionsOperation, GnomeRoundedCornersOperation,
};
pub use dotfiles::DotfilesOperation;
pub use fonts::{NerdFontsMode, NerdFontsOperation};
pub use integrations::{DockerLocalLogOperation, VsCodeExtensionOperation};
pub use managed_apt::ManagedAptSourcesOperation;
pub use npm::{NpmPackageMode, NpmPackageOperation};
pub use repository::{
    AptRepositoryOperation, AptRepositoryPath, AptRepositorySourceLayout, AptRepositoryToken,
};
pub use system::{EnsureAdminOperation, UbuntuSnapOperation, UnattendedUpgradesOperation};
pub use tools::{
    GoToolchainOperation, GoToolchainSelector, NodeToolchainOperation, NodeToolchainSelector,
    PythonToolchainOperation, RustToolchainOperation, RustToolchainSelector, ToolMutationMode,
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
            Self::AptBootstrapPackages { packages } => {
                std::iter::once("apt-bootstrap-packages".into())
                    .chain(packages.clone())
                    .collect()
            }
            Self::AptMetadataRefresh => vec!["apt-metadata-refresh".into()],
            Self::AptRepository(operation) => operation.display_args(),
            Self::ManagedAptSources(operation) => operation.display_args(),
            Self::AptPackages { packages } => std::iter::once("apt-packages".into())
                .chain(packages.clone())
                .collect(),
            Self::AptPurge { packages } => std::iter::once("apt-purge".into())
                .chain(packages.clone())
                .collect(),
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

pub(crate) fn execute(
    operation: &Operation,
    env: &[(OsString, OsString)],
) -> Result<OperationOutcome> {
    execute_on_host(operation, Host::new(env, Path::new(DOCKER_LOCK))?)
}

#[cfg(test)]
pub(crate) fn execute_with_docker_lock_for_test(
    operation: &Operation,
    env: &[(OsString, OsString)],
    docker_lock_open_path: &Path,
) -> Result<()> {
    execute_on_host(operation, Host::new(env, docker_lock_open_path)?).map(|_| ())
}

#[cfg(test)]
pub(crate) fn execute_with_outcome_for_test(
    operation: &Operation,
    env: &[(OsString, OsString)],
    docker_lock_open_path: &Path,
) -> Result<OperationOutcome> {
    execute_on_host(operation, Host::new(env, docker_lock_open_path)?)
}

fn execute_on_host(operation: &Operation, host: Host<'_>) -> Result<OperationOutcome> {
    match operation {
        Operation::AptBootstrapPackages { packages } => {
            completed(apt::bootstrap_packages(&host, packages))
        }
        Operation::AptMetadataRefresh => completed(apt::metadata_refresh(&host)),
        Operation::AptRepository(operation) => completed(repository::execute(&host, operation)),
        Operation::ManagedAptSources(operation) => {
            completed(managed_apt::execute(&host, operation))
        }
        Operation::AptPackages { packages } => completed(apt::packages(&host, packages)),
        Operation::AptPurge { packages } => completed(apt::purge(&host, packages)),
        Operation::AptUpgrade { policy } => completed(apt::upgrade(&host, *policy)),
        Operation::DockerGroup => completed(integrations::docker_group(&host)),
        Operation::DockerLocalLog(operation) => {
            completed(integrations::docker_local_log(&host, operation))
        }
        Operation::DesktopSetting(operation) => {
            completed(desktop::desktop_setting(&host, operation))
        }
        Operation::BinaryPackage(package) => completed(binary::execute(&host, package)),
        Operation::Dotfiles(operation) => completed(dotfiles::execute(&host, operation)),
        Operation::FlatpakEnsureFlathub => completed(flatpak::ensure_flathub(&host)),
        Operation::FlatpakEnsureApps { refs } => completed(flatpak::ensure_apps(&host, refs)),
        Operation::FlatpakUpdateApps { refs } => completed(flatpak::update_apps(&host, refs)),
        Operation::FnmBootstrap => completed(languages::fnm_bootstrap(&host)),
        Operation::EnsureAdmin(operation) => completed(system::ensure_admin(&host, operation)),
        Operation::GnomeExtensions(operation) => desktop::gnome_extensions(&host, operation),
        Operation::GnomeDock(operation) => desktop::gnome_dock(&host, operation),
        Operation::GnomeRoundedCorners(operation) => {
            desktop::gnome_rounded_corners(&host, operation)
        }
        Operation::GoToolchain(operation) => completed(tools::execute_go(&host, operation)),
        Operation::NerdFonts(operation) => completed(fonts::execute(&host, operation)),
        Operation::RustupBootstrap => completed(provisioning::rustup(&host)),
        Operation::CargoBinstallBootstrap(operation) => {
            completed(cargo_binstall::execute(&host, operation))
        }
        Operation::RustToolchain(operation) => completed(tools::execute_rust(&host, operation)),
        Operation::CargoPackageSet(operation) => completed(cargo::execute(&host, operation)),
        Operation::NodeToolchain(operation) => completed(tools::execute_node(&host, operation)),
        Operation::NpmPackageSet(operation) => completed(npm::execute(&host, operation)),
        Operation::UbuntuSnap(operation) => completed(system::ubuntu_snap(&host, operation)),
        Operation::UnattendedUpgrades(operation) => {
            completed(system::unattended_upgrades(&host, operation))
        }
        Operation::UvBootstrap => completed(languages::uv_bootstrap(&host)),
        Operation::PythonToolchain(operation) => completed(tools::execute_python(&host, operation)),
        Operation::VirtualBoxGroup => completed(integrations::virtualbox_group(&host)),
        Operation::VsCodeExtensionSet(operation) => {
            completed(integrations::vscode_extensions(&host, operation))
        }
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

    pub fn require_input<I, S>(
        &self,
        operation: &str,
        program: &str,
        args: I,
        input: &[u8],
    ) -> Result<()>
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
        let mode = u32::from_str_radix(mode, 16)
            .context("Docker transaction lock stat returned malformed mode output")?;
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
        let mode = fields
            .next()
            .and_then(|value| u32::from_str_radix(value, 16).ok());
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

#[cfg(test)]
mod tests {
    use super::{resolve_home, retry_executable_busy, EXECUTABLE_FILE_BUSY};
    use std::{cell::Cell, ffi::OsString, io, path::PathBuf};

    #[test]
    fn operations_require_home_without_a_literal_fallback() {
        assert_eq!(
            resolve_home(&[], Some(OsString::from("/home/process"))).unwrap(),
            PathBuf::from("/home/process")
        );
        assert!(resolve_home(&[], None).is_err());
    }

    #[test]
    fn command_start_retries_only_executable_busy_errors() {
        let attempts = Cell::new(0);
        let value = retry_executable_busy(|| {
            attempts.set(attempts.get() + 1);
            if attempts.get() == 1 {
                Err(io::Error::from_raw_os_error(EXECUTABLE_FILE_BUSY))
            } else {
                Ok(7)
            }
        })
        .unwrap();
        assert_eq!(value, 7);
        assert_eq!(attempts.get(), 2);

        let attempts = Cell::new(0);
        let error = retry_executable_busy(|| -> io::Result<()> {
            attempts.set(attempts.get() + 1);
            Err(io::Error::from_raw_os_error(13))
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(13));
        assert_eq!(attempts.get(), 1);
    }
}
