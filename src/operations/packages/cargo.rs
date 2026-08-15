use anyhow::{Context, Result, bail};

use std::path::Path;

use super::super::{Host, executable_file, path_program};

pub(crate) fn install(host: &Host, packages: &[String]) -> Result<()> {
    let cargo_home = host.home().join(".cargo");
    let cargo = path_program(&cargo_home.join("bin/cargo"), "managed Cargo executable path")?;
    let output = host.require("Cargo installed package query", &cargo, ["install", "--list"])?;
    let installed = installed_crates(&output.stdout)?;
    let missing =
        packages.iter().filter(|package| !installed.contains(crate_name(package))).cloned().collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    let binstall = resolve_binstall(host, &cargo_home)?;
    let mut args = vec!["--no-confirm".to_owned(), "--".into()];
    args.extend(missing);
    host.require("cargo-binstall install", &binstall, args)?;
    Ok(())
}

pub(crate) fn update_all(host: &Host) -> Result<()> {
    let program = host.home().join(".cargo/bin/cargo-install-update");
    if !executable_file(&program) {
        return Ok(());
    }
    let program = path_program(&program, "managed cargo-install-update executable path")?;
    host.require("Cargo package update", &program, ["-a"])?;
    Ok(())
}

fn installed_crates(output: &[u8]) -> Result<std::collections::BTreeSet<String>> {
    let output = std::str::from_utf8(output).context("cargo install --list returned non-UTF-8 state")?;
    let mut installed = std::collections::BTreeSet::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let header = line.strip_suffix(':').context("cargo install --list returned malformed state")?;
        let mut fields = header.splitn(3, char::is_whitespace).filter(|field| !field.is_empty());
        let name = fields.next().context("cargo install --list returned malformed state")?;
        let version = fields.next().context("cargo install --list returned malformed state")?;
        if !version.starts_with('v') || name.chars().any(char::is_control) {
            bail!("cargo install --list returned malformed state");
        }
        match fields.next() {
            None => {
                installed.insert(name.to_owned());
            }
            Some(source)
                if source.starts_with('(') && source.ends_with(')') && !source.chars().any(char::is_control) => {}
            Some(_) => bail!("cargo install --list returned malformed state"),
        }
    }
    Ok(installed)
}

fn crate_name(package: &str) -> &str {
    package.split_once('@').map_or(package, |(name, _)| name)
}

fn resolve_binstall(host: &Host, cargo_home: &Path) -> Result<String> {
    if cfg!(target_os = "macos") {
        return super::super::macos::formula_executable(host, "cargo-binstall", "cargo-binstall");
    }
    let managed = cargo_home.join("bin/cargo-binstall");
    if executable_file(&managed) {
        return path_program(&managed, "cargo-binstall executable path");
    }
    bail!("Cargo package operation: managed cargo-binstall is unavailable after install")
}
