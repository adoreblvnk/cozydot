use anyhow::{Context, Result, bail};
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

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
        self.run(&format!("{label} CLI version check"), program, ["--version"])
            .with_context(|| format!("{label} integration requires an existing usable {program} CLI"))?;
        Ok(())
    }

    pub fn curl<I, S>(&self, label: &str, url: &str, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut curl_args = ["--location", "--fail", "--silent", "--show-error", "--retry", "3", "--retry-all-errors"]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        curl_args.extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        // keep URL after `--` so a leading hyphen can't become a curl option
        curl_args.extend([OsString::from("--"), OsString::from(url)]);
        self.run(label, "curl", curl_args)
    }

    pub fn home(&self) -> PathBuf {
        self.home.clone()
    }

    pub fn executable_on_path(&self, name: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|directory| executable_file(&directory.join(name))))
    }
}

pub(crate) fn executable_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(crate) fn regular_executable_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(crate) fn path_program(path: &Path, description: &str) -> Result<String> {
    path.to_str().map(str::to_owned).with_context(|| format!("{description} is not UTF-8: {}", path.display()))
}

pub(crate) fn require_regular_executable(path: &Path, description: &str, unavailable: &str) -> Result<String> {
    if !regular_executable_file(path) {
        bail!("{unavailable}");
    }
    path_program(path, description)
}

pub(crate) fn one_record<'a>(bytes: &'a [u8], command: &str) -> Result<&'a str> {
    let output = std::str::from_utf8(bytes).with_context(|| format!("{command} returned non-UTF-8 output"))?;
    let record = output.strip_suffix('\n').unwrap_or(output);
    if record.is_empty() || record.contains(['\n', '\r']) {
        bail!("{command} returned malformed record output");
    }
    Ok(record)
}

pub(crate) struct TempPath(tempfile::TempPath);

impl TempPath {
    pub fn new(stem: &str) -> Result<Self> {
        Self::new_with_suffix(stem, "")
    }

    pub fn new_with_suffix(stem: &str, suffix: &str) -> Result<Self> {
        tempfile::Builder::new()
            .prefix(stem)
            .suffix(suffix)
            .tempfile()
            .map(|file| Self(file.into_temp_path()))
            .context("create temporary file")
    }

    pub fn new_in_with_suffix(parent: &Path, stem: &str, suffix: &str) -> Result<Self> {
        tempfile::Builder::new()
            .prefix(stem)
            .suffix(suffix)
            .tempfile_in(parent)
            .map(|file| Self(file.into_temp_path()))
            .context("create temporary file")
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
