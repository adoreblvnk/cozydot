use anyhow::{Context, Result, ensure};

use crate::operations::host::{
    self, is_regular_executable, require_regular_executable, shell::append_profile, temp_path,
};

const CARGO_INIT: &str = r#"if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi"#;

pub fn is_installed() -> Result<bool> {
    Ok(is_regular_executable(&host::home()?.join(".cargo/bin/rustup")))
}

pub fn install(selector: &str) -> Result<()> {
    let installer = temp_path("rustup", "")?;
    let path = installer.to_str().context("rustup installer path is not UTF-8")?;
    let curl_args = ["--proto", "=https", "--tlsv1.2", "--output", path];
    host::curl("rustup installer download", "https://sh.rustup.rs", curl_args)?;
    // disable installer PATH edits because cozydot appends its own profile snippet
    host::run("rustup install", "sh", [path, "-y", "--no-modify-path", "--default-toolchain", selector])?;
    let rustup_path = host::home()?.join(".cargo/bin/rustup");
    ensure!(is_regular_executable(&rustup_path), "rustup installer did not publish the managed rustup executable");
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
