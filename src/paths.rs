use std::{env, path::PathBuf};

use anyhow::{Context, Result, ensure};

pub(crate) fn config_home() -> Result<PathBuf> {
    xdg_home("XDG_CONFIG_HOME", ".config")
}

pub(crate) fn config_dir() -> Result<PathBuf> {
    Ok(config_home()?.join("cozydot"))
}

pub(crate) fn state_home() -> Result<PathBuf> {
    xdg_home("XDG_STATE_HOME", ".local/state")
}

fn xdg_home(variable: &str, default: &str) -> Result<PathBuf> {
    // empty XDG values fall back to HOME while non-empty values must be absolute
    if let Some(path) = env::var_os(variable).filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        ensure!(path.is_absolute(), "{variable} must be an absolute path");
        return Ok(path);
    }
    Ok(PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(default))
}
