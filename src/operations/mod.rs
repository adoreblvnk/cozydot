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
pub use binary::{
    BinaryPackageFormat, BinaryPackageMode, BinaryPackageOperation, BinaryPackageSelector, BinarySha256,
    BinarySourceOperation, GithubRepository,
};
pub use packages::cargo::CargoPackageMode;
pub use packages::fonts::NerdFontsMode;
pub use packages::npm::NpmPackageMode;
pub use repository::{AptRepositoryOperation, AptRepositoryPath, AptRepositorySourceLayout, AptRepositoryToken};
pub use system::{DesktopEnvironment, DesktopSetting, DesktopTheme};
pub use tools::{GoToolchainSelector, NodeToolchainSelector, RustToolchainSelector, ToolMutationMode};

use crate::platform::{Architecture, ManagedAptSources};
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
const RUSTUP_BOOTSTRAP_FLAGS: [&str; 3] = ["-y", "--default-toolchain", "none"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    AptBootstrapPackages {
        packages: Vec<String>,
    },
    AptMetadataRefresh,
    AptRepository(AptRepositoryOperation),
    ManagedAptSources(ManagedAptSources),
    AptPackages {
        packages: Vec<String>,
    },
    AptPurge {
        packages: Vec<String>,
    },
    AptUpgrade {
        policy: AptUpgradePolicy,
    },
    DockerGroup,
    DockerLocalLog {
        max_size: Option<String>,
    },
    DesktopSetting {
        target: DesktopEnvironment,
        setting: DesktopSetting,
    },
    BinaryPackage(BinaryPackageOperation),
    Dotfiles {
        root: PathBuf,
        packages: Vec<String>,
    },
    FlatpakEnsureFlathub,
    FlatpakEnsureApps {
        refs: Vec<String>,
    },
    FlatpakUpdateApps {
        refs: Vec<String>,
    },
    FnmBootstrap,
    EnsureAdmin,
    GnomeExtensions {
        extensions: Vec<String>,
    },
    GnomeDock,
    GnomeRoundedCorners,
    GoToolchain {
        selector: GoToolchainSelector,
        architecture: Architecture,
        mode: ToolMutationMode,
    },
    NerdFonts {
        families: Vec<String>,
        mode: NerdFontsMode,
    },
    RustupBootstrap,
    CargoBinstallBootstrap {
        architecture: Architecture,
    },
    RustToolchain {
        selector: RustToolchainSelector,
        architecture: Architecture,
        mode: ToolMutationMode,
    },
    CargoPackageSet {
        packages: Vec<String>,
        mode: CargoPackageMode,
    },
    NodeToolchain {
        selector: NodeToolchainSelector,
        architecture: Architecture,
        mode: ToolMutationMode,
    },
    NpmPackageSet {
        packages: Vec<String>,
        mode: NpmPackageMode,
    },
    UbuntuSnap {
        enabled: bool,
    },
    UnattendedUpgrades {
        enabled: bool,
    },
    UvBootstrap,
    PythonToolchain {
        version: String,
        architecture: Architecture,
    },
    VirtualBoxGroup,
    VsCodeExtensionSet {
        extensions: Vec<String>,
    },
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
            Self::ManagedAptSources(policy) => vec![
                "managed-apt-sources".into(),
                policy.distro.clone(),
                policy.release.clone(),
                policy.architecture.canonical().into(),
            ],
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
            Self::DockerLocalLog { max_size } => std::iter::once("docker-local-log".into())
                .chain(max_size.iter().cloned())
                .collect(),
            Self::DesktopSetting { target, setting } => {
                let target_str = match target {
                    DesktopEnvironment::Gnome => "gnome",
                    DesktopEnvironment::Cinnamon => "cinnamon",
                };
                let (name, value) = match setting {
                    DesktopSetting::Theme(DesktopTheme::Light) => ("theme", "light".into()),
                    DesktopSetting::Theme(DesktopTheme::Dark) => ("theme", "dark".into()),
                    DesktopSetting::Terminal(executable) => ("terminal", executable.clone()),
                    DesktopSetting::IdleTimeoutSeconds(seconds) => ("idle-timeout-seconds", seconds.to_string()),
                    DesktopSetting::IdleDim(enabled) => ("idle-dim", enabled.to_string()),
                };
                vec!["desktop-setting".into(), target_str.into(), name.into(), value]
            }
            Self::BinaryPackage(package) => package.display_args(),
            Self::Dotfiles { packages, .. } => std::iter::once("dotfiles-backup-stow".into())
                .chain(packages.iter().cloned())
                .collect(),
            Self::FlatpakEnsureFlathub => vec!["flatpak-ensure-flathub".into()],
            Self::FlatpakEnsureApps { refs } => std::iter::once("flatpak-ensure-apps".into())
                .chain(refs.clone())
                .collect(),
            Self::FlatpakUpdateApps { refs } => std::iter::once("flatpak-update-apps".into())
                .chain(refs.clone())
                .collect(),
            Self::FnmBootstrap => vec!["fnm-bootstrap".into()],
            Self::EnsureAdmin => vec!["ensure-admin".into()],
            Self::GnomeExtensions { extensions } => std::iter::once("gnome-extensions".into())
                .chain(extensions.iter().cloned())
                .collect(),
            Self::GnomeDock => vec!["gnome-dock".into()],
            Self::GnomeRoundedCorners => vec!["gnome-rounded-corners".into()],
            Self::GoToolchain {
                selector,
                architecture,
                mode,
            } => vec![
                "go-toolchain".into(),
                match mode {
                    ToolMutationMode::EnsurePresent => "ensure-present",
                    ToolMutationMode::UpdateMoving => "update-moving",
                }
                .into(),
                match selector {
                    GoToolchainSelector::Latest => "latest",
                    GoToolchainSelector::Version(v) => v,
                }
                .into(),
                architecture.go_archive().into(),
            ],
            Self::NerdFonts { families, mode } => [
                "nerd-fonts".into(),
                match mode {
                    NerdFontsMode::EnsurePresent => "ensure-present".into(),
                    NerdFontsMode::Update => "update".into(),
                },
            ]
            .into_iter()
            .chain(families.iter().cloned())
            .collect(),
            Self::RustupBootstrap => vec!["rustup-bootstrap".into()],
            Self::CargoBinstallBootstrap { architecture } => {
                vec!["cargo-binstall-bootstrap".into(), architecture.canonical().into()]
            }
            Self::RustToolchain {
                selector,
                architecture,
                mode,
            } => vec![
                "rust-toolchain".into(),
                match mode {
                    ToolMutationMode::EnsurePresent => "ensure-present",
                    ToolMutationMode::UpdateMoving => "update-moving",
                }
                .into(),
                match selector {
                    RustToolchainSelector::Stable => "stable",
                    RustToolchainSelector::Version(v) => v,
                }
                .into(),
                architecture.rust_target().into(),
            ],
            Self::CargoPackageSet { packages, mode } => std::iter::once("cargo-package-set".into())
                .chain(std::iter::once(
                    match mode {
                        CargoPackageMode::EnsurePresent => "ensure-present",
                        CargoPackageMode::UpdateCurrent => "update-current",
                    }
                    .into(),
                ))
                .chain(packages.iter().cloned())
                .collect(),
            Self::NodeToolchain {
                selector,
                architecture,
                mode,
            } => vec![
                "node-toolchain".into(),
                match mode {
                    ToolMutationMode::EnsurePresent => "ensure-present",
                    ToolMutationMode::UpdateMoving => "update-moving",
                }
                .into(),
                match selector {
                    NodeToolchainSelector::Lts => "lts",
                    NodeToolchainSelector::Latest => "latest",
                    NodeToolchainSelector::Version(v) => v,
                }
                .into(),
                architecture.canonical().into(),
            ],
            Self::NpmPackageSet { packages, mode } => std::iter::once("npm-package-set".into())
                .chain(std::iter::once(
                    match mode {
                        NpmPackageMode::EnsurePresent => "ensure-present",
                        NpmPackageMode::UpdateCurrent => "update-current",
                    }
                    .into(),
                ))
                .chain(packages.iter().cloned())
                .collect(),
            Self::UbuntuSnap { enabled } => vec!["ubuntu-snap".into(), enabled.to_string()],
            Self::UnattendedUpgrades { enabled } => vec!["unattended-upgrades".into(), enabled.to_string()],
            Self::UvBootstrap => vec!["uv-bootstrap".into()],
            Self::PythonToolchain { version, architecture } => vec![
                "python-toolchain".into(),
                version.clone(),
                architecture.canonical().into(),
            ],
            Self::VirtualBoxGroup => vec!["virtualbox-group".into()],
            Self::VsCodeExtensionSet { extensions } => std::iter::once("vscode-extension-set".into())
                .chain(extensions.iter().cloned())
                .collect(),
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
        Operation::ManagedAptSources(policy) => completed(repository::managed_apt::execute(&host, policy)),
        Operation::AptPackages { packages } => completed(apt::packages(&host, packages)),
        Operation::AptPurge { packages } => completed(apt::purge(&host, packages)),
        Operation::AptUpgrade { policy } => completed(apt::upgrade(&host, *policy)),
        Operation::DockerGroup => completed(system::docker_group(&host)),
        Operation::DockerLocalLog { max_size } => completed(system::docker_local_log(&host, max_size.as_deref())),
        Operation::DesktopSetting { target, setting } => completed(system::desktop_setting(&host, *target, setting)),
        Operation::BinaryPackage(package) => completed(binary::execute(&host, package)),
        Operation::Dotfiles { root, packages } => completed(packages::dotfiles::execute(&host, root, packages)),
        Operation::FlatpakEnsureFlathub => completed(packages::flatpak::ensure_flathub(&host)),
        Operation::FlatpakEnsureApps { refs } => completed(packages::flatpak::ensure_apps(&host, refs)),
        Operation::FlatpakUpdateApps { refs } => completed(packages::flatpak::update_apps(&host, refs)),
        Operation::FnmBootstrap => completed(languages::fnm_bootstrap(&host)),
        Operation::EnsureAdmin => completed(system::ensure_admin(&host)),
        Operation::GnomeExtensions { extensions } => system::gnome_extensions(&host, extensions),
        Operation::GnomeDock => system::gnome_dock(&host),
        Operation::GnomeRoundedCorners => system::gnome_rounded_corners(&host),
        Operation::GoToolchain {
            selector,
            architecture,
            mode,
        } => completed(tools::execute_go(&host, selector, *architecture, *mode)),
        Operation::NerdFonts { families, mode } => completed(packages::fonts::execute(&host, families, *mode)),
        Operation::RustupBootstrap => completed(languages::rustup(&host)),
        Operation::CargoBinstallBootstrap { architecture } => {
            completed(binary::cargo_binstall::execute(&host, *architecture))
        }
        Operation::RustToolchain {
            selector,
            architecture,
            mode,
        } => completed(tools::execute_rust(&host, selector, *architecture, *mode)),
        Operation::CargoPackageSet { packages, mode } => completed(packages::cargo::execute(&host, packages, *mode)),
        Operation::NodeToolchain {
            selector,
            architecture,
            mode,
        } => completed(tools::execute_node(&host, selector, *architecture, *mode)),
        Operation::NpmPackageSet { packages, mode } => completed(packages::npm::execute(&host, packages, *mode)),
        Operation::UbuntuSnap { enabled } => completed(system::ubuntu_snap(&host, *enabled)),
        Operation::UnattendedUpgrades { enabled } => completed(system::unattended_upgrades(&host, *enabled)),
        Operation::UvBootstrap => completed(languages::uv_bootstrap(&host)),
        Operation::PythonToolchain { version, architecture } => {
            completed(tools::execute_python(&host, version, *architecture))
        }
        Operation::VirtualBoxGroup => completed(system::virtualbox_group(&host)),
        Operation::VsCodeExtensionSet { extensions } => completed(system::vscode_extensions(&host, extensions)),
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
    use anyhow::Result;

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
        host.require("APT bootstrap metadata refresh", "sudo", ["apt-get", "update", "-qq"])?;
        install(host, "APT bootstrap package installation", packages.to_vec())
    }

    pub fn packages(host: &Host<'_>, packages: &[String]) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }
        install(host, "APT package installation", packages.to_vec())
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
        if packages.is_empty() {
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
        args.extend(packages.iter().cloned());
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
}

pub(crate) mod languages {
    use anyhow::{bail, Context, Result};
    use std::{ffi::OsStr, os::unix::fs::PermissionsExt, path::PathBuf};

    use crate::operations::{Host, TempPath, RUSTUP_BOOTSTRAP_FLAGS};

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
        host.require(
            "rustup bootstrap",
            "sh",
            std::iter::once(installer.path().as_os_str()).chain(RUSTUP_BOOTSTRAP_FLAGS.map(OsStr::new)),
        )?;
        if !executable_file(&cargo_home.join("bin/rustup")) {
            bail!("rustup bootstrap did not publish the managed rustup executable");
        }
        Ok(())
    }
}

