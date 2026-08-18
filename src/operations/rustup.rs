use anyhow::{Result, bail};

use super::{Host, TempPath, regular_executable_file, require_regular_executable, shell::append_profile};

const CARGO_INIT: &str = r#"if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi"#;

pub fn install(host: &Host, selector: &str) -> Result<()> {
    let rustup_path = host.home().join(".cargo/bin/rustup");
    if !regular_executable_file(&rustup_path) {
        let installer = TempPath::new(host, "rustup")?;
        host.curl(
            "rustup installer download",
            "https://sh.rustup.rs",
            ["--proto", "=https", "--tlsv1.2", "--output", &installer.path().to_string_lossy()],
        )?;
        host.run(
            "rustup install",
            "sh",
            [
                installer.path().to_string_lossy().into_owned(),
                "-y".to_owned(),
                "--no-modify-path".to_owned(),
                "--default-toolchain".to_owned(),
                selector.to_owned(),
            ],
        )?;
        if !regular_executable_file(&rustup_path) {
            bail!("rustup installer did not publish the managed rustup executable");
        }
    } else {
        let rustup = require_regular_executable(
            &rustup_path,
            "managed tool executable path",
            "rustup toolchain install: rustup is unavailable after install",
        )?;
        host.run("rustup toolchain install", &rustup, ["toolchain", "install", "--no-update", "--", selector])?;
        host.run("rustup default", &rustup, ["default", "--", selector])?;
    }
    append_profile(host, CARGO_INIT)
}

pub(crate) fn update_toolchains(host: &Host) -> Result<()> {
    let rustup = require_regular_executable(
        &host.home().join(".cargo/bin/rustup"),
        "managed tool executable path",
        "Rust toolchain update: rustup is unavailable after install",
    )?;
    host.run("Rust toolchain update", &rustup, ["update"])?;
    Ok(())
}
