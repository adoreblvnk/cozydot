mod appimaged;
mod apps;
mod desktop;
mod downloads;
mod languages;
mod snap_cleanup;

use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Appimaged {
        arch: String,
    },
    DockerConfig {
        user: String,
    },
    DownloadBinary {
        name: String,
        url: String,
        repo: String,
        pattern: String,
    },
    GnomeExtension {
        extension: String,
    },
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
    NodeInstall {
        version: String,
        npm: Vec<String>,
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
            Self::Appimaged { arch } => vec!["appimaged".into(), arch.clone()],
            Self::DockerConfig { user } => vec!["docker-config".into(), user.clone()],
            Self::DownloadBinary { name, .. } => vec!["download-binary".into(), name.clone()],
            Self::GnomeExtension { extension } => {
                vec!["gnome-extension".into(), extension.clone()]
            }
            Self::GnomeTerminal { terminal } => vec!["gnome-terminal".into(), terminal.clone()],
            Self::GoInstall { version, arch } => {
                vec!["go-install".into(), version.clone(), arch.clone()]
            }
            Self::NerdFont { font } => vec!["nerdfont".into(), font.clone()],
            Self::NodeInstall { version, npm } => std::iter::once("node-install".into())
                .chain(std::iter::once(version.clone()))
                .chain(npm.clone())
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
        Operation::Appimaged { arch } => appimaged::execute(&host, arch),
        Operation::DockerConfig { user } => apps::docker(&host, user),
        Operation::DownloadBinary {
            name,
            url,
            repo,
            pattern,
        } => downloads::binary(&host, name, url, repo, pattern),
        Operation::GnomeExtension { extension } => desktop::gnome_extension(&host, extension),
        Operation::GnomeTerminal { terminal } => apps::gnome_terminal(&host, terminal),
        Operation::GoInstall { version, arch } => languages::go(&host, version, arch),
        Operation::NerdFont { font } => downloads::nerdfont(&host, font),
        Operation::NodeInstall { version, npm } => languages::node(&host, version, npm),
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

pub(crate) struct TempPath(PathBuf);

impl TempPath {
    pub fn new(host: &Host<'_>, stem: &str) -> Result<Self> {
        for attempt in 0..100 {
            let path = host
                .temp_dir()
                .join(format!("{stem}.{}.{attempt}", std::process::id()));
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
