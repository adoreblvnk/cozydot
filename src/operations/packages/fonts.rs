use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, fs, path::Path};
use url::Url;

use super::super::{Host, TempPath};

const FONT_ROOT: &str = "/usr/share/fonts";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NerdFontsMode {
    Install,
    Update,
}

pub(crate) fn apply(host: &Host, families: &[String], mode: NerdFontsMode) -> Result<()> {
    apply_at(host, families, mode, Path::new(FONT_ROOT), true)
}

pub(crate) fn apply_user(host: &Host, families: &[String], mode: NerdFontsMode) -> Result<()> {
    let parent = host.home().join("Library/Fonts");
    fs::create_dir_all(&parent).context("create user font directory")?;
    apply_at(host, families, mode, &parent, false)
}

fn apply_at(host: &Host, families: &[String], mode: NerdFontsMode, parent: &Path, privileged: bool) -> Result<()> {
    let mut changed = false;
    for family in families {
        let destination = parent.join(family);
        let is_present = match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.is_dir() => true,
            Ok(_) => bail!("Nerd Font destination conflict at {}", destination.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).context(format!("inspect Nerd Font destination {}", destination.display()));
            }
        };
        if mode == NerdFontsMode::Update || !is_present {
            install_family(host, family, &destination, privileged)?;
            changed = true;
        }
    }
    if changed && privileged {
        host.require(
            "Nerd Font cache refresh",
            "sudo",
            [OsStr::new("fc-cache"), OsStr::new("--force"), parent.as_os_str()],
        )?;
    }
    Ok(())
}

fn install_family(host: &Host, family: &str, destination: &Path, privileged: bool) -> Result<()> {
    let archive = TempPath::new_with_suffix(host, "nerd-font", ".tar.xz")?;
    let mut url = Url::parse("https://github.com/ryanoasis/nerd-fonts/releases/latest/download/placeholder.tar.xz")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Nerd Fonts URL cannot be a base"))?
        .pop()
        .push(&format!("{family}.tar.xz"));
    host.require(
        "Nerd Font archive download",
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
            url.as_str().as_ref(),
        ],
    )?;
    let path = destination.to_str().context("font path is not UTF-8")?;
    let archive_path = archive.path().to_str().context("font archive path is not UTF-8")?;
    if privileged {
        host.require("Nerd Font destination replacement", "sudo", ["rm", "--recursive", "--force", "--", path])?;
        host.require("Nerd Font destination creation", "sudo", ["mkdir", "--parents", "--", path])?;
        host.require(
            "Nerd Font archive extraction",
            "sudo",
            ["tar", "--extract", "--xz", "--directory", path, "--file", archive_path],
        )?;
    } else {
        host.require("Nerd Font destination replacement", "rm", ["-rf", path])?;
        host.require("Nerd Font destination creation", "mkdir", ["-p", path])?;
        host.require("Nerd Font archive extraction", "tar", ["-xJf", archive_path, "-C", path])?;
    }
    Ok(())
}
