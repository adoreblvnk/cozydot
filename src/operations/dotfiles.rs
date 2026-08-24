use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::host;

pub(crate) fn apply(stow_dir: &Path, packages: &[String], replace: bool) -> Result<()> {
    let stow_dir =
        fs::canonicalize(stow_dir).with_context(|| format!("canonicalize stow directory {}", stow_dir.display()))?;
    if !fs::symlink_metadata(&stow_dir)?.file_type().is_dir() {
        bail!("stow directory is not a directory: {}", stow_dir.display());
    }

    let mut package_dirs = Vec::with_capacity(packages.len());
    for package in packages {
        let package_dir = stow_dir.join(package);
        let metadata = fs::symlink_metadata(&package_dir)
            .with_context(|| format!("dotfiles package {package:?} does not exist"))?;
        ensure!(metadata.file_type().is_dir(), "dotfiles package {package:?} is not a real directory");
        package_dirs.push((package, package_dir));
    }

    host::require_cli("GNU Stow", "stow").context("dotfiles require GNU Stow")?;
    let home = host::home()?;
    let mut args = vec![OsStr::new("--dir"), stow_dir.as_os_str(), OsStr::new("--target"), home.as_os_str()];
    if !replace {
        // simulate first so conflicts fail before Stow mutates the home directory
        let mut check_args = args.clone();
        check_args.extend([OsStr::new("--simulate"), OsStr::new("--")]);
        check_args.extend(packages.iter().map(OsStr::new));
        host::run("stow package check", "stow", check_args)?;
    }

    let mut conflicts = Vec::new();
    if replace {
        for (package, package_dir) in &package_dirs {
            collect_conflicts(package_dir, home.clone(), package, &mut conflicts)?;
        }
        // back up each target once when multiple Stow packages claim it
        conflicts.sort_by(|left, right| left.1.cmp(&right.1));
        conflicts.dedup_by(|left, right| left.1 == right.1);
        backup_conflicts(&home, &conflicts)?;
    }

    if packages.iter().any(|package| package == "gnupg") {
        // keep GnuPG state outside the repository by preventing Stow from folding this directory
        let target = home.join(".gnupg");
        fs::create_dir_all(&target).context("create GnuPG home")?;
        // require ~/.gnupg to be a non-symlink dir
        if !fs::symlink_metadata(&target)?.file_type().is_dir() {
            bail!("GnuPG home is not a real directory: {}", target.display());
        }
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).context("secure GnuPG home")?;
    }
    args.push(OsStr::new("--"));
    args.extend(packages.iter().map(OsStr::new));
    host::run("stow package install", "stow", args)?;
    Ok(())
}

fn collect_conflicts(
    source: &Path,
    target: PathBuf,
    package: &str,
    conflicts: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    let source_metadata =
        fs::symlink_metadata(source).with_context(|| format!("read dotfiles source metadata {}", source.display()))?;
    if source_metadata.file_type().is_dir() {
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
                entries.sort_by_key(std::fs::DirEntry::file_name);
                for entry in entries {
                    collect_conflicts(&entry.path(), target.join(entry.file_name()), package, conflicts)?;
                }
            }
            Ok(_) if !resolves_to(&target, source) => conflicts.push((package.to_owned(), target)),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("read dotfile destination metadata"),
        }
    } else if source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
        match fs::symlink_metadata(&target) {
            Ok(_) if !resolves_to(&target, source) => conflicts.push((package.to_owned(), target)),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("read dotfile destination metadata"),
        }
    } else {
        bail!("unsupported dotfiles source type at {}", source.display());
    }
    Ok(())
}

fn resolves_to(target: &Path, source: &Path) -> bool {
    let Ok(target) = fs::canonicalize(target) else {
        return false;
    };
    let Ok(source) = fs::canonicalize(source) else {
        return false;
    };
    target == source
}

fn backup_conflicts(home: &Path, conflicts: &[(String, PathBuf)]) -> Result<()> {
    if conflicts.is_empty() {
        return Ok(());
    }
    let state_home = crate::paths::state_home()?;
    let timestamp_context = "dotfiles backup timestamp is before the Unix epoch";
    let now = SystemTime::now();
    let elapsed = now.duration_since(UNIX_EPOCH).context(timestamp_context)?;
    let timestamp = elapsed.as_nanos();
    let backup_root = state_home.join("cozydot/dotfile-backups").join(format!("{timestamp}-{}", std::process::id()));
    for (package, conflict) in conflicts {
        let context = || format!("dotfiles conflict escaped the home directory: {}", conflict.display());
        let relative = conflict.strip_prefix(home).with_context(context)?;
        let backup = backup_root.join(package).join(relative);
        let parent = backup.parent().context("dotfiles backup has no parent")?;
        fs::create_dir_all(parent).context("create dotfiles backup directory")?;
        // rename makes each backup atomic, requiring HOME & XDG_STATE_HOME on same filesystem
        fs::rename(conflict, &backup)
            .with_context(|| format!("move dotfiles conflict {} to {}", conflict.display(), backup.display()))?;
    }
    Ok(())
}
