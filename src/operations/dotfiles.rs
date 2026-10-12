use anyhow::{Context, Result, bail, ensure};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, path::PathBuf};

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
    let stow_path = stow_dir.to_str().context("stow directory path is not UTF-8")?;
    let home_path = home.to_str().context("home directory path is not UTF-8")?;
    let mut args = vec!["--dir", stow_path, "--target", home_path];
    if !replace {
        // simulate first so conflicts fail before Stow mutates the home directory
        let mut check_args = args.clone();
        check_args.push("--simulate");
        check_args.extend(packages.iter().map(String::as_str));
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
    args.extend(packages.iter().map(String::as_str));
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
    if source_metadata.file_type().is_dir()
        && let Ok(metadata) = fs::symlink_metadata(&target)
        && metadata.file_type().is_dir()
    {
        let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            collect_conflicts(&entry.path(), target.join(entry.file_name()), package, conflicts)?;
        }
        return Ok(());
    }
    ensure!(
        source_metadata.file_type().is_dir()
            || source_metadata.file_type().is_file()
            || source_metadata.file_type().is_symlink(),
        "unsupported dotfiles source type at {}",
        source.display()
    );
    match fs::symlink_metadata(&target) {
        Ok(_) if !same_file::is_same_file(&target, source).unwrap_or(false) => {
            conflicts.push((package.to_owned(), target))
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("read dotfile destination metadata"),
    }
    Ok(())
}

fn backup_conflicts(home: &Path, conflicts: &[(String, PathBuf)]) -> Result<()> {
    if conflicts.is_empty() {
        return Ok(());
    }
    let state_home = crate::paths::state_home()?;
    let timestamp_context = "dotfiles backup timestamp is before the Unix epoch";
    let now = std::time::SystemTime::now();
    let elapsed = now.duration_since(std::time::UNIX_EPOCH).context(timestamp_context)?;
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

pub(crate) fn unlink(stow_dir: &Path, packages: &[String]) -> Result<()> {
    let stow_dir =
        fs::canonicalize(stow_dir).with_context(|| format!("canonicalize stow directory {}", stow_dir.display()))?;
    if !fs::symlink_metadata(&stow_dir)?.file_type().is_dir() {
        bail!("stow directory is not a directory: {}", stow_dir.display());
    }
    let home = host::home()?;
    for package in packages {
        let package_dir = stow_dir.join(package);
        if fs::symlink_metadata(&package_dir).is_ok_and(|metadata| metadata.file_type().is_dir()) {
            materialize_tree(&package_dir, &home, false)?;
        }
    }
    Ok(())
}

// traverse package tree & unfold Stow directory symlinks or match leaf symlinks
fn materialize_tree(source: &Path, target: &Path, unfolded: bool) -> Result<()> {
    let source_meta =
        fs::symlink_metadata(source).with_context(|| format!("read dotfiles source metadata {}", source.display()))?;
    if source_meta.file_type().is_dir() {
        let is_folded = fs::symlink_metadata(target)
            .is_ok_and(|m| m.file_type().is_symlink() && same_file::is_same_file(target, source).unwrap_or(false));
        if is_folded {
            fs::remove_file(target)?;
        }
        if unfolded || is_folded {
            fs::create_dir_all(target)?;
            fs::set_permissions(target, source_meta.permissions())?;
        }
        if unfolded || is_folded || fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_dir()) {
            let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(fs::DirEntry::file_name);
            for entry in entries {
                materialize_tree(&entry.path(), &target.join(entry.file_name()), unfolded || is_folded)?;
            }
        }
        return Ok(());
    }
    let should_materialize = unfolded
        || fs::symlink_metadata(target)
            .is_ok_and(|m| m.file_type().is_symlink() && same_file::is_same_file(target, source).unwrap_or(false));
    if should_materialize {
        materialize_file(source, target)?;
    }
    Ok(())
}

// atomically replace symlink with standalone copy preserving mode via fs::copy
fn materialize_file(source: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().context("dotfile target has no parent")?;
    let temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create staging file beside {}", target.display()))?;
    fs::copy(source, temp.path()).with_context(|| format!("copy dotfile to {}", target.display()))?;
    temp.persist(target).with_context(|| format!("materialize dotfile at {}", target.display()))?;
    Ok(())
}
