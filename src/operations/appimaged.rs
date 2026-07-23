use super::{Host, TempPath};
use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{fs, io::Read, os::unix::fs::PermissionsExt, path::Path, thread, time::Duration};
use url::Url;

const RELEASE_API: &str = "https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous";
const GITHUB_ACCEPT: &str = "Accept: application/vnd.github+json";
const GITHUB_API_VERSION: &str = "X-GitHub-Api-Version: 2022-11-28";
const USER_AGENT: &str = concat!("User-Agent: cozydot/", env!("CARGO_PKG_VERSION"));

pub(crate) fn execute(host: &Host, architecture: Architecture) -> Result<()> {
    if host.run("systemctl", ["--user", "--quiet", "is-active", "appimaged.service"])?.status.success() {
        return Ok(());
    }

    ensure_fuse(host)?;
    let home = host.home();
    let destination = home.join("Applications/appimaged.AppImage");
    if fs::symlink_metadata(&destination)
        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0)
        && require_elf(&destination, architecture).is_ok()
    {
        let program =
            destination.to_str().with_context(|| format!("appimaged path is not UTF-8: {}", destination.display()))?;
        host.require("launch appimaged", program, std::iter::empty::<&str>())?;
        wait_until_active(host)?;
        return Ok(());
    }
    let _ = host.run("systemctl", ["--user", "stop", "appimaged.service"])?;

    let applications = home.join("Applications");
    fs::create_dir_all(&applications).context("create Applications directory")?;
    let url = resolve_asset(host, architecture)?;
    let temporary = TempPath::new_in_with_suffix(&applications, "appimaged-", ".AppImage")?;
    host.require(
        "download appimaged",
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
            temporary.path().as_os_str(),
            "--".as_ref(),
            url.as_str().as_ref(),
        ],
    )?;
    require_elf(temporary.path(), architecture)?;
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o755))?;
    let destination = applications.join("appimaged.AppImage");
    fs::rename(temporary.path(), &destination).context("publish appimaged")?;

    let program =
        destination.to_str().with_context(|| format!("appimaged path is not UTF-8: {}", destination.display()))?;
    host.require("launch appimaged", program, std::iter::empty::<&str>())?;
    wait_until_active(host)?;
    Ok(())
}

fn ensure_fuse(host: &Host) -> Result<()> {
    let package =
        if host.run("apt-cache", ["show", "libfuse2t64"])?.status.success() { "libfuse2t64" } else { "libfuse2" };
    if !package_is_installed(host, package)? {
        host.require("refresh APT for appimaged", "sudo", ["apt-get", "update", "-qq"])?;
        host.require("install appimaged FUSE support", "sudo", ["apt-get", "install", "-y", "-qq", "--", package])?;
    }
    Ok(())
}

fn package_is_installed(host: &Host, package: &str) -> Result<bool> {
    let output = host.run("dpkg-query", ["--show", "--showformat=${db:Status-Abbrev}", package])?;
    Ok(output.status.success() && output.stdout == b"ii ")
}

fn wait_until_active(host: &Host) -> Result<()> {
    for attempt in 0..20 {
        if host.run("systemctl", ["--user", "--quiet", "is-active", "appimaged.service"])?.status.success() {
            return Ok(());
        }
        if attempt < 19 {
            thread::sleep(Duration::from_millis(250));
        }
    }
    bail!("appimaged did not become active after launch")
}

fn resolve_asset(host: &Host, architecture: Architecture) -> Result<Url> {
    let output = host.require(
        "resolve appimaged release",
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
            "--header",
            GITHUB_ACCEPT,
            "--header",
            GITHUB_API_VERSION,
            "--header",
            USER_AGENT,
            RELEASE_API,
        ],
    )?;
    let release: Value = serde_json::from_slice(&output.stdout).context("parse appimaged release JSON")?;
    let assets = release
        .as_object()
        .and_then(|release| release.get("assets"))
        .and_then(Value::as_array)
        .context("appimaged release JSON is missing assets")?;
    let suffix = match architecture {
        Architecture::Amd64 => "-x86_64.AppImage",
        Architecture::Arm64 => "-aarch64.AppImage",
        Architecture::Arm32 => "-armhf.AppImage",
    };
    let matches = assets
        .iter()
        .filter_map(|asset| {
            let object = asset.as_object()?;
            let name = object.get("name")?.as_str()?;
            (name.starts_with("appimaged-") && name.ends_with(suffix))
                .then(|| object.get("browser_download_url")?.as_str())
                .flatten()
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!("appimaged release selector matched {} assets for {}", matches.len(), architecture.canonical());
    }
    Url::parse(matches[0]).context("selected appimaged release asset has an invalid browser_download_url")
}

fn require_elf(path: &Path, architecture: Architecture) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let mut header = [0; 20];
    let expected_machine = match architecture {
        Architecture::Amd64 => 62,
        Architecture::Arm64 => 183,
        Architecture::Arm32 => 40,
    };
    if !metadata.file_type().is_file()
        || metadata.len() == 0
        || fs::File::open(path).and_then(|mut file| file.read_exact(&mut header)).is_err()
        || header[..4] != *b"\x7fELF"
        || header[5] != 1
        || u16::from_le_bytes([header[18], header[19]]) != expected_machine
    {
        bail!("appimaged is not a nonempty ELF file for {}", architecture.canonical());
    }
    Ok(())
}
