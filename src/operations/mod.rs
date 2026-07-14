mod appimaged;
mod apps;
mod apt;
mod cargo;
mod desktop;
mod direct;
mod dotfiles;
mod downloads;
mod flatpak;
mod fonts;
mod integrations;
mod languages;
mod managed_apt;
mod npm;
mod provisioning;
mod repository;
mod snap_cleanup;
mod system;
mod tools;

pub use apt::AptUpgradePolicy;
pub use cargo::{CargoPackageMode, CargoPackageOperation};
pub use desktop::{
    DesktopEnvironment, DesktopSetting, DesktopSettingOperation, DesktopTheme, GnomeDockOperation,
    GnomeExtensionsOperation, GnomeRoundedCornersOperation,
};
pub use direct::{
    DirectPackageFormat, DirectPackageMode, DirectPackageOperation, DirectPackageSelector,
    GithubRepository,
};
pub use dotfiles::DotfilesOperation;
pub use fonts::NerdFontsOperation;
pub use integrations::{DockerLocalLogOperation, VsCodeExtensionOperation};
pub use managed_apt::ManagedAptSourcesOperation;
pub use npm::{NpmPackageMode, NpmPackageOperation};
pub use system::{EnsureAdminOperation, UbuntuSnapOperation, UnattendedUpgradesOperation};
pub use tools::{
    GoToolchainOperation, GoToolchainSelector, NodeToolchainOperation, NodeToolchainSelector,
    PythonToolchainOperation, RustToolchainOperation, RustToolchainSelector, ToolMutationMode,
};

use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::Duration,
};

