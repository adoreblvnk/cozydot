use anyhow::{Context, Result};

use crate::operations::{host, toolchains::fnm};

pub(crate) fn install(packages: &[String]) -> Result<()> {
    let fnm = fnm::find_executable()?.context("npm install: managed fnm is unavailable after install")?;
    let mut missing = Vec::new();
    for package in packages {
        // split at the final @ so scoped names remain intact while versions/tags are ignored
        let name = package.rsplit_once('@').map_or(package.as_str(), |(name, _)| name);
        let name = if name.is_empty() { package } else { name };
        let args = ["exec", "--using=default", "--", "npm", "list", "--global", "--depth=0", "--", name];
        if !host::output(&fnm, args)?.status.success() {
            missing.push(package.as_str());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let mut args = vec!["exec", "--using=default", "--", "npm", "install", "--global", "--"];
    args.extend(missing);
    host::run("npm package install", &fnm, args)?;
    Ok(())
}

pub(crate) fn update() -> Result<()> {
    let Some(fnm) = fnm::find_executable()? else { return Ok(()) };
    host::run("npm package update", &fnm, ["exec", "--using=default", "--", "npm", "update", "--global"])?;
    Ok(())
}
