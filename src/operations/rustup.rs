use anyhow::{Result, bail};

use super::{Host, TempPath, real_executable_file, required_real_executable, shell::append_profile};

const CARGO_INIT: &str = r#"if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi"#;

pub fn install_rustup(host: &Host) -> Result<()> {
    let cargo_home = host.home().join(".cargo");
    let installed = cargo_home.join("bin/rustup");
    if !real_executable_file(&installed) {
        let installer = TempPath::new(host, "rustup")?;
        host.curl(
            "rustup installer download",
            "https://sh.rustup.rs",
            ["--proto", "=https", "--tlsv1.2", "--output", &installer.path().to_string_lossy()],
        )?;
        host.require(
            "rustup install",
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
            bail!("rustup installer did not publish the managed rustup executable");
        }
    }
    append_profile(host, CARGO_INIT)
}

pub(crate) fn install_default_toolchain(host: &Host, selector: &str) -> Result<()> {
    let rustup = required_real_executable(
        &host.home().join(".cargo/bin/rustup"),
        "managed tool executable path",
        "Rust toolchain operation: rustup is unavailable after install",
    )?;
    host.require(
        "rustup toolchain install",
        &rustup,
        ["toolchain", "install", "--profile", "minimal", "--no-self-update", "--", selector],
    )?;
    host.require("rustup default", &rustup, ["default", "--", selector])?;
    Ok(())
}

pub(crate) fn update_rust(host: &Host) -> Result<()> {
    let rustup = required_real_executable(
        &host.home().join(".cargo/bin/rustup"),
        "managed tool executable path",
        "Rust toolchain update: rustup is unavailable after install",
    )?;
    host.require("Rust toolchain update", &rustup, ["update"])?;
    Ok(())
}
