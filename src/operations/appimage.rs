use anyhow::{Context, Result};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use super::{Host, TempPath};

pub(super) fn install_appimage(host: &Host, label: &str, url: &str, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .with_context(|| format!("AppImage destination has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent).context("create AppImage destination directory")?;
    let temp = TempPath::new_in_with_suffix(parent, ".appimage-", ".part")?;
    host.curl(label, url, ["--output".as_ref(), temp.path().as_os_str()])?;
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).context("make AppImage executable")?;
    fs::rename(temp.path(), destination).with_context(|| format!("install AppImage at {}", destination.display()))
}
