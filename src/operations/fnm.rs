use anyhow::{Result, bail};

use super::{Host, TempPath, path_program, real_executable_file, shell::append_shell};

const FNM_BASH_INIT: &str = r#"FNM_PATH="$HOME/.local/share/fnm"
if [ -d "$FNM_PATH" ]; then
  export PATH="$FNM_PATH:$PATH"
  eval "$(fnm env --use-on-cd --shell bash)"
fi"#;
const FNM_ZSH_INIT: &str = r#"eval "$(fnm env --use-on-cd --shell zsh)""#;

pub fn install_fnm(host: &Host) -> Result<()> {
    if cfg!(target_os = "macos") {
        super::macos::install_formula(host, "fnm")?;
        return append_shell(host, FNM_ZSH_INIT);
    }

    let data_home = host.home().join(".local/share");
    let installed = data_home.join("fnm/fnm");
    if !real_executable_file(&installed) {
        let installer = TempPath::new(host, "fnm-install")?;
        host.curl(
            "FNM installer download",
            "https://fnm.vercel.app/install",
            ["--output", &installer.path().to_string_lossy()],
        )?;
        host.require(
            "FNM install",
            "env",
            [
                format!("XDG_DATA_HOME={}", data_home.display()),
                "bash".to_owned(),
                installer.path().to_string_lossy().into_owned(),
                "--skip-shell".to_owned(),
            ],
        )?;
        if !real_executable_file(&installed) {
            bail!("FNM installer did not publish executable {}", installed.display());
        }
    }
    append_shell(host, FNM_BASH_INIT)
}

pub(crate) fn install_default_toolchain(host: &Host, selector: &str) -> Result<()> {
    let fnm = resolve_fnm(host)?;
    fnm_install(host, &fnm, selector)?;
    host.require("fnm default", &fnm, ["default", "--", fnm_alias(selector)])?;
    Ok(())
}

fn fnm_install(host: &Host, fnm: &str, selector: &str) -> Result<()> {
    if selector == "lts" {
        host.require("fnm install", fnm, ["install", "--progress", "never", "--lts"])?;
    } else {
        host.require("fnm install", fnm, ["install", "--progress", "never", "--", selector])?;
    }
    Ok(())
}

fn resolve_fnm(host: &Host) -> Result<String> {
    if cfg!(target_os = "macos") {
        return super::macos::formula_executable(host, "fnm", "fnm");
    }
    let data_home = host.home().join(".local/share");
    let managed = data_home.join("fnm/fnm");
    if real_executable_file(&managed) {
        return path_program(&managed, "managed fnm executable path");
    }
    bail!("Node toolchain operation: fnm is unavailable after install")
}

fn fnm_alias(selector: &str) -> &str {
    match selector {
        "lts" => "lts-latest",
        value => value,
    }
}
