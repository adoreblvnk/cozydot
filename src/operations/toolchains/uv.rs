use crate::operations::host::{
    self, is_regular_executable, require_regular_executable, shell::append_profile, temp_path,
};
use anyhow::{Context, Result, ensure};

const UV_INIT: &str = r#"if [ -f "$HOME/.local/bin/env" ]; then
  . "$HOME/.local/bin/env"
fi"#;

pub fn is_installed() -> Result<bool> {
    Ok(is_regular_executable(&host::home()?.join(".local/bin/uv")))
}

pub(crate) fn is_python_installed(selector: &str) -> Result<bool> {
    if !is_installed()? {
        return Ok(false);
    }
    let uv = host::home()?.join(".local/bin/uv");
    let output = host::output(uv.to_str().unwrap_or("uv"), ["python", "find", "--no-config", "--", selector])?;
    Ok(output.status.success())
}

pub fn install() -> Result<()> {
    let installer = temp_path("uv-install", "")?;
    let path = installer.to_str().context("uv installer path is not UTF-8")?;
    host::curl("uv installer download", "https://astral.sh/uv/install.sh", ["--output", path])?;
    // disable installer PATH edits because cozydot sources the upstream env snippet itself
    host::run("uv install", "env", ["UV_NO_MODIFY_PATH=1", "sh", path])?;
    let uv_path = host::home()?.join(".local/bin/uv");
    ensure!(is_regular_executable(&uv_path), "uv installer did not publish executable {}", uv_path.display());
    append_profile(UV_INIT)
}

pub(crate) fn install_py(selector: &str) -> Result<()> {
    let uv = require_regular_executable(
        &host::home()?.join(".local/bin/uv"),
        "managed tool executable path",
        "uv python install: uv is unavailable after install",
    )?;
    let args = ["python", "install", "--no-config", "--managed-python", "--no-progress", "--default", "--", selector];
    host::run("uv python install", &uv, args)?;
    Ok(())
}

pub(crate) fn upgrade_py() -> Result<()> {
    let uv = require_regular_executable(
        &host::home()?.join(".local/bin/uv"),
        "managed tool executable path",
        "Python version upgrade: uv is unavailable after install",
    )?;
    host::run("uv self update", &uv, ["self", "update"])?;
    let args = ["python", "upgrade", "--no-config", "--managed-python", "--no-progress"];
    host::run("Python version upgrade", &uv, args)?;
    Ok(())
}
