use super::github::Release;
use crate::operations::{host, packages::apt};
use crate::platform::Arch;
use anyhow::{Context, Result, bail};

const RELEASE_API: &str = "https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous";

pub(crate) fn install(arch: Arch) -> Result<()> {
    ensure_fuse()?;
    if !host::output("systemctl", ["--user", "--quiet", "is-active", "appimaged.service"])?.status.success() {
        // https://github.com/probonopd/go-appimage/blob/master/src/appimaged/README.md#initial-setup
        // cleanup is idempotent; an absent service or package is already the desired state
        let _ = host::output("systemctl", ["--user", "stop", "appimaged.service"]);
        let _ = host::output("sudo", ["apt-get", "-y", "purge", "appimagelauncher"]);

        let home = host::home()?;
        let service = home.join(".config/systemd/user/default.target.wants/appimagelauncherd.service");
        host::run("remove conflicting appimaged service", "rm", ["-f".as_ref(), service.as_os_str()])?;
        host::run("reload user services", "systemctl", ["--user", "daemon-reload"])?;
        let cache = home.join(".local/share/applications");
        // pass the cache path as $1 so the shell never parses user-controlled path contents
        host::run(
            "clear AppImage cache",
            "sh",
            ["-c".as_ref(), r#"rm -f -- "$1"/appimage*"#.as_ref(), "sh".as_ref(), cache.as_os_str()],
        )?;

        let applications = home.join("Applications");
        let destination = applications.join("appimaged.AppImage");
        super::appimage::install_appimage("download appimaged", &resolve_asset_url(arch)?, &destination)?;
        host::run(
            "launch appimaged",
            destination.to_str().with_context(|| format!("appimaged path is not UTF-8: {}", destination.display()))?,
            std::iter::empty::<&str>(),
        )?;
    }

    Ok(())
}

fn resolve_asset_url(arch: Arch) -> Result<String> {
    let output = host::curl("resolve appimaged release", RELEASE_API, std::iter::empty::<&str>())?;
    let release: Release = serde_json::from_slice(&output.stdout).context("parse appimaged release JSON")?;
    let suffix = match arch {
        Arch::X86_64 => "-x86_64.AppImage",
        Arch::Aarch64 => "-aarch64.AppImage",
    };
    for asset in release.assets {
        if asset.name.starts_with("appimaged-") && asset.name.ends_with(suffix) {
            return Ok(asset.browser_download_url);
        }
    }
    bail!("appimaged release has no asset for {}", arch.as_str())
}

fn ensure_fuse() -> Result<()> {
    // newer releases use libfuse2t64 while older releases retain libfuse2
    let output = host::output("apt-cache", ["show", "libfuse2t64"])?;
    let package = if output.status.success() { "libfuse2t64" } else { "libfuse2" };
    apt::install(&[package.into()])
}