pub(crate) mod packages {

    pub(crate) mod cargo {
        use anyhow::{bail, Context, Result};
        use std::{
            os::unix::fs::PermissionsExt,
            path::{Path, PathBuf},
        };

        use super::super::Host;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum CargoPackageMode {
            EnsurePresent,
            UpdateCurrent,
        }

        pub(crate) fn execute(host: &Host<'_>, packages: &[String], mode: CargoPackageMode) -> Result<()> {
            let cargo_home = host
                .value("CARGO_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| host.home().join(".cargo"));
            if !cargo_home.is_absolute() {
                bail!("Cargo package operation requires an absolute CARGO_HOME");
            }
            let binstall = resolve_binstall(&cargo_home)?
                .context("Cargo package operation: managed cargo-binstall is unavailable after bootstrap")?;
            let mut args = vec!["--no-confirm".to_owned()];
            if mode == CargoPackageMode::UpdateCurrent {
                args.push("--force".into());
            }
            args.extend(packages.to_vec());
            host.require("Cargo package mutation", &binstall, args)?;
            Ok(())
        }

        fn resolve_binstall(cargo_home: &Path) -> Result<Option<String>> {
            let managed = cargo_home.join("bin/cargo-binstall");
            if executable_file(&managed) {
                return path_program(&managed, "cargo-binstall executable path").map(Some);
            }
            Ok(None)
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
        use std::{
            os::unix::fs::PermissionsExt,
            path::{Path, PathBuf},
        };

        use super::super::Host;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum NpmPackageMode {
            EnsurePresent,
            UpdateCurrent,
        }

        pub(crate) fn execute(host: &Host<'_>, packages: &[String], mode: NpmPackageMode) -> Result<()> {
            let fnm = resolve_fnm(host)?;
            let version = selected_version(host, &fnm)?;

            let command = match mode {
                NpmPackageMode::EnsurePresent => "install",
                NpmPackageMode::UpdateCurrent => "update",
            };
            let mut npm_args = vec![command.to_owned(), "--global".into(), "--".into()];
            npm_args.extend(packages.to_vec());
            run_npm_required(host, &fnm, &version, "npm package mutation", npm_args)?;
            Ok(())
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
        use anyhow::Result;

        const FLATHUB_NAME: &str = "flathub";
        const FLATHUB_DESCRIPTOR_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
        const FLATHUB_URL: &str = "https://dl.flathub.org/repo/";

        pub fn ensure_flathub(host: &Host<'_>) -> Result<()> {
            host.require(
                "Flathub remote ensure",
                "flatpak",
                [
                    "--user",
                    "remote-add",
                    "--if-not-exists",
                    FLATHUB_NAME,
                    FLATHUB_DESCRIPTOR_URL,
                ],
            )?;
            let url_arg = format!("--url={FLATHUB_URL}");
            host.require(
                "Flathub remote security canonicalization",
                "flatpak",
                [
                    "--user",
                    "remote-modify",
                    &url_arg,
                    "--gpg-verify",
                    "--enumerate",
                    "--use-for-deps",
                    "--enable",
                    "--no-filter",
                    FLATHUB_NAME,
                ],
            )?;
            Ok(())
        }

        pub fn ensure_apps(host: &Host<'_>, refs: &[String]) -> Result<()> {
            let mut args = vec![
                "--user".to_owned(),
                "install".into(),
                "--app".into(),
                "--noninteractive".into(),
                "-y".into(),
                "flathub".into(),
                "--".into(),
            ];
            args.extend(refs.iter().cloned());
            host.require("Flatpak application installation", "flatpak", args)?;
            Ok(())
        }

        pub fn update_apps(host: &Host<'_>, refs: &[String]) -> Result<()> {
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
    }

    pub(crate) mod fonts {
        use anyhow::{bail, Context, Result};
        use std::{ffi::OsStr, fs, path::Path};
        use url::Url;

        use super::super::{Host, TempDir, TempPath};

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum NerdFontsMode {
            EnsurePresent,
            Update,
        }

        pub(crate) fn execute(host: &Host<'_>, families: &[String], mode: NerdFontsMode) -> Result<()> {
            let data_home = host
                .value("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| host.home().join(".local/share"));
            if !data_home.is_absolute() {
                bail!("Nerd Fonts XDG data directory must be absolute");
            }
            let parent = data_home.join("fonts/cozydot");
            for family in families {
                let destination = parent.join(family);
                let is_present = match fs::symlink_metadata(&destination) {
                    Ok(metadata) => {
                        if metadata.is_dir() {
                            validate_extracted_tree(&destination)
                                .with_context(|| format!("validate installed Nerd Font family {family:?}"))?;
                            true
                        } else {
                            bail!("Nerd Font destination conflict at {}", destination.display());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                    Err(error) => {
                        return Err(error).context(format!("inspect Nerd Font destination {}", destination.display()))
                    }
                };
                if mode == NerdFontsMode::Update || !is_present {
                    install_family_with_destination(host, family, &destination, &parent, &data_home)?;
                }
            }
            refresh_cache(host, "Nerd Font cache refresh", &parent)?;
            Ok(())
        }

        fn install_family_with_destination(
            host: &Host<'_>,
            family: &str,
            destination: &Path,
            parent: &Path,
            data_home: &Path,
        ) -> Result<()> {
            fs::create_dir_all(parent).context("create Nerd Fonts destination directory")?;
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
            let stage = TempDir::new_in(data_home, ".cozydot-font-stage")?;
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
            let replacing = match fs::symlink_metadata(destination) {
                Ok(metadata) if metadata.is_dir() => true,
                Ok(_) => bail!("Nerd Font destination conflict at {}", destination.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error).context("inspect Nerd Font destination"),
            };
            publish_family(stage.path(), destination, replacing)?;
            sync_publication_directories(stage.path(), destination)?;
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
    }

    pub(crate) mod dotfiles {
        use anyhow::{bail, Context, Result};
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        use super::super::Host;

        pub(crate) fn execute(host: &Host<'_>, root: &Path, packages: &[String]) -> Result<()> {
            let root = fs::canonicalize(root)
                .with_context(|| format!("dotfiles operation: canonicalize root {}", root.display()))?;
            if !fs::symlink_metadata(&root)?.file_type().is_dir() {
                bail!("dotfiles root is not a directory: {}", root.display());
            }
            for package in packages {
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
