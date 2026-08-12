use anyhow::{Context, Result, bail};
use std::{fs, fs::OpenOptions, io::Write, path::Path};

use super::{Host, TempPath, real_executable_file};

const CARGO_ENV: &str = r#"if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi"#;
const UV_ENV: &str = r#"if [ -f "$HOME/.local/bin/env" ]; then
  . "$HOME/.local/bin/env"
fi"#;
const FNM_BASH_ENV: &str = r#"FNM_PATH="$HOME/.local/share/fnm"
if [ -d "$FNM_PATH" ]; then
  export PATH="$FNM_PATH:$PATH"
  eval "$(fnm env --use-on-cd --shell bash)"
fi"#;
const FNM_ZSH_ENV: &str = r#"eval "$(fnm env --use-on-cd --shell zsh)""#;
const GO_ENV: &str = r#"export PATH="/usr/local/go/bin:$PATH""#;

pub fn rustup(host: &Host) -> Result<()> {
    let cargo_home = host.home().join(".cargo");
    let installed = cargo_home.join("bin/rustup");
    if !real_executable_file(&installed) {
        let installer = TempPath::new(host, "rustup")?;
        host.require(
            "rustup bootstrap download",
            "curl",
            [
                "--proto",
                "=https",
                "--tlsv1.2",
                "-sSf",
                "-o",
                &installer.path().to_string_lossy(),
                "https://sh.rustup.rs",
            ],
        )?;
        host.require(
            "rustup bootstrap",
            "env",
            [
                format!("CARGO_HOME={}", cargo_home.display()),
                "sh".to_owned(),
                installer.path().to_string_lossy().into_owned(),
                "-y".to_owned(),
                "--default-toolchain".to_owned(),
                "none".to_owned(),
            ],
        )?;
        if !real_executable_file(&installed) {
            bail!("rustup bootstrap did not publish the managed rustup executable");
        }
    }
    append_profile(host, CARGO_ENV)
}

pub fn fnm_bootstrap(host: &Host) -> Result<()> {
    if cfg!(target_os = "macos") {
        super::macos::install_formula(host, "fnm")?;
        return append_shell(host, FNM_ZSH_ENV);
    }

    let data_home = host.home().join(".local/share");
    let installed = data_home.join("fnm/fnm");
    if !real_executable_file(&installed) {
        let installer = TempPath::new(host, "fnm-install")?;
        host.require(
            "FNM bootstrap download",
            "curl",
            ["-fsSL", "-o", &installer.path().to_string_lossy(), "https://fnm.vercel.app/install"],
        )?;
        host.require(
            "FNM bootstrap",
            "env",
            [
                format!("XDG_DATA_HOME={}", data_home.display()),
                "bash".to_owned(),
                installer.path().to_string_lossy().into_owned(),
                "--skip-shell".to_owned(),
            ],
        )?;
        if !real_executable_file(&installed) {
            bail!("FNM bootstrap did not publish executable {}", installed.display());
        }
    }
    append_shell(host, FNM_BASH_ENV)
}

pub fn uv_bootstrap(host: &Host) -> Result<()> {
    let installed = host.home().join(".local/bin/uv");
    if !real_executable_file(&installed) {
        let installer = TempPath::new(host, "uv-install")?;
        host.require(
            "uv bootstrap download",
            "curl",
            ["-LsSf", "-o", &installer.path().to_string_lossy(), "https://astral.sh/uv/install.sh"],
        )?;
        host.require(
            "uv bootstrap",
            "env",
            ["UV_NO_MODIFY_PATH=1", "sh", installer.path().to_str().context("uv installer path is not UTF-8")?],
        )?;
        if !real_executable_file(&installed) {
            bail!("uv bootstrap did not publish executable {}", installed.display());
        }
    }
    append_profile(host, UV_ENV)
}

pub fn go_profile(host: &Host) -> Result<()> {
    append_profile(host, GO_ENV)
}

fn append_profile(host: &Host, snippet: &str) -> Result<()> {
    let name = if cfg!(target_os = "macos") { ".zprofile" } else { ".profile" };
    append_once(&host.home().join(name), snippet)
}

fn append_shell(host: &Host, snippet: &str) -> Result<()> {
    let name = if cfg!(target_os = "macos") { ".zshrc" } else { ".bashrc" };
    append_once(&host.home().join(name), snippet)
}

fn append_once(path: &Path, snippet: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create shell profile directory {}", parent.display()))?;
    }
    let current = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("read shell profile {}", path.display())),
    };
    if current.contains(snippet) {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open shell profile {}", path.display()))?;
    if !current.is_empty() && !current.ends_with('\n') {
        writeln!(file)?;
    }
    writeln!(file, "{snippet}")?;
    Ok(())
}