const ETXTBSY: i32 = 26;
const COZYDOT_RUNTIME_DIRECTORY: &str = "/run/cozydot";
const DOCKER_LOCK: &str = "/run/cozydot/docker-daemon.lock";
const ETXTBSY_BACKOFFS: [Duration; 4] = [
    Duration::from_millis(20),
    Duration::from_millis(40),
    Duration::from_millis(80),
    Duration::from_millis(160),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    AptBootstrapPackages {
        packages: Vec<String>,
    },
    AptMetadataRefresh,
    ManagedAptSources(ManagedAptSourcesOperation),
    AptCodecs {
        package: String,
    },
    AptPackages {
        packages: Vec<String>,
    },
    AptPurge {
        packages: Vec<String>,
    },
    AptUpgrade {
        policy: AptUpgradePolicy,
    },
    AptSource {
        destination: String,
        contents: String,
    },
    Appimaged {
        arch: String,
    },
    DockerConfig {
        user: String,
    },
    DockerGroup,
    DockerLocalLog(DockerLocalLogOperation),
    DesktopSetting(DesktopSettingOperation),
    DirectPackage(DirectPackageOperation),
    Dotfiles(DotfilesOperation),
    DownloadBinary {
        name: String,
        url: String,
        repo: String,
        pattern: String,
    },
    FlatpakEnsureFlathub,
    FlatpakEnsureApps {
        refs: Vec<String>,
    },
    FlatpakUpdateApps {
        refs: Vec<String>,
    },
    FnmBootstrap,
    EnsureAdmin(EnsureAdminOperation),
    GnomeExtension {
        extension: String,
    },
    GnomeExtensions(GnomeExtensionsOperation),
    GnomeDependencies,
    GnomeDockSettings,
    GnomeDock(GnomeDockOperation),
    GnomeRoundedCornersSettings,
    GnomeRoundedCorners(GnomeRoundedCornersOperation),
    GnomeTerminal {
        terminal: String,
    },
    GoInstall {
        version: String,
        arch: String,
    },
    GoToolchain(GoToolchainOperation),
    NerdFont {
        font: String,
    },
    NerdFonts(NerdFontsOperation),
    RepositoryKey {
        url: String,
        destination: String,
    },
    RustupBootstrap,
    RustToolchain(RustToolchainOperation),
    CargoPackageSet(CargoPackageOperation),
    CargoPackages {
        packages: Vec<String>,
        force: bool,
    },
    NodeInstall {
        version: String,
        npm: Vec<String>,
        update: bool,
    },
    NodeToolchain(NodeToolchainOperation),
    NpmPackageSet(NpmPackageOperation),
    NpmPackages {
        packages: Vec<String>,
    },
    PyenvInstall {
        update: bool,
        version: String,
        pip: bool,
    },
    SnapCleanup,
    UbuntuSnap(UbuntuSnapOperation),
    UnattendedUpgrades(UnattendedUpgradesOperation),
    UvInstall {
        version_enabled: bool,
        version: String,
    },
    UvBootstrap,
    PythonToolchain(PythonToolchainOperation),
    VirtualBoxConfig {
        user: String,
    },
    VirtualBoxGroup,
    VsCodeExtension {
        extension: String,
    },
    VsCodeExtensionSet(VsCodeExtensionOperation),
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
            Self::ManagedAptSources(operation) => operation.display_args(),
            Self::AptCodecs { package } => vec!["apt-codecs".into(), package.clone()],
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
            Self::AptSource { destination, .. } => {
                vec!["apt-source".into(), destination.clone()]
            }
            Self::Appimaged { arch } => vec!["appimaged".into(), arch.clone()],
            Self::DockerConfig { user } => vec!["docker-config".into(), user.clone()],
            Self::DockerGroup => vec!["docker-group".into()],
            Self::DockerLocalLog(operation) => operation.display_args(),
            Self::DesktopSetting(operation) => operation.display_args(),
            Self::DirectPackage(package) => package.display_args(),
            Self::Dotfiles(operation) => operation.display_args(),
            Self::DownloadBinary { name, .. } => vec!["download-binary".into(), name.clone()],
            Self::FlatpakEnsureFlathub => vec!["flatpak-ensure-flathub".into()],
            Self::FlatpakEnsureApps { refs } => std::iter::once("flatpak-ensure-apps".into())
                .chain(refs.clone())
                .collect(),
            Self::FlatpakUpdateApps { refs } => std::iter::once("flatpak-update-apps".into())
                .chain(refs.clone())
                .collect(),
            Self::FnmBootstrap => vec!["fnm-bootstrap".into()],
            Self::EnsureAdmin(operation) => operation.display_args(),
            Self::GnomeExtension { extension } => {
                vec!["gnome-extension".into(), extension.clone()]
            }
            Self::GnomeExtensions(operation) => operation.display_args(),
            Self::GnomeDependencies => vec!["gnome-dependencies".into()],
            Self::GnomeDockSettings => vec!["gnome-dock-settings".into()],
            Self::GnomeDock(operation) => operation.display_args(),
            Self::GnomeRoundedCornersSettings => vec!["gnome-rounded-corners-settings".into()],
            Self::GnomeRoundedCorners(operation) => operation.display_args(),
            Self::GnomeTerminal { terminal } => vec!["gnome-terminal".into(), terminal.clone()],
            Self::GoInstall { version, arch } => {
                vec!["go-install".into(), version.clone(), arch.clone()]
            }
            Self::GoToolchain(operation) => operation.display_args(),
            Self::NerdFont { font } => vec!["nerdfont".into(), font.clone()],
            Self::NerdFonts(operation) => operation.display_args(),
            Self::RepositoryKey { destination, .. } => {
                vec!["repository-key".into(), destination.clone()]
            }
            Self::RustupBootstrap => vec!["rustup-bootstrap".into()],
            Self::RustToolchain(operation) => operation.display_args(),
            Self::CargoPackageSet(operation) => operation.display_args(),
            Self::CargoPackages { packages, force } => {
                let mut args = vec!["cargo-packages".into()];
                if *force {
                    args.push("--force".into());
                }
                args.extend(packages.clone());
                args
            }
            Self::NodeInstall {
                version,
                npm,
                update,
            } => {
                let mut args = vec!["node-install".into(), version.clone()];
                if *update {
                    args.push("--update".into());
                }
                args.extend(npm.clone());
                args
            }
            Self::NodeToolchain(operation) => operation.display_args(),
            Self::NpmPackageSet(operation) => operation.display_args(),
            Self::NpmPackages { packages } => std::iter::once("npm-packages".into())
                .chain(packages.clone())
                .collect(),
            Self::PyenvInstall {
                update,
                version,
                pip,
            } => vec![
                "pyenv-install".into(),
                update.to_string(),
                version.clone(),
                pip.to_string(),
            ],
            Self::SnapCleanup => vec!["snap-cleanup".into()],
            Self::UbuntuSnap(operation) => operation.display_args(),
            Self::UnattendedUpgrades(operation) => operation.display_args(),
            Self::UvInstall {
                version_enabled,
                version,
            } => vec![
                "uv-install".into(),
                version_enabled.to_string(),
                version.clone(),
            ],
            Self::UvBootstrap => vec!["uv-bootstrap".into()],
            Self::PythonToolchain(operation) => operation.display_args(),
            Self::VirtualBoxConfig { user } => vec!["virtualbox-config".into(), user.clone()],
            Self::VirtualBoxGroup => vec!["virtualbox-group".into()],
            Self::VsCodeExtension { extension } => {
                vec!["vscode-extension".into(), extension.clone()]
            }
            Self::VsCodeExtensionSet(operation) => operation.display_args(),
        }
    }
}

