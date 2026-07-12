use crate::assets::{hash_file, Assets};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, File},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

pub fn run() -> Result<PathBuf> {
    let assets = Assets::discover()?;
    let root = config_root()?;
    sync(&root, &assets.config(), &assets.dotfiles())?;
    Ok(root)
}

fn sync(root: &Path, config: &Path, dotfiles: &Path) -> Result<()> {
    ensure_directory_path(root, Path::new(""))?;
    let manifest_path = root.join(".managed-files");
    let pending_path = root.join(".managed-files.pending");
    let mut managed = read_manifest(&manifest_path)?;
    recover_pending(root, &pending_path, &mut managed)?;

    let mut sources = vec![(config.to_path_buf(), PathBuf::from("cozydot.yaml"))];
    collect_files(dotfiles, Path::new("dotfiles"), &mut sources)?;
    sources.sort_by(|a, b| a.1.cmp(&b.1));
    let mut installs = 0usize;
    for (source, relative) in sources {
        validate_relative(&relative)?;
        let destination = root.join(&relative);
        let new_hash = hash_file(&source)?;
        let old_hash = managed.get(&relative).cloned();
        let install = match fs::symlink_metadata(&destination) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => true,
            Err(e) => return Err(e.into()),
            Ok(metadata) if !metadata.file_type().is_file() => false,
            Ok(_) => old_hash
                .as_ref()
                .is_some_and(|hash| hash_file(&destination).ok().as_ref() == Some(hash)),
        };
        if !install {
            continue;
        }
        append_pending(&pending_path, old_hash.as_deref(), &new_hash, &relative)?;
        install_file(root, &source, &relative)?;
        managed.insert(relative.clone(), new_hash);
        installs += 1;
        if env::var_os("COZYDOT_TEST_FAIL_AFTER_RELATIVE").as_deref() == Some(relative.as_os_str())
            || env::var("COZYDOT_TEST_FAIL_AFTER_INSTALLS").ok().as_deref()
                == Some(&installs.to_string())
        {
            bail!("injected init failure");
        }
    }
    write_manifest(&manifest_path, &managed)?;
    remove_if_exists(&pending_path)?;
    Ok(())
}

pub fn config_root() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("cozydot"));
    }
    Ok(PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".config/cozydot"))
}

fn collect_files(dir: &Path, relative: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        let child = relative.join(entry.file_name());
        if kind.is_dir() {
            collect_files(&entry.path(), &child, out)?;
        } else if kind.is_file() {
            out.push((entry.path(), child));
        } else {
            bail!("bundled asset is not a regular file: {}", child.display());
        }
    }
    Ok(())
}

fn install_file(root: &Path, source: &Path, relative: &Path) -> Result<()> {
    let parent = relative.parent().unwrap_or(Path::new(""));
    ensure_directory_path(root, parent)?;
    let destination = root.join(relative);
    let mut temporary = tempfile::Builder::new()
        .prefix(".cozydot.")
        .tempfile_in(destination.parent().unwrap())?;
    if env::var("COZYDOT_TEST_FAIL_MANAGED_FILE_AT")
        .ok()
        .as_deref()
        == Some("cp")
    {
        bail!("injected copy failure");
    }
    io::copy(&mut File::open(source)?, &mut temporary)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .as_file_mut()
        .set_permissions(fs::metadata(source)?.permissions())?;
    if env::var("COZYDOT_TEST_FAIL_MANAGED_FILE_AT")
        .ok()
        .as_deref()
        == Some("signal")
    {
        bail!("injected signal failure");
    }
    if env::var("COZYDOT_TEST_FAIL_MANAGED_FILE_AT")
        .ok()
        .as_deref()
        == Some("mv")
    {
        bail!("injected rename failure");
    }
    temporary.persist(&destination).map_err(|e| e.error)?;
    sync_directory(destination.parent().unwrap())?;
    Ok(())
}

fn ensure_directory_path(root: &Path, relative: &Path) -> Result<()> {
    if root.exists() && fs::symlink_metadata(root)?.file_type().is_symlink() {
        bail!("configuration root is a symlink");
    }
    fs::create_dir_all(root)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            bail!("unsafe destination path");
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                bail!("refusing symlinked config path: {}", current.display())
            }
            Ok(meta) if !meta.is_dir() => {
                bail!("refusing non-directory config path: {}", current.display())
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn read_manifest(path: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let mut result = BTreeMap::new();
    let text = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(result),
        Err(e) => return Err(e.into()),
    };
    for line in text.lines() {
        let (hash, relative) = line
            .split_once('\t')
            .context("malformed managed-files record")?;
        let relative = PathBuf::from(relative);
        validate_hash(hash)?;
        validate_relative(&relative)?;
        if result.insert(relative, hash.into()).is_some() {
            bail!("duplicate managed-files record");
        }
    }
    Ok(result)
}

