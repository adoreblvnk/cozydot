use anyhow::{Context, Result};
use serde::Deserialize;

use crate::operations::host::{self, is_regular_executable, shell::append_profile, temp_path};
use crate::platform::Arch;
use crate::style::WARNING;

const GO_PATH_INIT: &str = r#"export PATH="/usr/local/go/bin:$PATH""#;

pub(crate) fn is_installed(selector: &str, arch: Arch) -> Result<bool> {
    let go = "/usr/local/go/bin/go";
    if !is_regular_executable(go.as_ref()) {
        return Ok(false);
    }
    let target_os = if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let latest = if selector == "latest" { Some(latest_version()?) } else { None };
    let version = latest.as_deref().unwrap_or(selector);
    let expected = format!("go version go{version} {target_os}/{}", arch.go());
    let output = host::output(go, ["version"])?;
    if !output.status.success() {
        return Ok(false);
    }
    let stdout = std::str::from_utf8(&output.stdout).unwrap_or("").trim();
    Ok(stdout == expected)
}

pub(crate) fn install_toolchain(selector: &str, arch: Arch) -> Result<()> {
    let target_os = if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let latest = if selector == "latest" { Some(latest_version()?) } else { None };
    let version = latest.as_deref().unwrap_or(selector);
    let archive = temp_path("go", ".tar.gz")?;
    let archive_path = archive.to_str().context("Go archive path is not UTF-8")?;
    let url = format!("https://go.dev/dl/go{version}.{target_os}-{}.tar.gz", arch.go());
    host::curl("Go archive download", &url, ["--proto", "=https", "--output", archive_path])?;
    // remove whole tree so files missing from new release can't survive replacement
    host::run("Go installation replacement", "sudo", ["rm", "-rf", "/usr/local/go"])?;
    host::run("Go archive extraction", "sudo", ["tar", "-xzf", archive_path, "-C", "/usr/local"])?;
    append_profile(GO_PATH_INIT)
}

pub(crate) fn update_toolchain(selector: &str, arch: Arch) -> Result<()> {
    if selector != "latest" {
        anstream::eprintln!(
            "{WARNING}warning:{WARNING:#} skipping Go update because tools.go is pinned to an exact version"
        );
        return Ok(());
    }
    install_toolchain(selector, arch)
}

fn latest_version() -> Result<String> {
    #[derive(Deserialize)]
    struct Release {
        version: String,
    }

    let metadata = host::curl("Go release resolution", "https://go.dev/dl/?mode=json", ["--proto", "=https"])?;
    let releases: Vec<Release> = serde_json::from_slice(&metadata.stdout).context("parse Go release JSON")?;
    let version = &releases.first().context("Go metadata has no stable release")?.version;
    Ok(version.strip_prefix("go").unwrap_or(version).to_owned())
}
