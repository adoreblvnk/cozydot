use anyhow::{Context, Result};
use serde::Deserialize;

use crate::operations::host::{Host, TempPath, is_regular_executable, shell::append_profile};
use crate::platform::Architecture;

const GO_PATH_INIT: &str = r#"export PATH="/usr/local/go/bin:$PATH""#;

pub(crate) fn install_toolchain(host: &Host, selector: &str, architecture: Architecture) -> Result<()> {
    let target_os = if cfg!(target_os = "macos") { "darwin" } else { "linux" };
    let latest = if selector == "latest" { Some(latest_version(host)?) } else { None };
    let version = latest.as_deref().unwrap_or(selector);
    let program = "/usr/local/go/bin/go";
    let expected = format!("go version go{version} {target_os}/{}", architecture.go());
    // verify that Go is executable & go version output matches expected version & platform
    let installed = is_regular_executable(program.as_ref());
    let output = if installed { host.output(program, ["version"]).ok() } else { None };
    let successful_output = output.as_ref().filter(|output| output.status.success());
    let stdout = successful_output.and_then(|output| std::str::from_utf8(&output.stdout).ok());
    let version_matches = stdout.is_some_and(|stdout| stdout.trim() == expected);
    if !version_matches {
        let archive = TempPath::new_with_suffix("go", ".tar.gz")?;
        let filename = format!("go{version}.{target_os}-{}.tar.gz", architecture.go());
        let url = format!("https://go.dev/dl/{filename}");
        let output = archive.path().as_os_str();
        host.curl("Go archive download", &url, ["--proto".as_ref(), "=https".as_ref(), "--output".as_ref(), output])?;
        // remove whole tree so files missing from new release can't survive replacement
        host.run("Go installation replacement", "sudo", ["rm", "-rf", "/usr/local/go"])?;
        let archive = archive.path().to_str().context("Go archive path is not UTF-8")?;
        host.run("Go archive extraction", "sudo", ["tar", "-xzf", archive, "-C", "/usr/local"])?;
    }
    append_profile(host, GO_PATH_INIT)
}

pub(crate) fn update_toolchain(host: &Host, selector: &str, architecture: Architecture) -> Result<()> {
    if selector != "latest" {
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
    let version = &releases.first().context("Go metadata has no stable release")?.version;
    version.strip_prefix("go").map(str::to_owned).context("Go metadata returned malformed release version")
}