pub fn execute(operation: &Operation, env: &[(OsString, OsString)]) -> Result<()> {
    execute_on_host(
        operation,
        Host {
            env,
            docker_lock_open_path: Path::new(DOCKER_LOCK),
        },
    )
}

#[doc(hidden)]
pub fn execute_with_docker_lock_for_test(
    operation: &Operation,
    env: &[(OsString, OsString)],
    docker_lock_open_path: &Path,
) -> Result<()> {
    execute_on_host(
        operation,
        Host {
            env,
            docker_lock_open_path,
        },
    )
}

fn execute_on_host(operation: &Operation, host: Host<'_>) -> Result<()> {
    match operation {
        Operation::AptBootstrapPackages { packages } => apt::bootstrap_packages(&host, packages),
        Operation::AptMetadataRefresh => apt::metadata_refresh(&host),
        Operation::ManagedAptSources(operation) => managed_apt::execute(&host, operation),
        Operation::AptCodecs { package } => provisioning::apt_codecs(&host, package),
        Operation::AptPackages { packages } => apt::packages(&host, packages),
        Operation::AptPurge { packages } => apt::purge(&host, packages),
        Operation::AptUpgrade { policy } => apt::upgrade(&host, *policy),
        Operation::AptSource {
            destination,
            contents,
        } => repository::source(&host, destination, contents),
        Operation::Appimaged { arch } => appimaged::execute(&host, arch),
        Operation::DockerConfig { user } => apps::docker(&host, user),
        Operation::DockerGroup => integrations::docker_group(&host),
        Operation::DockerLocalLog(operation) => integrations::docker_local_log(&host, operation),
        Operation::DesktopSetting(operation) => desktop::desktop_setting(&host, operation),
        Operation::DirectPackage(package) => direct::execute(&host, package),
        Operation::Dotfiles(operation) => dotfiles::execute(&host, operation),
        Operation::DownloadBinary {
            name,
            url,
            repo,
            pattern,
        } => downloads::binary(&host, name, url, repo, pattern),
        Operation::FlatpakEnsureFlathub => flatpak::ensure_flathub(&host),
        Operation::FlatpakEnsureApps { refs } => flatpak::ensure_apps(&host, refs),
        Operation::FlatpakUpdateApps { refs } => flatpak::update_apps(&host, refs),
        Operation::FnmBootstrap => languages::fnm_bootstrap(&host),
        Operation::EnsureAdmin(operation) => system::ensure_admin(&host, operation),
        Operation::GnomeExtension { extension } => desktop::gnome_extension(&host, extension),
        Operation::GnomeExtensions(operation) => desktop::gnome_extensions(&host, operation),
        Operation::GnomeDependencies => provisioning::gnome_dependencies(&host),
        Operation::GnomeDockSettings => provisioning::gnome_dock_settings(&host),
        Operation::GnomeDock(operation) => desktop::gnome_dock(&host, operation),
        Operation::GnomeRoundedCornersSettings => provisioning::gnome_rounded_settings(&host),
        Operation::GnomeRoundedCorners(operation) => {
            desktop::gnome_rounded_corners(&host, operation)
        }
        Operation::GnomeTerminal { terminal } => apps::gnome_terminal(&host, terminal),
        Operation::GoInstall { version, arch } => languages::go(&host, version, arch),
        Operation::GoToolchain(operation) => tools::execute_go(&host, operation),
        Operation::NerdFont { font } => downloads::nerdfont(&host, font),
        Operation::NerdFonts(operation) => fonts::execute(&host, operation),
        Operation::RepositoryKey { url, destination } => repository::key(&host, url, destination),
        Operation::RustupBootstrap => provisioning::rustup(&host),
        Operation::RustToolchain(operation) => tools::execute_rust(&host, operation),
        Operation::CargoPackageSet(operation) => cargo::execute(&host, operation),
        Operation::CargoPackages { packages, force } => {
            provisioning::cargo_packages(&host, packages, *force)
        }
        Operation::NodeInstall {
            version,
            npm,
            update,
        } => languages::node(&host, version, npm, *update),
        Operation::NodeToolchain(operation) => tools::execute_node(&host, operation),
        Operation::NpmPackageSet(operation) => npm::execute(&host, operation),
        Operation::NpmPackages { packages } => languages::npm_packages(&host, packages),
        Operation::PyenvInstall {
            update,
            version,
            pip,
        } => languages::pyenv(&host, *update, version, *pip),
        Operation::SnapCleanup => snap_cleanup::execute(&host),
        Operation::UbuntuSnap(operation) => system::ubuntu_snap(&host, operation),
        Operation::UnattendedUpgrades(operation) => system::unattended_upgrades(&host, operation),
        Operation::UvInstall {
            version_enabled,
            version,
        } => languages::uv(&host, *version_enabled, version),
        Operation::UvBootstrap => languages::uv_bootstrap(&host),
        Operation::PythonToolchain(operation) => tools::execute_python(&host, operation),
        Operation::VirtualBoxConfig { user } => apps::virtualbox(&host, user),
        Operation::VirtualBoxGroup => integrations::virtualbox_group(&host),
        Operation::VsCodeExtension { extension } => apps::vscode_extension(&host, extension),
        Operation::VsCodeExtensionSet(operation) => {
            integrations::vscode_extensions(&host, operation)
        }
    }
}

