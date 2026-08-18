use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::host::Host;

pub(crate) fn apply(host: &Host, root: &Path, packages: &[String], replace: bool) -> Result<()> {
    let root = fs::canonicalize(root).with_context(|| format!("canonicalize dotfiles root {}", root.display()))?;
    if !fs::symlink_metadata(&root)?.file_type().is_dir() {
        bail!("dotfiles root is not a directory: {}", root.display());
    }

    let mut sources = Vec::with_capacity(packages.len());
    for package in packages {
        let source = root.join(package);
        let metadata =
            fs::symlink_metadata(&source).with_context(|| format!("dotfiles package {package:?} does not exist"))?;
        if !metadata.file_type().is_dir() {
            bail!("dotfiles package {package:?} is not a real directory");
        }
        sources.push((package, source));
    }

    host.require_cli("GNU Stow", "stow").context("dotfiles require GNU Stow")?;
    let home = host.home();
    let mut args = vec![OsStr::new("--dir"), root.as_os_str(), OsStr::new("--target"), home.as_os_str()];
    if !replace {
        let mut check_args = args.clone();
        check_args.extend([OsStr::new("--simulate"), OsStr::new("--")]);
        check_args.extend(packages.iter().map(OsStr::new));
        host.run("stow package check", "stow", check_args)?;
    }

    let mut conflicts = Vec::new();
    if replace {
        for (package, source) in &sources {
            collect_conflicts(source, home.clone(), package, &mut conflicts)?;
        }
        conflicts.sort_by(|left, right| left.1.cmp(&right.1));
        conflicts.dedup_by(|left, right| left.1 == right.1);
        backup_conflicts(host, &conflicts)?;
    }

    if sources.iter().any(|(_, source)| source.join(".gnupg").is_dir()) {
        prepare_gnupg_home(&home)?;
    }
    args.push(OsStr::new("--"));
    args.extend(packages.iter().map(OsStr::new));
    host.run("stow package install", "stow", args)?;
    Ok(())
}

fn prepare_gnupg_home(home: &Path) -> Result<()> {
    // keep ~/.gnupg as real 0700 dir instead of letting Stow fold it into symlink
    let target = home.join(".gnupg");
    if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        fs::remove_file(&target).context("replace folded GnuPG dotfiles directory")?;
    }
    fs::create_dir_all(&target).context("create GnuPG home")?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).context("secure GnuPG home")?;
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

fn backup_conflicts(host: &Host, conflicts: &[(String, PathBuf)]) -> Result<()> {
    if conflicts.is_empty() {
        return Ok(());
    }
    let state_home =
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/state"));
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("dotfiles backup timestamp is before the Unix epoch")?
        .as_nanos();
    let backup_root = state_home.join("cozydot/dotfile-backups").join(format!("{timestamp}-{}", std::process::id()));
    for (package, conflict) in conflicts {
        let relative = conflict
            .strip_prefix(host.home())
            .with_context(|| format!("dotfiles conflict escaped the home directory: {}", conflict.display()))?;
        let backup = backup_root.join(package).join(relative);
        let parent = backup.parent().context("dotfiles backup has no parent")?;
        fs::create_dir_all(parent).context("create dotfiles backup directory")?;
        // rename makes each backup atomic, requiring HOME & XDG_STATE_HOME on same filesystem
        fs::rename(conflict, &backup)
            .with_context(|| format!("move dotfiles conflict {} to {}", conflict.display(), backup.display()))?;
        if fs::symlink_metadata(conflict).is_ok() || fs::symlink_metadata(&backup).is_err() {
            bail!("dotfiles conflict backup did not move {} to {}", conflict.display(), backup.display());
        }
    }
    Ok(())
}

fn resolves_to(target: &Path, source: &Path) -> bool {
    fs::canonicalize(target).and_then(|target| fs::canonicalize(source).map(|source| target == source)).unwrap_or(false)
}
