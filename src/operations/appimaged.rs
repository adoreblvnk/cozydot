use super::Host;
use super::parsers::GitHubRelease;
use crate::platform::Architecture;
use anyhow::{Context, Result};

const RELEASE_API: &str = "https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous";

pub(crate) fn install(host: &Host, architecture: Architecture) -> Result<()> {
    if !host.output("systemctl", ["--user", "--quiet", "is-active", "appimaged.service"])?.status.success() {
        // https://github.com/probonopd/go-appimage/blob/master/src/appimaged/README.md#initial-setup
        let _ = host.output("systemctl", ["--user", "stop", "appimaged.service"]);
        let _ = host.output("sudo", ["apt-get", "-y", "purge", "appimagelauncher"]);

        let home = host.home();
        let service = home.join(".config/systemd/user/default.target.wants/appimagelauncherd.service");
        host.run("remove conflicting appimaged service", "rm", ["-f".as_ref(), service.as_os_str()])?;
        host.run("reload user services", "systemctl", ["--user", "daemon-reload"])?;
        let cache = home.join(".local/share/applications");
        host.run(
            "clear AppImage cache",
            "sh",
            ["-c".as_ref(), r#"rm -f -- "$1"/appimage*"#.as_ref(), "sh".as_ref(), cache.as_os_str()],
        )?;

        let applications = home.join("Applications");
        let destination = applications.join("appimaged.AppImage");
        let url = resolve_asset_url(host, architecture)?;
        super::appimage::install_appimage(host, "download appimaged", &url, &destination)?;
        host.run(
            "launch appimaged",
            destination.to_str().with_context(|| format!("appimaged path is not UTF-8: {}", destination.display()))?,
            std::iter::empty::<&str>(),
        )?;
    }

    ensure_fuse(host)
}

fn resolve_asset_url(host: &Host, architecture: Architecture) -> Result<String> {
    let output = host.curl("resolve appimaged release", RELEASE_API, std::iter::empty::<&str>())?;
    let release: GitHubRelease = serde_json::from_slice(&output.stdout).context("parse appimaged release JSON")?;
    let suffix = match architecture {
        Architecture::X86_64 => "-x86_64.AppImage",
        Architecture::Aarch64 => "-aarch64.AppImage",
        Architecture::Arm => "-armhf.AppImage",
    };
    release
        .assets
        .into_iter()
        .find(|asset| asset.name.starts_with("appimaged-") && asset.name.ends_with(suffix))
        .map(|asset| asset.browser_download_url)
        .with_context(|| format!("appimaged release has no asset for {}", architecture.canonical()))
}

fn ensure_fuse(host: &Host) -> Result<()> {
    let package =
        if host.output("apt-cache", ["show", "libfuse2t64"])?.status.success() { "libfuse2t64" } else { "libfuse2" };
    if !host.output("dpkg", ["--status", package])?.status.success() {
        host.run("AppImage FUSE support install", "sudo", ["apt-get", "install", "-qq", package])?;
    }
    Ok(())
}
