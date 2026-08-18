use crate::platform::Architecture;
use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Host, TempPath, regular_executable_file, shell::append_profile};

const GO_PATH_INIT: &str = r#"export PATH="/usr/local/go/bin:$PATH""#;

pub enum GoToolchainSelector {
    Latest,
    Version(String),
}

pub(crate) fn install_toolchain(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<()> {
    let target_os = if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let version = match selector {
        GoToolchainSelector::Latest => latest_version(host)?,
        GoToolchainSelector::Version(version) => version.clone(),
    };
    let program = "/usr/local/go/bin/go";
    let expected = format!("go version go{version} {target_os}/{}", architecture.go());
    // verify that Go is executable & go version output matches expected version & platform
    if !regular_executable_file(program.as_ref())
        || !host.output(program, ["version"]).is_ok_and(|output| {
            output.status.success() && std::str::from_utf8(&output.stdout).is_ok_and(|stdout| stdout.trim() == expected)
        })
    {
        let archive = TempPath::new_with_suffix("go", ".tar.gz")?;
        let filename = format!("go{version}.{target_os}-{}.tar.gz", architecture.go_archive());
        let url = format!("https://go.dev/dl/{filename}");
        host.curl(
            "Go archive download",
            &url,
            ["--proto".as_ref(), "=https".as_ref(), "--output".as_ref(), archive.path().as_os_str()],
        )?;
        // remove whole tree so files missing from new release can't survive replacement
        host.run("Go installation replacement", "sudo", ["rm", "-rf", "/usr/local/go"])?;
        host.run(
            "Go archive extraction",
            "sudo",
            ["tar", "-xzf", archive.path().to_str().context("Go archive path is not UTF-8")?, "-C", "/usr/local"],
        )?;
    }
    append_profile(host, GO_PATH_INIT)
}

pub(crate) fn update_toolchain(host: &Host, selector: &GoToolchainSelector, architecture: Architecture) -> Result<()> {
    if matches!(selector, GoToolchainSelector::Version(_)) {
        eprintln!("warning: Go update skipped because shared.tools.go is pinned to an exact version");
        return Ok(());
    }
    install_toolchain(host, selector, architecture)
}

fn latest_version(host: &Host) -> Result<String> {
    #[derive(Deserialize)]
    struct Release {
        version: String,
    }

    let metadata = host.curl("Go release resolution", "https://go.dev/dl/?mode=json", ["--proto", "=https"])?;
    let releases: Vec<Release> = serde_json::from_slice(&metadata.stdout).context("parse Go release JSON")?;
    releases
        .first()
        .context("Go metadata has no stable release")?
        .version
        .strip_prefix("go")
        .map(str::to_owned)
        .context("Go metadata returned malformed release version")
}
