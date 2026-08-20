use anyhow::{Context, Result, bail};

use std::path::Path;

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

pub(crate) fn install_cargo_update(host: &Host) -> Result<()> {
    let cargo_home = host.home().join(".cargo");
    if executable_file(&cargo_home.join("bin/cargo-install-update")) {
        return Ok(());
    }
    let binstall = cargo_home.join("bin/cargo-binstall");
    let program = if cfg!(target_os = "macos") {
        super::homebrew::formula_executable(host, "cargo-binstall", "cargo-binstall")?
    } else {
        binstall
            .to_str()
            .with_context(|| format!("cargo-binstall executable path is not UTF-8: {}", binstall.display()))?
            .to_owned()
    };
    host.run("cargo-update install", &program, ["--no-confirm", "cargo-update"])?;
    Ok(())
}

pub(crate) fn install_crates(host: &Host, crates: &[String]) -> Result<()> {
    let cargo_home = host.home().join(".cargo");
    let cargo = path_program(&cargo_home.join("bin/cargo"), "managed Cargo executable path")?;
    let output = host.run("Cargo installed package query", &cargo, ["install", "--list"])?;
    let installed = installed_crates(&output.stdout)?;
    let missing = crates.iter().filter(|name| !installed.contains(crate_name(name))).cloned().collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    let binstall = resolve_binstall(host, &cargo_home)?;
    let mut args = vec!["--no-confirm".to_owned(), "--".into()];
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
    let mut installed = std::collections::BTreeSet::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let header = line.strip_suffix(':').context("cargo install --list returned malformed output")?;
        let mut fields = header.splitn(3, char::is_whitespace).filter(|field| !field.is_empty());
        let name = fields.next().context("cargo install --list returned malformed output")?;
        let version = fields.next().context("cargo install --list returned malformed output")?;
        if !version.starts_with('v') || name.chars().any(char::is_control) {
            bail!("cargo install --list returned malformed output");
        }
        match fields.next() {
            None => {
                installed.insert(name.to_owned());
            }
            Some(source)
                if source.starts_with('(') && source.ends_with(')') && !source.chars().any(char::is_control) => {}
            Some(_) => bail!("cargo install --list returned malformed output"),
        }
    }
    Ok(installed)
}

fn crate_name(name: &str) -> &str {
    name.split_once('@').map_or(name, |(name, _)| name)
}

fn resolve_binstall(host: &Host, cargo_home: &Path) -> Result<String> {
    if cfg!(target_os = "macos") {
        return super::homebrew::formula_executable(host, "cargo-binstall", "cargo-binstall");
    }
    let managed = cargo_home.join("bin/cargo-binstall");
    if executable_file(&managed) {
        return path_program(&managed, "cargo-binstall executable path");
    }
    bail!("cargo-binstall: managed executable is unavailable after install")
}
