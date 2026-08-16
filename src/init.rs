use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Preset {
    #[default]
    Cozydot,
    Cli,
    Vm,
}

impl Preset {
    fn name(self) -> &'static str {
        match self {
            Self::Cozydot => "cozydot",
            Self::Cli => "cli",
            Self::Vm => "vm",
        }
    }
}

/// Synchronize bundled config and dotfiles while preserving unmanaged or modified paths.
pub fn init(preset: Preset) -> Result<PathBuf> {
    let mut initialization = Initialization::resolve_and_validate_configuration_root()?;
    let preset = select_embedded_preset(preset)?;
    initialization.synchronize_active_configuration(preset)?;
    initialization.synchronize_bundled_dotfiles()?;
    // Publish ownership last so the manifest never claims files from a partial synchronization.
    initialization.publish_managed_file_manifest()?;
    Ok(initialization.root)
}

pub fn config_root() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("cozydot"));
    }
    Ok(PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".config/cozydot"))
}

#[derive(Clone, Copy, Debug)]
pub struct Record {
    pub path: &'static str,
    pub bytes: &'static [u8],
    pub mode: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct PresetRecord {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

pub fn records() -> &'static [Record] {
    RECORDS
}

pub fn preset(name: &str) -> Option<&'static PresetRecord> {
    PRESETS.iter().find(|preset| preset.name == name)
}

struct Initialization {
    root: PathBuf,
    managed: BTreeMap<PathBuf, String>,
}

impl Initialization {
    fn resolve_and_validate_configuration_root() -> Result<Self> {
        let root = config_root()?;
        ensure_directory_path(&root, Path::new(""))?;
        let managed = read_manifest(&root.join(".managed-files"))?;
        Ok(Self { root, managed })
    }

    fn synchronize_active_configuration(&mut self, preset: &PresetRecord) -> Result<()> {
        self.synchronize_record(&Record { path: "cozydot.yaml", bytes: preset.bytes, mode: 0o644 })
    }

    fn synchronize_bundled_dotfiles(&mut self) -> Result<()> {
        for record in records() {
            self.synchronize_record(record)?;
        }
        Ok(())
    }

    fn synchronize_record(&mut self, record: &Record) -> Result<()> {
        let relative = PathBuf::from(record.path);
        validate_relative(&relative)?;
        let destination = self.root.join(&relative);
        let new_hash = hash_bytes(record.bytes);
        let old_hash = self.managed.get(&relative);
        let install = match fs::symlink_metadata(&destination) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => true,
            Err(e) => return Err(e.into()),
            Ok(metadata) if !metadata.file_type().is_file() => false,
            Ok(_) => old_hash.is_some_and(|hash| hash_file(&destination).ok().as_ref() == Some(hash)),
        };
        if install {
            install_file(&self.root, record, &relative)?;
            self.managed.insert(relative, new_hash);
        }
        Ok(())
    }

    fn publish_managed_file_manifest(&self) -> Result<()> {
        write_manifest(&self.root.join(".managed-files"), &self.managed)
    }
}

fn select_embedded_preset(preset: Preset) -> Result<&'static PresetRecord> {
    self::preset(preset.name()).context("embedded preset is missing")
}

fn install_file(root: &Path, record: &Record, relative: &Path) -> Result<()> {
    let parent = relative.parent().unwrap_or(Path::new(""));
    ensure_directory_path(root, parent)?;
    let destination = root.join(relative);
    let destination_parent = required_parent(&destination)?;
    let mut temporary = tempfile::Builder::new().prefix(".cozydot.").tempfile_in(destination_parent)?;
    temporary.write_all(record.bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary.as_file_mut().set_permissions(fs::Permissions::from_mode(record.mode))?;
    temporary.persist(&destination).map_err(|e| e.error)?;
    sync_directory(destination_parent)?;
    Ok(())
}

fn ensure_directory_path(root: &Path, relative: &Path) -> Result<()> {
    // Managed paths must not traverse existing symlinks that could redirect writes outside the config root.
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

fn write_manifest(path: &Path, managed: &BTreeMap<PathBuf, String>) -> Result<()> {
    let parent = required_parent(path)?;
    let mut temporary = tempfile::Builder::new().prefix(".managed-files.").tempfile_in(parent)?;
    for (relative, hash) in managed {
        writeln!(temporary, "{}\t{}", hash, relative.display())?;
    }
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    sync_directory(parent)?;
    Ok(())
}

fn required_parent(path: &Path) -> Result<&Path> {
    path.parent().with_context(|| format!("path has no parent: {}", path.display()))
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
