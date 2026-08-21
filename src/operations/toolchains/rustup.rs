use anyhow::{Result, bail};
use std::ffi::OsStr;

use crate::operations::host::{
    self, is_regular_executable, path_program, require_regular_executable, shell::append_profile, temp_path,
};

const CARGO_INIT: &str = r#"if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi"#;

pub fn install(selector: &str) -> Result<()> {
    let rustup_path = host::home()?.join(".cargo/bin/rustup");
    if !is_regular_executable(&rustup_path) {
        let installer = temp_path("rustup")?;
        host::curl(
            "rustup installer download",
            "https://sh.rustup.rs",
            [
                OsStr::new("--proto"),
                OsStr::new("=https"),
                OsStr::new("--tlsv1.2"),
                OsStr::new("--output"),
                installer.as_os_str(),
            ],
        )?;
        host::run(
            "rustup install",
            "sh",
            [
                installer.as_os_str(),
                OsStr::new("-y"),
                OsStr::new("--no-modify-path"),
                OsStr::new("--default-toolchain"),
                OsStr::new(selector),
            ],
        )?;
        if !is_regular_executable(&rustup_path) {
            bail!("rustup installer did not publish the managed rustup executable");
        }
    } else {
        let rustup = path_program(&rustup_path, "managed tool executable path")?;
        host::run("rustup toolchain install", &rustup, ["toolchain", "install", "--no-update", "--", selector])?;
        host::run("rustup default", &rustup, ["default", "--", selector])?;
    }
    append_profile(CARGO_INIT)
}

pub(crate) fn update_toolchains() -> Result<()> {
    let rustup = require_regular_executable(
        &host::home()?.join(".cargo/bin/rustup"),
        "managed tool executable path",
        "Rust toolchain update: rustup is unavailable after install",
    )?;
    host::run("Rust toolchain update", &rustup, ["update"])?;
    Ok(())
}
