use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, ValueEnum)]
pub enum Preset {
    Cozydot,
    Cli,
    Vm,
}

/// Create `cozydot.yaml` & `dotfiles` dir without overwriting user-managed changes.
pub fn init(preset: Preset) -> Result<PathBuf> {
    let root = config_root()?;
    ensure_directory_path(&root, Path::new(""))?;
    let managed = read_manifest(&root.join(".managed-files"))?;
    let mut init = Init { root, managed };
    let preset = match preset {
        Preset::Cozydot => COZYDOT_PRESET,
        Preset::Cli => CLI_PRESET,
        Preset::Vm => VM_PRESET,
    };
    init.sync_cozydot_yaml(preset)?;
    init.sync_bundled_dotfiles()?;
    // write ownership last so retries preserve files synced by partial runs
    write_manifest(&init.root.join(".managed-files"), &init.managed)?;
    Ok(init.root)
}

pub fn config_root() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("cozydot"));
    }
    Ok(PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".config/cozydot"))
}

pub struct Record {
    pub path: &'static str,
    pub bytes: &'static [u8],
    pub mode: u32,
}

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

struct Init {
    root: PathBuf,
    managed: BTreeMap<PathBuf, String>,
}

impl Init {
    fn sync_cozydot_yaml(&mut self, preset: &'static [u8]) -> Result<()> {
        let record = Record { path: "cozydot.yaml", bytes: preset, mode: 0o644 };
        let relative = PathBuf::from(record.path);
        let dest = self.root.join(&relative);
        let new_hash = hash_bytes(record.bytes);
        let old_hash = self.managed.get(&relative);
        let write = match fs::symlink_metadata(&dest) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => true,
            Err(e) => return Err(e.into()),
            Ok(metadata) if !metadata.file_type().is_file() => false,
            Ok(_) => old_hash.is_some_and(|hash| hash_file(&dest).ok().as_ref() == Some(hash)),
        };
        if write {
            write_file(&self.root, &record, &relative)?;
            self.managed.insert(relative, new_hash);
        }
        Ok(())
    }

    fn sync_bundled_dotfiles(&mut self) -> Result<()> {
        let mut packages = BTreeMap::<PathBuf, Vec<&Record>>::new();
        for record in RECORDS {
            let relative = PathBuf::from(record.path);
            let package = relative.components().take(2).collect();
            packages.entry(package).or_default().push(record);
        }
        for (package, records) in packages {
            self.sync_dotfile_package(&package, &records)?;
        }
        Ok(())
    }

    fn sync_dotfile_package(&mut self, package: &Path, records: &[&Record]) -> Result<()> {
        if !self.dotfile_package_is_unmodified(package)? {
            return Ok(());
        }

        for record in records {
            let relative = PathBuf::from(record.path);
            write_file(&self.root, record, &relative)?;
            self.managed.insert(relative, hash_bytes(record.bytes));
        }
        Ok(())
    }

    fn dotfile_package_is_unmodified(&self, package: &Path) -> Result<bool> {
        let managed = self
            .managed
            .iter()
            .filter(|(relative, _)| {
                relative.strip_prefix(package).is_ok_and(|suffix| suffix.components().next().is_some())
            })
            .collect::<Vec<_>>();
        let dest = self.root.join(package);
        match fs::symlink_metadata(&dest) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(managed.is_empty()),
            Err(error) => return Err(error.into()),
            Ok(metadata) if !metadata.file_type().is_dir() => return Ok(false),
            Ok(_) if managed.is_empty() => return Ok(false),
            Ok(_) => {}
        }

        let mut files = BTreeSet::new();
        if !collect_real_files(&dest, &self.root, &mut files)? {
            return Ok(false);
        }
        if !files.iter().eq(managed.iter().map(|(relative, _)| *relative)) {
            return Ok(false);
        }
        Ok(managed
            .iter()
            .all(|(relative, hash)| hash_file(&self.root.join(relative)).is_ok_and(|current| &current == *hash)))
    }
}

fn write_file(root: &Path, record: &Record, relative: &Path) -> Result<()> {
    let parent = relative.parent().unwrap_or(Path::new(""));
    ensure_directory_path(root, parent)?;
    let dest = root.join(relative);
    let dest_parent = required_parent(&dest)?;
    let mut temp = tempfile::Builder::new().prefix(".cozydot.").tempfile_in(dest_parent)?;
    temp.write_all(record.bytes)?;
    temp.as_file_mut().sync_all()?;
    temp.as_file_mut().set_permissions(fs::Permissions::from_mode(record.mode))?;
    temp.persist(&dest).map_err(|e| e.error)?;
    sync_dir(dest_parent)?;
    Ok(())
}

/// Create missing dirs under `root` & fail if `root` or a child dir is a symlink.
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

fn collect_real_files(dir: &Path, root: &Path, files: &mut BTreeSet<PathBuf>) -> Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if !collect_real_files(&entry.path(), root, files)? {
                return Ok(false);
            }
        } else if file_type.is_file() {
            files.insert(entry.path().strip_prefix(root)?.to_path_buf());
        } else {
            return Ok(false);
        }
    }
    Ok(true)
}

fn read_manifest(path: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let mut result = BTreeMap::new();
    let text = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(result),
        Err(e) => return Err(e.into()),
    };
    for line in text.lines() {
        let (hash, relative) = line.split_once('\t').context("malformed managed-files record")?;
        let relative = PathBuf::from(relative);
        validate_hash(hash)?;
        validate_relative(&relative)?;
        if result.insert(relative, hash.into()).is_some() {
            bail!("duplicate managed-files record");
        }
    }
    Ok(result)
}

// TODO: do we really need this where we're going?
fn write_manifest(path: &Path, managed: &BTreeMap<PathBuf, String>) -> Result<()> {
    let parent = required_parent(path)?;
    let mut temp = tempfile::Builder::new().prefix(".managed-files.").tempfile_in(parent)?;
    for (relative, hash) in managed {
        writeln!(temp, "{}\t{}", hash, relative.display())?;
    }
    temp.as_file_mut().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    sync_dir(parent)?;
    Ok(())
}

fn required_parent(path: &Path) -> Result<&Path> {
    path.parent().with_context(|| format!("path has no parent: {}", path.display()))
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 record");
    }
    Ok(())
}
fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_file(path: &Path) -> Result<String> {
    Ok(hash_bytes(&fs::read(path)?))
}
fn validate_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.components().any(|c| !matches!(c, Component::Normal(_)))
        || path.to_string_lossy().contains(['\t', '\n'])
    {
        bail!("unsafe managed path: {}", path.display());
    }
    Ok(())
}
