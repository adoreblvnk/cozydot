mod appimaged;
mod apps;
mod apt;
mod desktop;
mod direct;
mod downloads;
mod flatpak;
mod languages;
mod provisioning;
mod repository;
mod snap_cleanup;

pub use apt::AptUpgradePolicy;
pub use direct::{
    DirectPackageFormat, DirectPackageMode, DirectPackageOperation, DirectPackageSelector,
    GithubRepository,
};

use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::Duration,
};

const ETXTBSY: i32 = 26;
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
    DirectPackage(DirectPackageOperation),
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
    GnomeExtension {
        extension: String,
    },
    GnomeDependencies,
    GnomeDockSettings,
    GnomeRoundedCornersSettings,
    GnomeTerminal {
        terminal: String,
    },
    GoInstall {
        version: String,
        arch: String,
    },
    NerdFont {
        font: String,
    },
    RepositoryKey {
        url: String,
        destination: String,
    },
    RustupBootstrap,
    CargoPackages {
        packages: Vec<String>,
        force: bool,
    },
    NodeInstall {
        version: String,
        npm: Vec<String>,
        update: bool,
    },
    NpmPackages {
        packages: Vec<String>,
    },
    PyenvInstall {
        update: bool,
        version: String,
        pip: bool,
    },
    SnapCleanup,
    UvInstall {
        version_enabled: bool,
        version: String,
    },
    VirtualBoxConfig {
        user: String,
    },
    VsCodeExtension {
        extension: String,
    },
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
            Self::DirectPackage(package) => package.display_args(),
            Self::DownloadBinary { name, .. } => vec!["download-binary".into(), name.clone()],
            Self::FlatpakEnsureFlathub => vec!["flatpak-ensure-flathub".into()],
            Self::FlatpakEnsureApps { refs } => std::iter::once("flatpak-ensure-apps".into())
                .chain(refs.clone())
                .collect(),
            Self::FlatpakUpdateApps { refs } => std::iter::once("flatpak-update-apps".into())
                .chain(refs.clone())
                .collect(),
            Self::GnomeExtension { extension } => {
                vec!["gnome-extension".into(), extension.clone()]
            }
            Self::GnomeDependencies => vec!["gnome-dependencies".into()],
            Self::GnomeDockSettings => vec!["gnome-dock-settings".into()],
            Self::GnomeRoundedCornersSettings => vec!["gnome-rounded-corners-settings".into()],
            Self::GnomeTerminal { terminal } => vec!["gnome-terminal".into(), terminal.clone()],
            Self::GoInstall { version, arch } => {
                vec!["go-install".into(), version.clone(), arch.clone()]
            }
            Self::NerdFont { font } => vec!["nerdfont".into(), font.clone()],
            Self::RepositoryKey { destination, .. } => {
                vec!["repository-key".into(), destination.clone()]
            }
            Self::RustupBootstrap => vec!["rustup-bootstrap".into()],
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
            Self::UvInstall {
                version_enabled,
                version,
            } => vec![
                "uv-install".into(),
                version_enabled.to_string(),
                version.clone(),
            ],
            Self::VirtualBoxConfig { user } => vec!["virtualbox-config".into(), user.clone()],
            Self::VsCodeExtension { extension } => {
                vec!["vscode-extension".into(), extension.clone()]
            }
        }
    }
}

pub fn execute(operation: &Operation, env: &[(OsString, OsString)]) -> Result<()> {
    let host = Host { env };
    match operation {
        Operation::AptBootstrapPackages { packages } => apt::bootstrap_packages(&host, packages),
        Operation::AptMetadataRefresh => apt::metadata_refresh(&host),
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
        Operation::DirectPackage(package) => direct::execute(&host, package),
        Operation::DownloadBinary {
            name,
            url,
            repo,
            pattern,
        } => downloads::binary(&host, name, url, repo, pattern),
        Operation::FlatpakEnsureFlathub => flatpak::ensure_flathub(&host),
        Operation::FlatpakEnsureApps { refs } => flatpak::ensure_apps(&host, refs),
        Operation::FlatpakUpdateApps { refs } => flatpak::update_apps(&host, refs),
        Operation::GnomeExtension { extension } => desktop::gnome_extension(&host, extension),
        Operation::GnomeDependencies => provisioning::gnome_dependencies(&host),
        Operation::GnomeDockSettings => provisioning::gnome_dock_settings(&host),
        Operation::GnomeRoundedCornersSettings => provisioning::gnome_rounded_settings(&host),
        Operation::GnomeTerminal { terminal } => apps::gnome_terminal(&host, terminal),
        Operation::GoInstall { version, arch } => languages::go(&host, version, arch),
        Operation::NerdFont { font } => downloads::nerdfont(&host, font),
        Operation::RepositoryKey { url, destination } => repository::key(&host, url, destination),
        Operation::RustupBootstrap => provisioning::rustup(&host),
        Operation::CargoPackages { packages, force } => {
            provisioning::cargo_packages(&host, packages, *force)
        }
        Operation::NodeInstall {
            version,
            npm,
            update,
        } => languages::node(&host, version, npm, *update),
        Operation::NpmPackages { packages } => languages::npm_packages(&host, packages),
        Operation::PyenvInstall {
            update,
            version,
            pip,
        } => languages::pyenv(&host, *update, version, *pip),
        Operation::SnapCleanup => snap_cleanup::execute(&host),
        Operation::UvInstall {
            version_enabled,
            version,
        } => languages::uv(&host, *version_enabled, version),
        Operation::VirtualBoxConfig { user } => apps::virtualbox(&host, user),
        Operation::VsCodeExtension { extension } => apps::vscode_extension(&host, extension),
    }
}

pub(crate) struct Host<'a> {
    env: &'a [(OsString, OsString)],
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
        for attempt in 0..100 {
            let path = host
                .temp_dir()
                .join(format!("{stem}.{}.{attempt}", std::process::id()));
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
        os::unix::fs::{MetadataExt, PermissionsExt},
        process::Command,
        thread,
        time::Duration,
    };

    fn executable(directory: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = directory.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
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
