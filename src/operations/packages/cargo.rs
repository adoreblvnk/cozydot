use anyhow::{Context, Result};

use super::super::host::{Host, executable_file, path_program};

pub(crate) fn install_binstall(host: &Host) -> Result<()> {
    if cfg!(target_os = "macos") {
        return super::homebrew::install_packages(host, &["cargo-binstall".to_owned()], &[]);
    }
    let cargo_home = host.home().join(".cargo");
    let cargo_binstall = cargo_home.join("bin/cargo-binstall");
    if executable_file(&cargo_binstall) {
        return Ok(());
    }
    let cargo = cargo_home.join("bin/cargo");
    let program = cargo.to_str().with_context(|| format!("Cargo executable path is not UTF-8: {}", cargo.display()))?;
    host.run("cargo-binstall install", program, ["install", "cargo-binstall", "--locked"])?;
    Ok(())
}

pub(crate) fn install_crates(host: &Host, crates: &[String]) -> Result<()> {
    let cargo_home = host.home().join(".cargo");
    let cargo = path_program(&cargo_home.join("bin/cargo"), "managed Cargo executable path")?;
    let output = host.run("Cargo installed package query", &cargo, ["install", "--list"])?;
    let installed = installed_crates(&output.stdout)?;
    let mut missing = Vec::new();
    for name in crates {
        if !installed.contains(name.as_str()) {
            missing.push(name.as_str());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let binstall = if cfg!(target_os = "macos") {
        super::homebrew::formula_executable(host, "cargo-binstall", "cargo-binstall")?
    } else {
        path_program(&cargo_home.join("bin/cargo-binstall"), "cargo-binstall executable path")?
    };
    let mut args = vec!["--no-confirm", "--"];
    args.extend(missing);
    host.run("cargo-binstall install", &binstall, args)?;
    Ok(())
}

pub(crate) fn update_crates(host: &Host) -> Result<()> {
    let program = host.home().join(".cargo/bin/cargo-install-update");
    if !executable_file(&program) {
        return Ok(());
    }
    let program = path_program(&program, "managed cargo-install-update executable path")?;
    host.run("Cargo crate update", &program, ["-a"])?;
    Ok(())
}

fn installed_crates(output: &[u8]) -> Result<std::collections::BTreeSet<String>> {
    let output = std::str::from_utf8(output).context("cargo install --list returned non-UTF-8 output")?;
    let installed = output.lines().filter_map(|line| {
        let (name, version) = line.split_once(" v")?;
        let valid_name = !name.is_empty() && !name.contains(char::is_whitespace);
        let valid_version = version.starts_with(|character: char| character.is_ascii_digit());
        (valid_name && valid_version).then(|| name.to_owned())
    });
    Ok(installed.collect())
}
