use anyhow::{Context, Result, bail};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::super::Host;

pub(crate) fn apply(host: &Host, root: &Path, packages: &[String], replace: bool) -> Result<()> {
    let root =
        fs::canonicalize(root).with_context(|| format!("dotfiles operation: canonicalize root {}", root.display()))?;
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

    let mut conflicts = Vec::new();
    for (package, source) in &sources {
        collect_conflicts(source, host.home(), package, &mut conflicts)?;
    }
    conflicts.sort_by(|left, right| left.1.cmp(&right.1));
    conflicts.dedup_by(|left, right| left.1 == right.1);
    if !conflicts.is_empty() && !replace {
        let paths = conflicts.iter().map(|(_, path)| format!("  {}", path.display())).collect::<Vec<_>>().join("\n");
        bail!(
            "unmanaged dotfile conflicts:\n{paths}\nno dotfiles were changed; rerun with `cozydot dotfiles --replace`"
        );
    }
    host.require("Stow preflight", "stow", ["--version"]).context("dotfiles require GNU Stow")?;
    if replace {
        backup_conflicts(host, &conflicts)?;
    }

    for (package, source) in sources {
        apply_package(host, &root, package, &source)?;
    }
    Ok(())
}

fn apply_package(host: &Host, root: &Path, package: &str, source: &Path) -> Result<()> {
    prepare_gnupg_home(source, &host.home())?;
    host.require(
        "stow package install",
        "stow",
        [
            "--dir".as_ref(),
            root.as_os_str(),
            "--target".as_ref(),
            host.home().as_os_str(),
            "--stow".as_ref(),
            "--".as_ref(),
            package.as_ref(),
        ],
    )?;
    Ok(())
}

fn prepare_gnupg_home(source: &Path, home: &Path) -> Result<()> {
    let source = source.join(".gnupg");
    if !source.is_dir() {
        return Ok(());
    }

    // Prevent Stow's tree folding from making ~/.gnupg a symlink; keep this security-sensitive directory real and mode 0700.
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
        fs::symlink_metadata(source).with_context(|| format!("inspect dotfiles source {}", source.display()))?;
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
            Err(error) => return Err(error).context("inspect dotfile destination"),
        }
    } else if source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
        match fs::symlink_metadata(&target) {
            Ok(_) if !resolves_to(&target, source) => conflicts.push((package.to_owned(), target)),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect dotfile destination"),
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
        host.value("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/state"));
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
        // Rename makes each backup all-or-nothing and therefore requires HOME and XDG_STATE_HOME on one filesystem.
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
