use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, bail};

pub(crate) mod macos;
pub(crate) mod privileged_file;
pub(crate) mod shell;
pub(crate) mod systemd;
pub(crate) mod users;

pub(crate) struct Host {
    home: PathBuf,
}

impl Host {
    pub(crate) fn new() -> Result<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from).context("HOME is not set")?;
        Ok(Self { home })
    }

    pub fn output<I, S>(&self, program: &str, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args.into_iter().map(|arg| arg.as_ref().to_os_string()).collect::<Vec<_>>();
        let mut command = Command::new(program);
        command.args(&args);
        command.output().with_context(|| format!("start command {}", display(program, &args)))
    }

    pub fn run<I, S>(&self, label: &str, program: &str, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output(program, args)?;
        if !output.status.success() {
            bail!("{label}: {program} failed ({}): {}", output.status, String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(output)
    }

    pub fn require_cli(&self, label: &str, program: &str) -> Result<()> {
        let version_check = self.run(&format!("{label} CLI version check"), program, ["--version"]);
        version_check.with_context(|| format!("{label} integration requires an existing usable {program} CLI"))?;
        Ok(())
    }

    pub fn curl<I, S>(&self, label: &str, url: &str, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let options = ["--location", "--fail", "--silent", "--show-error", "--retry", "3", "--retry-all-errors"];
        let mut curl_args = options.map(OsString::from).to_vec();
        for arg in args {
            curl_args.push(arg.as_ref().to_os_string());
        }
        // keep URL after `--` so a leading hyphen can't become a curl option
        curl_args.extend([OsString::from("--"), OsString::from(url)]);
        self.run(label, "curl", curl_args)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn has_executable_on_path(&self, name: &str) -> bool {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        for directory in std::env::split_paths(&path) {
            if is_executable(&directory.join(name)) {
                return true;
            }
        }
        false
    }
}

pub(crate) struct TempPath(tempfile::TempPath);

impl TempPath {
    pub fn new(stem: &str) -> Result<Self> {
        Self::new_with_suffix(stem, "")
    }

    // TODO: in the future we can use tempfile directly instead of wrapping it?
    pub fn new_with_suffix(stem: &str, suffix: &str) -> Result<Self> {
        let file = tempfile::Builder::new().prefix(stem).suffix(suffix).tempfile().context("create temporary file")?;
        Ok(Self(file.into_temp_path()))
    }

    pub fn new_in_with_suffix(parent: &Path, stem: &str, suffix: &str) -> Result<Self> {
        let file = tempfile::Builder::new()
            .prefix(stem)
            .suffix(suffix)
            .tempfile_in(parent)
            .context("create temporary file")?;
        Ok(Self(file.into_temp_path()))
    }

    pub fn path(&self) -> &Path {
        self.0.as_ref()
    }
}

pub(crate) fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(crate) fn is_regular_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0
}

pub(crate) fn path_program(path: &Path, description: &str) -> Result<String> {
    path.to_str().map(str::to_owned).with_context(|| format!("{description} is not UTF-8: {}", path.display()))
}

pub(crate) fn require_regular_executable(path: &Path, description: &str, unavailable: &str) -> Result<String> {
    if !is_regular_executable(path) {
        bail!("{unavailable}");
    }
    path_program(path, description)
}

/// Returns stdout as 1 line after removing trailing newline. Rejects empty / multiline stdout.
pub(crate) fn stdout_line<'a>(bytes: &'a [u8], command: &str) -> Result<&'a str> {
    let output = std::str::from_utf8(bytes).with_context(|| format!("{command} returned non-UTF-8 output"))?;
    let record = output.strip_suffix('\n').unwrap_or(output);
    if record.is_empty() || record.contains('\n') {
        bail!("{command} returned malformed record output");
    }
    Ok(record)
}

fn display(program: &str, args: &[OsString]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(OsStr::new(program).to_string_lossy());
    for arg in args {
        parts.push(arg.to_string_lossy());
    }
    parts.join(" ")
}