fn recover_pending(
    root: &Path,
    path: &Path,
    managed: &mut BTreeMap<PathBuf, String>,
) -> Result<()> {
    let text = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    validate_pending(&text)?;
    for line in text.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        let relative = PathBuf::from(fields[2]);
        let current = hash_file(&root.join(&relative)).ok();
        if current.as_deref() == Some(fields[1]) {
            managed.insert(relative, fields[1].into());
        } else if fields[0] != "-" && current.as_deref() == Some(fields[0]) {
            managed.insert(relative, fields[0].into());
        } else {
            managed.remove(&relative);
        }
    }
    Ok(())
}

fn append_pending(path: &Path, old: Option<&str>, new: &str, relative: &Path) -> Result<()> {
    append_pending_with_failure(path, old, new, relative, None)
}

fn append_pending_with_failure(
    path: &Path,
    old: Option<&str>,
    new: &str,
    relative: &Path,
    failure: Option<&str>,
) -> Result<()> {
    validate_relative(relative)?;
    validate_hash(new)?;
    if let Some(old) = old {
        validate_hash(old)?;
    }
    let mut records = match fs::read_to_string(path) {
        Ok(text) => {
            validate_pending(&text)?;
            text
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    records.push_str(&format!(
        "{}\t{}\t{}\n",
        old.unwrap_or("-"),
        new,
        relative.display()
    ));
    let mut temporary = tempfile::Builder::new()
        .prefix(".managed-files.pending.")
        .tempfile_in(path.parent().unwrap())?;
    temporary.write_all(records.as_bytes())?;
    temporary.flush()?;
    temporary.as_file_mut().sync_all()?;
    if failure == Some("pre-publish") {
        bail!("injected pending journal failure before publication");
    }
    temporary.persist(path).map_err(|e| e.error)?;
    sync_directory(path.parent().unwrap())?;
    if failure == Some("post-publish") {
        bail!("injected pending journal failure after publication");
    }
    Ok(())
}

fn validate_pending(text: &str) -> Result<()> {
    for line in text.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 3 {
            bail!("malformed pending record");
        }
        if fields[0] != "-" {
            validate_hash(fields[0])?;
        }
        validate_hash(fields[1])?;
        validate_relative(Path::new(fields[2]))?;
    }
    Ok(())
}

fn write_manifest(path: &Path, managed: &BTreeMap<PathBuf, String>) -> Result<()> {
    let mut temporary = tempfile::Builder::new()
        .prefix(".managed-files.")
        .tempfile_in(path.parent().unwrap())?;
    for (relative, hash) in managed {
        writeln!(temporary, "{}\t{}", hash, relative.display())?;
    }
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    sync_directory(path.parent().unwrap())?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 record");
    }
    Ok(())
}
fn validate_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || path.to_string_lossy().contains(['\t', '\n'])
    {
        bail!("unsafe managed path: {}", path.display());
    }
    Ok(())
}
fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("config/cozydot");
        let config = temp.path().join("assets/default.yaml");
        let dotfiles = temp.path().join("assets/dotfiles");
        fs::create_dir_all(dotfiles.join("pkg/.config/app")).unwrap();
        fs::write(&config, "config v1\n").unwrap();
        fs::write(dotfiles.join("pkg/.config/app/file with spaces"), "v1\n").unwrap();
        (temp, root, config, dotfiles)
    }

    #[test]
    fn ownership_refreshes_only_unchanged_files_and_preserves_obsolete() {
        let (_temp, root, config, dotfiles) = fixture();
        sync(&root, &config, &dotfiles).unwrap();
        let managed_file = root.join("dotfiles/pkg/.config/app/file with spaces");
        fs::write(&config, "config v2\n").unwrap();
        fs::write(&managed_file, "user edit\n").unwrap();
        fs::write(dotfiles.join("pkg/.config/app/new"), "new\n").unwrap();
        sync(&root, &config, &dotfiles).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("cozydot.yaml")).unwrap(),
            "config v2\n"
        );
        assert_eq!(fs::read_to_string(&managed_file).unwrap(), "user edit\n");
        assert_eq!(
            fs::read_to_string(root.join("dotfiles/pkg/.config/app/new")).unwrap(),
            "new\n"
        );
        fs::remove_file(dotfiles.join("pkg/.config/app/new")).unwrap();
        sync(&root, &config, &dotfiles).unwrap();
        assert!(root.join("dotfiles/pkg/.config/app/new").is_file());
    }

    #[test]
    fn rejects_symlink_and_dangling_ancestors() {
        for dangling in [false, true] {
            let (temp, root, config, dotfiles) = fixture();
            fs::create_dir_all(root.join("dotfiles")).unwrap();
            let target = temp.path().join("outside");
            if !dangling {
                fs::create_dir(&target).unwrap();
            }
            symlink(&target, root.join("dotfiles/pkg")).unwrap();
            assert!(sync(&root, &config, &dotfiles).is_err());
            assert!(!target.join(".config").exists());
        }
    }

    #[test]
    fn pending_recovery_does_not_claim_post_crash_edits() {
        let (_temp, root, config, dotfiles) = fixture();
        fs::create_dir_all(&root).unwrap();
        let source = dotfiles.join("pkg/.config/app/file with spaces");
        let relative = Path::new("dotfiles/pkg/.config/app/file with spaces");
        let new_hash = hash_file(&source).unwrap();
        append_pending(
            &root.join(".managed-files.pending"),
            None,
            &new_hash,
            relative,
        )
        .unwrap();
        ensure_directory_path(&root, relative.parent().unwrap()).unwrap();
        fs::write(root.join(relative), "post-crash edit\n").unwrap();
        sync(&root, &config, &dotfiles).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(relative)).unwrap(),
            "post-crash edit\n"
        );
        assert!(!fs::read_to_string(root.join(".managed-files"))
            .unwrap()
            .contains("file with spaces"));
        assert!(!root.join(".managed-files.pending").exists());
    }

    #[test]
    fn missing_managed_destination_is_reinstalled() {
        let (_temp, root, config, dotfiles) = fixture();
        sync(&root, &config, &dotfiles).unwrap();
        fs::remove_file(root.join("cozydot.yaml")).unwrap();
        sync(&root, &config, &dotfiles).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("cozydot.yaml")).unwrap(),
            "config v1\n"
        );
    }

    #[test]
    fn refreshed_file_edited_after_crash_is_not_claimed() {
        let (_temp, root, config, dotfiles) = fixture();
        sync(&root, &config, &dotfiles).unwrap();
        let relative = Path::new("cozydot.yaml");
        let old_hash = hash_file(&root.join(relative)).unwrap();
        fs::write(&config, "config v2\n").unwrap();
        let new_hash = hash_file(&config).unwrap();
        append_pending(
            &root.join(".managed-files.pending"),
            Some(&old_hash),
            &new_hash,
            relative,
        )
        .unwrap();
        fs::write(root.join(relative), "user edit\n").unwrap();
        sync(&root, &config, &dotfiles).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(relative)).unwrap(),
            "user edit\n"
        );
        assert!(!fs::read_to_string(root.join(".managed-files"))
            .unwrap()
            .contains("cozydot.yaml"));
    }

    #[test]
    fn malformed_manifests_and_journals_are_rejected() {
        for (name, contents) in [
            (".managed-files", "not-a-record\n"),
            (".managed-files.pending", "too\tfew\n"),
        ] {
            let (_temp, root, config, dotfiles) = fixture();
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(name), contents).unwrap();
            assert!(sync(&root, &config, &dotfiles).is_err());
        }
    }

    #[test]
    fn pending_publication_failures_leave_recoverable_journals() {
        for failure in ["pre-publish", "post-publish"] {
            let (_temp, root, config, dotfiles) = fixture();
            fs::create_dir_all(&root).unwrap();
            let relative = Path::new("cozydot.yaml");
            let new_hash = hash_file(&config).unwrap();
            let pending = root.join(".managed-files.pending");
            let result =
                append_pending_with_failure(&pending, None, &new_hash, relative, Some(failure));
            assert!(result.is_err());
            if failure == "pre-publish" {
                assert!(!pending.exists());
            } else {
                assert!(fs::read_to_string(&pending).unwrap().ends_with('\n'));
            }
            sync(&root, &config, &dotfiles).unwrap();
            assert!(!pending.exists());
            assert_eq!(
                fs::read_to_string(root.join(relative)).unwrap(),
                "config v1\n"
            );
        }
    }
}
