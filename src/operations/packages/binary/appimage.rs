use anyhow::{Context, Result};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use crate::operations::host::{self, temp_path_in_with_suffix};

pub(super) fn install_appimage(label: &str, url: &str, destination: &Path) -> Result<()> {
    let parent = destination.parent();
    let parent = parent.with_context(|| format!("AppImage destination has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent).context("create AppImage destination directory")?;
    let temp = temp_path_in_with_suffix(parent, ".appimage-", ".part")?;
    host::curl(label, url, ["--output".as_ref(), temp.as_os_str()])?;
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o755)).context("make AppImage executable")?;
    fs::rename(&temp, destination).with_context(|| format!("install AppImage at {}", destination.display()))
}
