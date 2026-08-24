use anyhow::{Context, Result};
use regex::Regex;

use super::super::host::{self, is_executable, path_program};

pub(crate) fn install_binstall() -> Result<()> {
    if cfg!(target_os = "macos") {
        return super::homebrew::install_packages(&["cargo-binstall".to_owned()], &[]);
    }
    let cargo_home = host::home()?.join(".cargo");
    let cargo_binstall = cargo_home.join("bin/cargo-binstall");
    if is_executable(&cargo_binstall) {
        return Ok(());
    }
    let cargo = cargo_home.join("bin/cargo");
    let program = cargo.to_str().with_context(|| format!("Cargo executable path is not UTF-8: {}", cargo.display()))?;
    host::run("cargo-binstall install", program, ["install", "cargo-binstall", "--locked"])?;
    Ok(())
}

pub(crate) fn install_crates(crates: &[String]) -> Result<()> {
    let cargo_home = host::home()?.join(".cargo");
    let cargo = path_program(&cargo_home.join("bin/cargo"), "managed Cargo executable path")?;
    let output = host::run("Cargo installed package query", &cargo, ["install", "--list"])?;
    let installed = installed_crates(&output.stdout)?;
    let mut missing = Vec::new();
    for name in crates {
        if !installed.contains(name) {
            missing.push(name.as_str());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let binstall = if cfg!(target_os = "macos") {
        super::homebrew::executable_path("cargo-binstall", "cargo-binstall")?
    } else {
        path_program(&cargo_home.join("bin/cargo-binstall"), "cargo-binstall executable path")?
    };
    let mut args = vec!["--no-confirm", "--"];
    args.extend(missing);
    host::run("cargo-binstall install", &binstall, args)?;
    Ok(())
}

pub(crate) fn update_crates() -> Result<()> {
    let program = host::home()?.join(".cargo/bin/cargo-install-update");
    if !is_executable(&program) {
        return Ok(());
    }
    let program = path_program(&program, "managed cargo-install-update executable path")?;
    host::run("Cargo crate update", &program, ["-a"])?;
    Ok(())
}

fn installed_crates(output: &[u8]) -> Result<Vec<String>> {
    let output = std::str::from_utf8(output).context("cargo install --list returned non-UTF-8 output")?;
    // top-level records start with `<crate> v<version>`; indented binary lines must not match
    let pattern = Regex::new(r"^(\S+) v[0-9].*$")?;
    let installed = output.lines().filter_map(|line| pattern.captures(line).map(|captures| captures[1].to_owned()));
    Ok(installed.collect())
}
