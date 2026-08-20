use anyhow::{Result, bail};
use std::ffi::OsStr;

use crate::operations::{
    host::shell::append_shell,
    host::{Host, TempPath, path_program, regular_executable_file},
    packages::homebrew,
};

const FNM_BASH_INIT: &str = r#"FNM_PATH="$HOME/.local/share/fnm"
if [ -d "$FNM_PATH" ]; then
  export PATH="$FNM_PATH:$PATH"
  eval "$(fnm env --use-on-cd --shell bash)"
fi"#;
const FNM_ZSH_INIT: &str = r#"eval "$(fnm env --use-on-cd --shell zsh)""#;

pub fn install(host: &Host) -> Result<()> {
    if cfg!(target_os = "macos") {
        homebrew::install_packages(host, &["fnm".to_owned()], &[])?;
        return append_shell(host, FNM_ZSH_INIT);
    }

    let install_dir = host.home().join(".local/share/fnm");
    let fnm_path = install_dir.join("fnm");
    if !regular_executable_file(&fnm_path) {
        let installer = TempPath::new("fnm-install")?;
        host.curl(
            "fnm installer download",
            "https://fnm.vercel.app/install",
            [OsStr::new("--output"), installer.path().as_os_str()],
        )?;
        host.run(
            "fnm install",
            "bash",
            [
                installer.path().as_os_str(),
                OsStr::new("--install-dir"),
                install_dir.as_os_str(),
                OsStr::new("--skip-shell"),
            ],
        )?;
        if !regular_executable_file(&fnm_path) {
            bail!("fnm installer did not publish executable {}", fnm_path.display());
        }
    }
    append_shell(host, FNM_BASH_INIT)
}

pub(crate) fn install_version(host: &Host, selector: &str) -> Result<()> {
    let Some(fnm) = find_executable(host)? else {
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

pub(crate) fn find_executable(host: &Host) -> Result<Option<String>> {
    if cfg!(target_os = "macos") {
        return homebrew::formula_executable(host, "fnm", "fnm").map(Some);
    }
    let managed = host.home().join(".local/share/fnm/fnm");
    if regular_executable_file(&managed) {
        return path_program(&managed, "managed fnm executable path").map(Some);
    }
    Ok(None)
}
