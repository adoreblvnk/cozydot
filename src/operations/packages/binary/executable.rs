use crate::config::BinaryPackage;
use crate::operations::host;
use anyhow::{Context, Result};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub(super) fn path(package: &BinaryPackage) -> Result<PathBuf> {
    Ok(host::home()?.join(".local/bin").join(&package.name))
}

pub(super) fn install(label: &str, url: &str, destination: &Path) -> Result<()> {
    let parent = destination.parent();
    let parent = parent.with_context(|| format!("executable destination has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent).context("create executable destination directory")?;
    let temp = tempfile::NamedTempFile::new_in(parent).context("create temporary executable")?;
    host::curl(label, url, ["--output".as_ref(), temp.path().as_os_str()])?;
    temp.as_file().set_permissions(fs::Permissions::from_mode(0o755)).context("make executable executable")?;
    temp.persist(destination).with_context(|| format!("install executable at {}", destination.display()))?;
    Ok(())
}
