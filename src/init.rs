use anyhow::{bail, Context, Result};
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Preset {
    #[default]
    Cozydot,
    Full,
    Cli,
    Vm,
}

impl Preset {
    fn name(self) -> &'static str {
        match self {
            Self::Cozydot => "cozydot",
            Self::Full => "full",
            Self::Cli => "cli",
            Self::Vm => "vm",
        }
    }
}

pub fn run(preset_val: Preset) -> Result<PathBuf> {
    let root = config_root()?;
    let preset_rec = preset(preset_val.name()).context("embedded preset is missing")?;
    let mut records_vec = Vec::with_capacity(records().len() + 1);
    records_vec.push(Record {
        path: "cozydot.yaml",
        bytes: preset_rec.bytes,
        mode: 0o644,
    });
    records_vec.extend_from_slice(records());
    sync(&root, &records_vec)?;
    Ok(root)
}

fn sync(root: &Path, records: &[Record]) -> Result<()> {
    ensure_directory_path(root, Path::new(""))?;
    let manifest_path = root.join(".managed-files");
    let mut managed = read_manifest(&manifest_path)?;

    for record in records {
        let relative = PathBuf::from(record.path);
        validate_relative(&relative)?;
        let destination = root.join(&relative);
        let new_hash = hash_bytes(record.bytes);
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
        install_file(root, record, &relative)?;
        managed.insert(relative.clone(), new_hash);
    }
    write_manifest(&manifest_path, &managed)?;
    Ok(())
}

pub fn config_root() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("cozydot"));
    }
    Ok(PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".config/cozydot"))
}

fn install_file(root: &Path, record: &Record, relative: &Path) -> Result<()> {
    let parent = relative.parent().unwrap_or(Path::new(""));
    ensure_directory_path(root, parent)?;
    let destination = root.join(relative);
    let destination_parent = required_parent(&destination)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".cozydot.")
        .tempfile_in(destination_parent)?;
    temporary.write_all(record.bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .as_file_mut()
        .set_permissions(fs::Permissions::from_mode(record.mode))?;
    temporary.persist(&destination).map_err(|e| e.error)?;
    sync_directory(destination_parent)?;
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
    path.parent()
        .with_context(|| format!("path has no parent: {}", path.display()))
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
