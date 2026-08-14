use anyhow::{Result, bail};

use super::{Host, TempPath, path_program, real_executable_file, shell::append_profile};

const CARGO_INIT: &str = r#"if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi"#;

pub fn install_rustup(host: &Host) -> Result<()> {
    let cargo_home = host.home().join(".cargo");
    let installed = cargo_home.join("bin/rustup");
    if !real_executable_file(&installed) {
        let installer = TempPath::new(host, "rustup")?;
        host.require(
            "rustup installer download",
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

pub(crate) fn install_default_rust_toolchain(host: &Host, selector: &str) -> Result<()> {
    let rustup = managed_executable(host, "Rust toolchain operation: rustup is unavailable after install")?;
    host.require("rustup toolchain install", &rustup, rust_install_args(selector))?;
    host.require("rustup default", &rustup, ["default", "--", selector])?;
    Ok(())
}

pub(crate) fn update_rust(host: &Host) -> Result<()> {
    let rustup = managed_executable(host, "Rust toolchain update: rustup is unavailable after install")?;
    host.require("Rust toolchain update", &rustup, ["update"])?;
    Ok(())
}

fn managed_executable(host: &Host, message: &str) -> Result<String> {
    let path = host.home().join(".cargo/bin/rustup");
    if real_executable_file(&path) {
        return path_program(&path, "managed tool executable path");
    }
    bail!("{message}")
}

fn rust_install_args(toolchain: &str) -> [&str; 7] {
    ["toolchain", "install", "--profile", "minimal", "--no-self-update", "--", toolchain]
}
