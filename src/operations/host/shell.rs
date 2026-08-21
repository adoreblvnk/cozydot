use std::{fs, fs::OpenOptions, io::Write, path::Path};

use anyhow::{Context, Result};

use super::*;

pub(crate) fn append_profile(snippet: &str) -> Result<()> {
    let name = if cfg!(target_os = "macos") { ".zprofile" } else { ".profile" };
    append_once(&home()?.join(name), snippet)
}

pub(crate) fn append_shell_rc(snippet: &str) -> Result<()> {
    let name = if cfg!(target_os = "macos") { ".zshrc" } else { ".bashrc" };
    append_once(&home()?.join(name), snippet)
}

fn append_once(path: &Path, snippet: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create shell profile directory {}", parent.display()))?;
    }
    let current = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("read shell profile {}", path.display())),
    };
    if current.contains(snippet) {
        return Ok(());
    }
    let context = || format!("open shell profile {}", path.display());
    let mut file = OpenOptions::new().create(true).append(true).open(path).with_context(context)?;
    if !current.is_empty() && !current.ends_with('\n') {
        writeln!(file)?;
    }
    writeln!(file, "{snippet}")?;
    Ok(())
}
