use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::{Host, TempDir, TempPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolMutationMode {
    EnsurePresent,
    UpdateMoving,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoToolchainSelector {
    Latest,
    Version(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GoRelease {
    version: String,
    filename: String,
    checksum: String,
}

pub(crate) fn execute_rust(host: &Host, selector: &str) -> Result<()> {
    let rustup = resolve_managed(host, "CARGO_HOME", ".cargo", "bin/rustup")?
        .context("Rust toolchain operation: rustup is unavailable after bootstrap")?;
    host.require("Rust toolchain mutation", &rustup, rust_install_args(selector))?;
    host.require("Rust default toolchain mutation", &rustup, ["default", "--", selector])?;
    Ok(())
}

pub(crate) fn execute_go(
    host: &Host,
    selector: &GoToolchainSelector,
    architecture: Architecture,
    mode: ToolMutationMode,
) -> Result<()> {
    let expected_arch = architecture.go();

    let current = inspect_go(host, "/usr/local/go/bin/go")?;
    if mode == ToolMutationMode::EnsurePresent
        && let Some(state) = &current
        && state.architecture == expected_arch
    {
        match selector {
            GoToolchainSelector::Latest => return Ok(()),
            GoToolchainSelector::Version(requested) if version_matches(&state.version, requested) => return Ok(()),
            GoToolchainSelector::Version(_) => {}
        }
    }

    let requested = match selector {
        GoToolchainSelector::Latest => "latest",
        GoToolchainSelector::Version(version) => version,
    };

    let GoRelease { version, filename, checksum } = resolve_go_release(host, requested, architecture)?;

    if current.as_ref().is_some_and(|state| state.version == version && state.architecture == expected_arch) {
        return Ok(());
    }

    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        bail!("Go release metadata contains an invalid SHA-256 checksum");
    }
    let archive = TempPath::new_with_suffix(host, "go", ".tar.gz")?;
    let stage = TempDir::new(host, "go-stage")?;
    let url = format!("https://go.dev/dl/{filename}");
    host.require(
        "Go archive download",
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
            url.as_ref(),
        ],
    )?;
    let checksum_input = format!("{checksum}  {}\n", archive.path().display());
    host.require_input("Go archive checksum", "sha256sum", ["--check", "--status", "-"], checksum_input.as_bytes())?;
    let listing =
        host.require("Go archive preflight", "tar", ["--list", "--gzip", "--file", &archive.path().to_string_lossy()])?;
    validate_go_archive_listing(&listing.stdout)?;
    host.require(
        "Go archive extraction",
        "tar",
        [
            "--extract",
            "--gzip",
            "--directory",
            &stage.path().to_string_lossy(),
            "--file",
            &archive.path().to_string_lossy(),
        ],
    )?;
    let staged = stage.path().join("go");
    let staged_binary = staged.join("bin/go");
    let staged_program = path_program(&staged_binary, "staged Go executable")?;
    let staged_state =
        inspect_go(host, &staged_program)?.context("Go archive does not contain an executable Go toolchain")?;
    if staged_state.version != version || staged_state.architecture != expected_arch {
        bail!("Go archive toolchain does not match resolved release metadata");
    }
    host.require("Go toolchain publication", "sudo", ["rm", "-rf", "--", "/usr/local/go"])?;
    host.require(
        "Go toolchain publication",
        "sudo",
        ["mv".as_ref(), "--".as_ref(), staged.as_os_str(), "/usr/local/go".as_ref()],
    )?;
    let installed = inspect_go(host, "/usr/local/go/bin/go")?
        .context("Go toolchain publication did not create /usr/local/go/bin/go")?;
    if installed.version != version || installed.architecture != expected_arch {
        bail!("Go toolchain publication produced mismatched version or architecture");
    }
    Ok(())
}

pub(crate) fn execute_node(host: &Host, selector: &str) -> Result<()> {
    let fnm = resolve_fnm(host)?;
    let default = if selector == "lts" {
        host.require("Node toolchain mutation", &fnm, ["install", "--progress", "never", "--lts"])?;
        "lts-latest"
    } else {
        host.require("Node toolchain mutation", &fnm, ["install", "--progress", "never", "--", selector])?;
        selector
    };
    host.require("Node default toolchain mutation", &fnm, ["default", "--", default])?;
    Ok(())
}

pub(crate) fn execute_python(host: &Host, version: &str) -> Result<()> {
    let uv = resolve_managed(host, "UV_INSTALL_DIR", ".local/bin", "uv")?
        .context("Python toolchain operation: uv is unavailable after bootstrap")?;

    host.require(
        "Python toolchain mutation",
        &uv,
        ["python", "install", "--no-config", "--managed-python", "--no-progress", "--default", "--", version],
    )?;
    Ok(())
}

mod resolution {
    use super::*;

    pub(super) fn resolve_go_release(host: &Host, requested: &str, architecture: Architecture) -> Result<GoRelease> {
        let metadata = host.require(
            "Go release resolution",
            "curl",
            [
                "--proto",
                "=https",
                "--location",
                "--fail",
                "--silent",
                "--show-error",
                "--retry",
                "3",
                "--retry-all-errors",
                "--",
                "https://go.dev/dl/?mode=json&include=all",
            ],
        )?;
        let (version, filename, checksum) = super::super::latest_go(
            std::str::from_utf8(&metadata.stdout).context("Go release metadata is not UTF-8")?,
            requested,
            architecture.go_archive(),
        )?;
        Ok(GoRelease { version, filename, checksum })
    }
}

mod state {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct GoState {
        pub(super) version: String,
        pub(super) architecture: String,
    }

    pub(super) fn inspect_go(host: &Host, program: &str) -> Result<Option<GoState>> {
        if program.starts_with('/') && !executable_file(Path::new(program)) {
            return Ok(None);
        }
        let output = match host.run(program, ["version"]) {
            Ok(output) => output,
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                    error.kind() == std::io::ErrorKind::NotFound || error.kind() == std::io::ErrorKind::PermissionDenied
                }) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if !output.status.success() {
            return Ok(None);
        }
        parse_go_state(&output.stdout).map(Some)
    }

    pub(super) fn parse_go_state(output: &[u8]) -> Result<GoState> {
        let output = single_line(output, "go version")?;
        let fields = output.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 4
            || fields[0] != "go"
            || fields[1] != "version"
            || fields[3] != "linux/amd64" && fields[3] != "linux/arm64" && fields[3] != "linux/arm"
        {
            bail!("go returned malformed version state");
        }
        let version = fields[2]
            .strip_prefix("go")
            .filter(|version| numeric_version(version, 2, 3))
            .context("go returned malformed version state")?;
        Ok(GoState { version: version.to_owned(), architecture: fields[3].trim_start_matches("linux/").to_owned() })
    }

    pub(super) fn validate_go_archive_listing(output: &[u8]) -> Result<()> {
        let output = std::str::from_utf8(output).context("Go archive listing is not UTF-8")?;
        let mut saw_binary = false;
        for entry in output.lines() {
            if entry.is_empty()
                || !entry.starts_with("go/")
                || entry.split('/').any(|component| component == "..")
                || entry.chars().any(char::is_control)
            {
                bail!("Go archive contains an unsafe path");
            }
            saw_binary |= entry == "go/bin/go";
        }
        if !saw_binary {
            bail!("Go archive listing does not contain go/bin/go");
        }
        Ok(())
    }

    pub(super) fn resolve_fnm(host: &Host) -> Result<String> {
        let data_home =
            host.value("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/share"));
        if !data_home.is_absolute() {
            bail!("managed FNM data directory must be absolute");
        }
        let managed = data_home.join("fnm/fnm");
        if executable_file(&managed) {
            return path_program(&managed, "managed fnm executable");
        }
        bail!("Node toolchain operation: fnm is unavailable after bootstrap")
    }

    pub(super) fn resolve_managed(
        host: &Host,
        directory_variable: &str,
        default_directory: &str,
        relative_program: &str,
    ) -> Result<Option<String>> {
        let base =
            host.value(directory_variable).map(PathBuf::from).unwrap_or_else(|| host.home().join(default_directory));
        if !base.is_absolute() {
            bail!("managed tool directory must be absolute");
        }
        let managed = base.join(relative_program);
        if executable_file(&managed) {
            return path_program(&managed, "managed tool executable").map(Some);
        }
        Ok(None)
    }

    fn executable_file(path: &Path) -> bool {
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0)
    }

    pub(super) fn path_program(path: &Path, description: &str) -> Result<String> {
        path.to_str().map(str::to_owned).with_context(|| format!("{description} path is not UTF-8: {}", path.display()))
    }
}

use resolution::*;
use state::*;

fn rust_install_args(toolchain: &str) -> [&str; 7] {
    ["toolchain", "install", "--profile", "minimal", "--no-self-update", "--", toolchain]
}

fn version_matches(actual: &str, requested: &str) -> bool {
    actual == requested || actual.strip_prefix(requested).is_some_and(|rest| rest.starts_with('.'))
}

fn numeric_version(value: &str, min_parts: usize, max_parts: usize) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    (min_parts..=max_parts).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        })
}

fn single_line<'a>(output: &'a [u8], command: &str) -> Result<&'a str> {
    let output = std::str::from_utf8(output).with_context(|| format!("{command} returned non-UTF-8 state"))?;
    let output = output.strip_suffix('\n').unwrap_or(output);
    if output.is_empty() || output.contains(['\n', '\r']) {
        bail!("{command} returned malformed multiline state");
    }
    Ok(output)
}
