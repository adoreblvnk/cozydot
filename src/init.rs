use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use rust_embed::Embed;
use std::{
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
};

#[derive(Embed)]
#[folder = "dotfiles/"]
#[prefix = "dotfiles/"]
struct Dotfiles;

const COZYDOT_PRESET: &[u8] = include_bytes!("../configs/cozydot.yaml");
const CLI_PRESET: &[u8] = include_bytes!("../configs/cli.yaml");
const VM_PRESET: &[u8] = include_bytes!("../configs/vm.yaml");

#[derive(Clone, Debug, ValueEnum)]
pub enum Preset {
    Cozydot,
    Cli,
    Vm,
}

/// Create `cozydot.yaml` & `dotfiles` dir.
pub fn init(preset: Preset, force: bool) -> Result<PathBuf> {
    let root = crate::paths::config_dir()?;
    let config_path = root.join("cozydot.yaml");
    if config_path.exists() && !force {
        bail!("configuration already exists at {}; use --force to overwrite", config_path.display());
    }
    ensure_directory_path(&root, Path::new(""))?;
    let preset = match preset {
        Preset::Cozydot => COZYDOT_PRESET,
        Preset::Cli => CLI_PRESET,
        Preset::Vm => VM_PRESET,
    };
    write_file(&root, Path::new("cozydot.yaml"), preset, 0o644)?;
    for path in Dotfiles::iter() {
        let bytes = Dotfiles::get(path.as_ref()).with_context(|| format!("missing embedded file {path}"))?.data;
        let mode = if bytes.starts_with(b"#!") { 0o755 } else { 0o644 };
        write_file(&root, Path::new(path.as_ref()), bytes.as_ref(), mode)?;
    }
    Ok(root)
}

fn write_file(root: &Path, relative: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    ensure_directory_path(root, relative.parent().unwrap_or(Path::new("")))?;
    let dest = root.join(relative);
    let dest_parent = required_parent(&dest)?;
    // write & sync beside the destination before publishing with rename
    let mut temp = tempfile::NamedTempFile::with_prefix_in(".cozydot.", dest_parent)?;
    temp.write_all(bytes)?;
    temp.as_file_mut().sync_all()?;
    temp.as_file_mut().set_permissions(fs::Permissions::from_mode(mode))?;
    temp.persist(&dest)?;
    sync_dir(dest_parent)
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
            Ok(meta) if meta.file_type().is_symlink() => bail!("refusing symlinked config path: {}", current.display()),
            Ok(meta) if !meta.is_dir() => bail!("refusing non-directory config path: {}", current.display()),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn required_parent(path: &Path) -> Result<&Path> {
    path.parent().with_context(|| format!("path has no parent: {}", path.display()))
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
