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

pub(crate) fn output<I, S>(program: &str, args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args.into_iter().map(|arg| arg.as_ref().to_os_string()).collect::<Vec<_>>();
    let mut command = Command::new(program);
    command.args(&args);
    command.output().with_context(|| format!("start command {}", display(program, &args)))
}

pub(crate) fn run<I, S>(label: &str, program: &str, args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = output(program, args)?;
    if !output.status.success() {
        bail!("{label}: {program} failed ({}): {}", output.status, String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output)
}

pub(crate) fn require_cli(label: &str, program: &str) -> Result<()> {
    let version_check = run(&format!("{label} CLI version check"), program, ["--version"]);
    version_check.with_context(|| format!("{label} integration requires an existing usable {program} CLI"))?;
    Ok(())
}

pub(crate) fn curl<I, S>(label: &str, url: &str, args: I) -> Result<Output>
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
    run(label, "curl", curl_args)
}

pub(crate) fn home() -> Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).context("HOME is not set")
}

pub(crate) fn has_executable_on_path(name: &str) -> bool {
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

pub(crate) fn temp_path(stem: &str) -> Result<tempfile::TempPath> {
    temp_path_with_suffix(stem, "")
}

pub(crate) fn temp_path_with_suffix(stem: &str, suffix: &str) -> Result<tempfile::TempPath> {
    let file = tempfile::Builder::new().prefix(stem).suffix(suffix).tempfile().context("create temporary file")?;
    Ok(file.into_temp_path())
}

pub(crate) fn temp_path_in_with_suffix(parent: &Path, stem: &str, suffix: &str) -> Result<tempfile::TempPath> {
    let file =
        tempfile::Builder::new().prefix(stem).suffix(suffix).tempfile_in(parent).context("create temporary file")?;
    Ok(file.into_temp_path())
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
