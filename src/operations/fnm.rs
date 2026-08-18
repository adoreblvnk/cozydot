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

    let install_dir = host.home().join(".local/share/fnm");
    let fnm_path = install_dir.join("fnm");
    if !regular_executable_file(&fnm_path) {
        let installer = TempPath::new(host, "fnm-install")?;
        host.curl(
            "fnm installer download",
            "https://fnm.vercel.app/install",
            ["--output", &installer.path().to_string_lossy()],
        )?;
        host.run(
            "fnm install",
            "bash",
            [
                installer.path().to_string_lossy().into_owned(),
                "--install-dir".to_owned(),
                install_dir.to_string_lossy().into_owned(),
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
    let Some(fnm) = resolve(host)? else {
        bail!("fnm install: fnm is unavailable after install");
    };
    if selector == "lts" {
        host.run("fnm install", &fnm, ["install", "--progress", "never", "--lts"])?;
    } else {
        host.run("fnm install", &fnm, ["install", "--progress", "never", "--", selector])?;
    }
    host.run("fnm default", &fnm, ["default", "--", if selector == "lts" { "lts-latest" } else { selector }])?;
    Ok(())
}

pub(crate) fn resolve(host: &Host) -> Result<Option<String>> {
    if cfg!(target_os = "macos") {
        return super::macos::formula_executable(host, "fnm", "fnm").map(Some);
    }
    let managed = host.home().join(".local/share/fnm/fnm");
    if regular_executable_file(&managed) {
        return path_program(&managed, "managed fnm executable path").map(Some);
    }
    Ok(None)
}
