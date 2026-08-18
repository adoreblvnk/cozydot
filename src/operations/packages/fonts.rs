use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, fs, path::Path};

use super::super::{Host, TempPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NerdFontsMode {
    Install,
    Update,
}

pub(crate) fn apply(host: &Host, families: &[String], mode: NerdFontsMode) -> Result<()> {
    let parent = host.home().join(if cfg!(target_os = "macos") { "Library/Fonts" } else { ".local/share/fonts" });
    fs::create_dir_all(&parent).context("create user font directory")?;
    if !fs::symlink_metadata(&parent)?.file_type().is_dir() {
        bail!("user font path is not a real directory: {}", parent.display());
    }
    let mut changed = false;
    for family in families {
        let destination = parent.join(family);
        let is_family_installed = match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.is_dir() => true,
            Ok(_) => bail!("Nerd Font destination conflict at {}", destination.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).context(format!("read Nerd Font destination metadata {}", destination.display()));
            }
        };
        if mode == NerdFontsMode::Update || !is_family_installed {
            install(host, family, &destination)?;
            changed = true;
        }
    }
    if changed && !cfg!(target_os = "macos") {
        host.run("Nerd Font cache refresh", "fc-cache", [OsStr::new("--force"), parent.as_os_str()])?;
    }
    Ok(())
}

fn install(host: &Host, family: &str, destination: &Path) -> Result<()> {
    let archive = TempPath::new_with_suffix(host, "nerd-font", ".tar.xz")?;
    let url = format!("https://github.com/ryanoasis/nerd-fonts/releases/latest/download/{family}.tar.xz");
    host.curl(
        "Nerd Font archive download",
        &url,
        ["--proto".as_ref(), "=https".as_ref(), "--output".as_ref(), archive.path().as_os_str()],
    )?;
    let path = destination.to_str().context("font path is not UTF-8")?;
    let archive_path = archive.path().to_str().context("font archive path is not UTF-8")?;
    host.run("Nerd Font destination replacement", "rm", ["-rf", path])?;
    host.run("Nerd Font destination creation", "mkdir", ["-p", path])?;
    host.run("Nerd Font archive extraction", "tar", ["-xJf", archive_path, "-C", path])?;
    Ok(())
}
