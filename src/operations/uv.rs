use anyhow::{Context, Result, bail};

use super::{Host, TempPath, path_program, real_executable_file, shell::append_profile};

const UV_INIT: &str = r#"if [ -f "$HOME/.local/bin/env" ]; then
  . "$HOME/.local/bin/env"
fi"#;

pub fn install_uv(host: &Host) -> Result<()> {
    let installed = host.home().join(".local/bin/uv");
    if !real_executable_file(&installed) {
        let installer = TempPath::new(host, "uv-install")?;
        host.curl(
            "uv installer download",
            "https://astral.sh/uv/install.sh",
            ["--output", &installer.path().to_string_lossy()],
        )?;
        host.require(
            "uv install",
            "env",
            ["UV_NO_MODIFY_PATH=1", "sh", installer.path().to_str().context("uv installer path is not UTF-8")?],
        )?;
        if !real_executable_file(&installed) {
            bail!("uv installer did not publish executable {}", installed.display());
        }
    }
    append_profile(host, UV_INIT)
}

pub(crate) fn install_default_python(host: &Host, version: &str) -> Result<()> {
    let uv = managed_executable(host, "Python toolchain operation: uv is unavailable after install")?;
    host.require(
        "uv python install",
        &uv,
        ["python", "install", "--no-config", "--managed-python", "--no-progress", "--default", "--", version],
    )?;
    Ok(())
}

pub(crate) fn update_python(host: &Host) -> Result<()> {
    let uv = managed_executable(host, "Python toolchain update: uv is unavailable after install")?;
    host.require("uv self update", &uv, ["self", "update"])?;
    host.require(
        "Python toolchain update",
        &uv,
        ["python", "upgrade", "--no-config", "--managed-python", "--no-progress"],
    )?;
    Ok(())
}

fn managed_executable(host: &Host, message: &str) -> Result<String> {
    let path = host.home().join(".local/bin/uv");
    if real_executable_file(&path) {
        return path_program(&path, "managed tool executable path");
    }
    bail!("{message}")
}
