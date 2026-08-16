use anyhow::{Context, Result, bail};

use super::{Host, TempPath, regular_executable_file, require_regular_executable, shell::append_profile};

const UV_INIT: &str = r#"if [ -f "$HOME/.local/bin/env" ]; then
  . "$HOME/.local/bin/env"
fi"#;

pub fn install(host: &Host) -> Result<()> {
    let installed = host.home().join(".local/bin/uv");
    if !regular_executable_file(&installed) {
        let installer = TempPath::new(host, "uv-install")?;
        host.curl(
            "uv installer download",
            "https://astral.sh/uv/install.sh",
            ["--output", &installer.path().to_string_lossy()],
        )?;
        host.run_checked(
            "uv install",
            "env",
            ["UV_NO_MODIFY_PATH=1", "sh", installer.path().to_str().context("uv installer path is not UTF-8")?],
        )?;
        if !regular_executable_file(&installed) {
            bail!("uv installer did not publish executable {}", installed.display());
        }
    }
    append_profile(host, UV_INIT)
}

pub(crate) fn install_toolchain(host: &Host, selector: &str) -> Result<()> {
    let uv = require_regular_executable(
        &host.home().join(".local/bin/uv"),
        "managed tool executable path",
        "Python toolchain operation: uv is unavailable after install",
    )?;
    host.run_checked(
        "uv python install",
        &uv,
        ["python", "install", "--no-config", "--managed-python", "--no-progress", "--default", "--", selector],
    )?;
    Ok(())
}

pub(crate) fn update_toolchain(host: &Host) -> Result<()> {
    let uv = require_regular_executable(
        &host.home().join(".local/bin/uv"),
        "managed tool executable path",
        "Python toolchain update: uv is unavailable after install",
    )?;
    host.run_checked("uv self update", &uv, ["self", "update"])?;
    host.run_checked(
        "Python toolchain update",
        &uv,
        ["python", "upgrade", "--no-config", "--managed-python", "--no-progress"],
    )?;
    Ok(())
}
