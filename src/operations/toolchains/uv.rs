use crate::operations::host::{
    Host, TempPath, regular_executable_file, require_regular_executable, shell::append_profile,
};
use anyhow::{Result, bail};

const UV_INIT: &str = r#"if [ -f "$HOME/.local/bin/env" ]; then
  . "$HOME/.local/bin/env"
fi"#;

pub fn install(host: &Host) -> Result<()> {
    let uv_path = host.home().join(".local/bin/uv");
    if !regular_executable_file(&uv_path) {
        let installer = TempPath::new("uv-install")?;
        let path = installer.path().as_os_str();
        host.curl("uv installer download", "https://astral.sh/uv/install.sh", ["--output".as_ref(), path])?;
        host.run("uv install", "env", ["UV_NO_MODIFY_PATH=1".as_ref(), "sh".as_ref(), path])?;
        if !regular_executable_file(&uv_path) {
            bail!("uv installer did not publish executable {}", uv_path.display());
        }
    }
    append_profile(host, UV_INIT)
}

pub(crate) fn install_py(host: &Host, selector: &str) -> Result<()> {
    let uv = require_regular_executable(
        &host.home().join(".local/bin/uv"),
        "managed tool executable path",
        "uv python install: uv is unavailable after install",
    )?;
    host.run(
        "uv python install",
        &uv,
        ["python", "install", "--no-config", "--managed-python", "--no-progress", "--default", "--", selector],
    )?;
    Ok(())
}

pub(crate) fn upgrade_py_versions(host: &Host) -> Result<()> {
    let uv = require_regular_executable(
        &host.home().join(".local/bin/uv"),
        "managed tool executable path",
        "Python version upgrade: uv is unavailable after install",
    )?;
    host.run("uv self update", &uv, ["self", "update"])?;
    host.run("Python version upgrade", &uv, ["python", "upgrade", "--no-config", "--managed-python", "--no-progress"])?;
    Ok(())
}
