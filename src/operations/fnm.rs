use anyhow::{Result, bail};

use super::{Host, TempPath, path_program, regular_executable_file, shell::append_shell};

const FNM_BASH_INIT: &str = r#"FNM_PATH="$HOME/.local/share/fnm"
if [ -d "$FNM_PATH" ]; then
  export PATH="$FNM_PATH:$PATH"
  eval "$(fnm env --use-on-cd --shell bash)"
fi"#;
const FNM_ZSH_INIT: &str = r#"eval "$(fnm env --use-on-cd --shell zsh)""#;

pub fn install(host: &Host) -> Result<()> {
    if cfg!(target_os = "macos") {
        super::macos::install_formula(host, "fnm")?;
        return append_shell(host, FNM_ZSH_INIT);
    }

    let data_home = host.home().join(".local/share");
    let fnm_path = data_home.join("fnm/fnm");
    if !regular_executable_file(&fnm_path) {
        let installer = TempPath::new(host, "fnm-install")?;
        host.curl(
            "fnm installer download",
            "https://fnm.vercel.app/install",
            ["--output", &installer.path().to_string_lossy()],
        )?;
        host.run(
            "fnm install",
            "env",
            [
                format!("XDG_DATA_HOME={}", data_home.display()),
                "bash".to_owned(),
                installer.path().to_string_lossy().into_owned(),
                "--skip-shell".to_owned(),
            ],
        )?;
        if !regular_executable_file(&fnm_path) {
            bail!("fnm installer did not publish executable {}", fnm_path.display());
        }
    }
    append_shell(host, FNM_BASH_INIT)
}

pub(crate) fn install_version(host: &Host, selector: &str) -> Result<()> {
    let fnm = resolve_fnm(host)?;
    if selector == "lts" {
        host.run("fnm install", &fnm, ["install", "--progress", "never", "--lts"])?;
    } else {
        host.run("fnm install", &fnm, ["install", "--progress", "never", "--", selector])?;
    }
    host.run("fnm default", &fnm, ["default", "--", if selector == "lts" { "lts-latest" } else { selector }])?;
    Ok(())
}

fn resolve_fnm(host: &Host) -> Result<String> {
    if cfg!(target_os = "macos") {
        return super::macos::formula_executable(host, "fnm", "fnm");
    }
    let data_home = host.home().join(".local/share");
    let managed = data_home.join("fnm/fnm");
    if regular_executable_file(&managed) {
        return path_program(&managed, "managed fnm executable path");
    }
    bail!("fnm install: fnm is unavailable after install")
}
