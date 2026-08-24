use anyhow::{Context, Result};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use crate::operations::host;

pub(super) fn install_appimage(label: &str, url: &str, destination: &Path) -> Result<()> {
    let parent = destination.parent();
    let parent = parent.with_context(|| format!("AppImage destination has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent).context("create AppImage destination directory")?;
    // keep incomplete downloads hidden until permissions & contents are ready
    let temp = tempfile::NamedTempFile::new_in(parent).context("create temporary AppImage")?;
    host::curl(label, url, ["--output".as_ref(), temp.path().as_os_str()])?;
    temp.as_file().set_permissions(fs::Permissions::from_mode(0o755)).context("make AppImage executable")?;
    temp.persist(destination).with_context(|| format!("install AppImage at {}", destination.display()))?;
    Ok(())
}
