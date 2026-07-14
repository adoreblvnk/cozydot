use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::Host;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DotfilesOperation {
    root: PathBuf,
    packages: Vec<String>,
}

impl DotfilesOperation {
    pub fn new(root: PathBuf, packages: Vec<String>) -> Result<Self> {
        validate_packages(&packages)?;
        if root.as_os_str().is_empty() {
            bail!("dotfiles root must not be empty");
        }
        Ok(Self { root, packages })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        std::iter::once("dotfiles-backup-stow".into())
            .chain(self.packages.iter().cloned())
            .collect()
    }
}

pub(crate) fn execute(host: &Host<'_>, operation: &DotfilesOperation) -> Result<()> {
    validate_packages(&operation.packages).context("validate dotfiles operation")?;
    let root = fs::canonicalize(&operation.root).with_context(|| {
        format!(
            "dotfiles operation: canonicalize root {}",
            operation.root.display()
        )
    })?;
    if !fs::symlink_metadata(&root)?.file_type().is_dir() {
        bail!("dotfiles root is not a directory: {}", root.display());
    }
    for package in &operation.packages {
        apply_package(host, &root, package)?;
    }
    Ok(())
}

fn apply_package(host: &Host<'_>, root: &Path, package: &str) -> Result<()> {
    let source = root.join(package);
    let metadata = fs::symlink_metadata(&source)
        .with_context(|| format!("dotfiles package {package:?} does not exist"))?;
    if !metadata.file_type().is_dir() {
        bail!("dotfiles package {package:?} is not a real directory");
    }
    let mut conflicts = Vec::new();
    collect_conflicts(&source, host.home(), &mut conflicts)?;
    if !conflicts.is_empty() {
        backup_conflicts(host, package, &conflicts)?;
    }
    host.require(
        "dotfiles Stow mutation",
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
    verify_tree(&source, host.home())
        .with_context(|| format!("dotfiles package {package:?} postcondition"))
}

fn collect_conflicts(source: &Path, target: PathBuf, conflicts: &mut Vec<PathBuf>) -> Result<()> {
    let source_metadata = fs::symlink_metadata(source)
        .with_context(|| format!("inspect dotfiles source {}", source.display()))?;
    if source_metadata.file_type().is_dir() {
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
                entries.sort_by_key(|entry| entry.file_name());
                for entry in entries {
                    collect_conflicts(&entry.path(), target.join(entry.file_name()), conflicts)?;
                }
            }
            Ok(_) if !resolves_to(&target, source) => conflicts.push(target),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect dotfiles target"),
        }
    } else if source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
        match fs::symlink_metadata(&target) {
            Ok(_) if !resolves_to(&target, source) => conflicts.push(target),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect dotfiles target"),
        }
    } else {
        bail!("unsupported dotfiles source type at {}", source.display());
    }
    Ok(())
}

fn backup_conflicts(host: &Host<'_>, package: &str, conflicts: &[PathBuf]) -> Result<()> {
    let state_home = host
        .value("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/state"));
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("dotfiles backup timestamp is before the Unix epoch")?
        .as_nanos();
    let backup_root = state_home
        .join("cozydot/dotfile-backups")
        .join(format!("{timestamp}-{}", std::process::id()))
        .join(package);
    for conflict in conflicts {
        let relative = conflict.strip_prefix(host.home()).with_context(|| {
            format!(
                "dotfiles conflict escaped the home directory: {}",
                conflict.display()
            )
        })?;
        let backup = backup_root.join(relative);
        let parent = backup.parent().context("dotfiles backup has no parent")?;
        fs::create_dir_all(parent).context("create dotfiles backup directory")?;
        host.require(
            "dotfiles conflict backup",
            "mv",
            [
                "--no-clobber".as_ref(),
                "--".as_ref(),
                conflict.as_os_str(),
                backup.as_os_str(),
            ],
        )?;
        if fs::symlink_metadata(conflict).is_ok() || fs::symlink_metadata(&backup).is_err() {
            bail!(
                "dotfiles conflict backup did not move {} to {}",
                conflict.display(),
                backup.display()
            );
        }
    }
    Ok(())
}

fn verify_tree(source: &Path, target: PathBuf) -> Result<()> {
    let source_metadata = fs::symlink_metadata(source)?;
    if source_metadata.file_type().is_dir() {
        let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            verify_tree(&entry.path(), target.join(entry.file_name()))?;
        }
    } else if !resolves_to(&target, source) {
        bail!(
            "Stow did not link {} to {}",
            target.display(),
            source.display()
        );
    }
    Ok(())
}

fn resolves_to(target: &Path, source: &Path) -> bool {
    fs::canonicalize(target)
        .and_then(|target| fs::canonicalize(source).map(|source| target == source))
        .unwrap_or(false)
}

fn validate_packages(packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        bail!("dotfiles package sequence must not be empty");
    }
    let mut seen = BTreeSet::new();
    for package in packages {
        let mut components = Path::new(package).components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || package.contains(['\n', '\r'])
        {
            bail!("invalid dotfiles package directory name {package:?}");
        }
        if !seen.insert(package) {
            bail!("duplicate dotfiles package {package:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_are_single_unique_directory_components() {
        assert!(DotfilesOperation::new(PathBuf::from("/dotfiles"), vec!["bash".into()]).is_ok());
        for packages in [
            vec![],
            vec!["../bash".into()],
            vec!["nested/bash".into()],
            vec!["bash".into(), "bash".into()],
        ] {
            assert!(DotfilesOperation::new(PathBuf::from("/dotfiles"), packages).is_err());
        }
    }
}
