use crate::operations::{
    host::shell::append_shell_rc,
    host::{self, is_regular_executable, path_program, temp_path},
    packages::homebrew,
};
use anyhow::{Context, Result, ensure};

const FNM_BASH_INIT: &str = r#"FNM_PATH="$HOME/.local/share/fnm"
if [ -d "$FNM_PATH" ]; then
  export PATH="$FNM_PATH:$PATH"
  eval "$(fnm env --use-on-cd --shell bash)"
fi"#;
const FNM_ZSH_INIT: &str = r#"eval "$(fnm env --use-on-cd --shell zsh)""#;

pub fn install() -> Result<()> {
    if cfg!(target_os = "macos") {
        homebrew::install_packages(&["fnm".to_owned()], &[])?;
        return append_shell_rc(FNM_ZSH_INIT);
    }

    let install_dir = host::home()?.join(".local/share/fnm");
    let fnm_path = install_dir.join("fnm");
    if !is_regular_executable(&fnm_path) {
        let installer = temp_path("fnm-install", "")?;
        let path = installer.as_os_str();
        host::curl("fnm installer download", "https://fnm.vercel.app/install", ["--output".as_ref(), path])?;
        let install_dir = install_dir.as_os_str();
        // skip installer shell edits because Cozydot owns the profile snippet
        host::run("fnm install", "bash", [path, "--install-dir".as_ref(), install_dir, "--skip-shell".as_ref()])?;
        ensure!(is_regular_executable(&fnm_path), "fnm installer did not publish executable {}", fnm_path.display());
    }
    append_shell_rc(FNM_BASH_INIT)
}

pub(crate) fn install_version(selector: &str) -> Result<()> {
    let fnm = find_executable()?.context("fnm install: fnm is unavailable after install")?;
    if selector == "lts" {
        host::run("fnm install", &fnm, ["install", "--progress", "never", "--lts"])?;
    } else {
        host::run("fnm install", &fnm, ["install", "--progress", "never", "--", selector])?;
    }
    host::run("fnm default", &fnm, ["default", "--", if selector == "lts" { "lts-latest" } else { selector }])?;
    Ok(())
}

pub(crate) fn find_executable() -> Result<Option<String>> {
    if cfg!(target_os = "macos") {
        return homebrew::executable_path("fnm", "fnm").map(Some);
    }
    let managed = host::home()?.join(".local/share/fnm/fnm");
    if is_regular_executable(&managed) {
        return path_program(&managed, "managed fnm executable path").map(Some);
    }
    Ok(None)
}