pub(crate) struct Host<'a> {
    env: &'a [(OsString, OsString)],
    docker_lock_open_path: &'a Path,
}

impl Host<'_> {
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
        command
            .output()
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

    pub fn require_retrying_etxtbsy<I, S>(
        &self,
        operation: &str,
        program: &str,
        args: I,
    ) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .collect::<Vec<_>>();
        let output = output_retrying_etxtbsy(|| {
            let mut command = Command::new(program);
            command.args(&args);
            for (key, value) in self.env {
                command.env(key, value);
            }
            command
        })
        .with_context(|| format!("{operation}: start {}", display(program, &args)))?;
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
        let mut child = command
            .spawn()
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
        self.value("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("~"))
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.value("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"))
    }

    pub fn command_exists(&self, name: &str) -> bool {
        self.value("PATH")
            .and_then(|path| std::env::split_paths(&path).find(|dir| dir.join(name).is_file()))
            .is_some()
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

fn output_retrying_etxtbsy(mut command: impl FnMut() -> Command) -> io::Result<Output> {
    for attempt in 0..=ETXTBSY_BACKOFFS.len() {
        let mut command = command();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        match command.spawn() {
            Ok(child) => return child.wait_with_output(),
            Err(error) if error.raw_os_error() == Some(ETXTBSY) => {
                let Some(backoff) = ETXTBSY_BACKOFFS.get(attempt) else {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "ETXTBSY persisted across {} spawn attempts: {error}",
                            ETXTBSY_BACKOFFS.len() + 1
                        ),
                    ));
                };
                thread::sleep(*backoff);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub fn new(host: &Host<'_>, stem: &str) -> Result<Self> {
        Self::new_in(&host.temp_dir(), stem)
    }

    pub fn new_in(parent: &Path, stem: &str) -> Result<Self> {
        for attempt in 0..100 {
            let path = parent.join(format!("{stem}.{}.{attempt}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("create operation temporary directory"),
            }
        }
        bail!("could not allocate operation temporary directory")
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn publish_file(source: &Path, destination: &Path, mode: u32) -> Result<()> {
    let parent = destination
        .parent()
        .context("downloaded file destination has no parent")?;
    fs::create_dir_all(parent).context("create downloaded file destination directory")?;
    let mut source_file = fs::File::open(source).context("open downloaded file")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .context("stage downloaded file on destination filesystem")?;
    std::io::copy(&mut source_file, staged.as_file_mut()).context("copy downloaded file")?;
    staged
        .as_file_mut()
        .set_permissions(fs::Permissions::from_mode(mode))
        .context("set downloaded file permissions")?;
    staged
        .as_file_mut()
        .sync_all()
        .context("sync downloaded file")?;
    let staged_path = staged.into_temp_path();
    staged_path
        .persist(destination)
        .map_err(|error| error.error)
        .context("publish downloaded file")?;
    Ok(())
}

pub(crate) struct TempPath(PathBuf);

impl TempPath {
    pub fn new(host: &Host<'_>, stem: &str) -> Result<Self> {
        Self::new_with_suffix(host, stem, "")
    }

    pub fn new_with_suffix(host: &Host<'_>, stem: &str, suffix: &str) -> Result<Self> {
        for attempt in 0..100 {
            let path = host
                .temp_dir()
                .join(format!("{stem}.{}.{attempt}{suffix}", std::process::id()));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("create operation temporary file"),
            }
        }
        bail!("could not allocate operation temporary file")
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn display(program: &str, args: &[OsString]) -> String {
    std::iter::once(OsStr::new(program))
        .chain(args.iter().map(OsString::as_os_str))
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{output_retrying_etxtbsy, publish_file, ETXTBSY};
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        os::unix::fs::{MetadataExt, PermissionsExt},
        process::Command,
        thread,
        time::Duration,
    };

    fn executable(directory: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = directory.join(name);
        let mut temporary = tempfile::NamedTempFile::new_in(directory).unwrap();
        write!(temporary, "#!/bin/sh\n{body}\n").unwrap();
        temporary.flush().unwrap();
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o755)).unwrap();
        temporary.as_file().sync_all().unwrap();
        temporary.into_temp_path().persist(&path).unwrap();
        fs::File::open(directory).unwrap().sync_all().unwrap();
        path
    }

    #[test]
    fn downloaded_files_publish_across_filesystems() {
        let source_dir = tempfile::tempdir().unwrap();
        let Ok(destination_dir) = tempfile::tempdir_in("/dev/shm") else {
            return;
        };
        if fs::metadata(source_dir.path()).unwrap().dev()
            == fs::metadata(destination_dir.path()).unwrap().dev()
        {
            return;
        }
        let source = source_dir.path().join("download");
        let destination = destination_dir.path().join("installed");
        fs::write(&source, b"complete artifact").unwrap();
        assert_eq!(
            fs::rename(&source, &destination)
                .unwrap_err()
                .raw_os_error(),
            Some(18)
        );

        publish_file(&source, &destination, 0o755).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"complete artifact");
    }

    #[test]
    fn etxtbsy_spawn_retries_until_writer_closes() {
        let directory = tempfile::tempdir().unwrap();
        let program = executable(directory.path(), "eventually-ready", "exit 0");
        let writer = OpenOptions::new().write(true).open(&program).unwrap();
        assert_eq!(
            Command::new(&program).spawn().unwrap_err().raw_os_error(),
            Some(ETXTBSY)
        );
        let closer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            drop(writer);
        });
        let mut attempts = 0;

        let output = output_retrying_etxtbsy(|| {
            attempts += 1;
            Command::new(&program)
        })
        .unwrap();

        closer.join().unwrap();
        assert!(output.status.success());
        assert!(attempts > 1);
        assert!(attempts <= 5);
    }

    #[test]
    fn persistent_etxtbsy_exhausts_five_spawn_attempts() {
        let directory = tempfile::tempdir().unwrap();
        let program = executable(directory.path(), "always-busy", "exit 0");
        let _writer = OpenOptions::new().write(true).open(&program).unwrap();
        let mut attempts = 0;

        let error = output_retrying_etxtbsy(|| {
            attempts += 1;
            Command::new(&program)
        })
        .unwrap_err();

        assert_eq!(attempts, 5);
        assert!(error.to_string().contains("ETXTBSY"));
        assert!(error.to_string().contains("5 spawn attempts"));
    }

    #[test]
    fn non_etxtbsy_spawn_errors_are_not_retried() {
        let directory = tempfile::tempdir().unwrap();
        let inaccessible = executable(directory.path(), "inaccessible", "exit 0");
        fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o644)).unwrap();

        for (program, expected_error) in [(inaccessible, 13), (directory.path().join("missing"), 2)]
        {
            let mut attempts = 0;
            let error = output_retrying_etxtbsy(|| {
                attempts += 1;
                Command::new(&program)
            })
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(expected_error));
            assert_eq!(attempts, 1);
        }
    }

    #[test]
    fn nonzero_exit_is_not_retried() {
        let directory = tempfile::tempdir().unwrap();
        let program = executable(directory.path(), "fails", "exit 42");
        let mut attempts = 0;

        let output = output_retrying_etxtbsy(|| {
            attempts += 1;
            Command::new(&program)
        })
        .unwrap();

        assert_eq!(output.status.code(), Some(42));
        assert_eq!(attempts, 1);
    }
}
