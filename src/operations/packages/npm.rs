use anyhow::{Result, bail};

use super::super::{fnm, host::Host};

pub(crate) fn install(host: &Host, packages: &[String]) -> Result<()> {
    let Some(fnm) = fnm::find_executable(host)? else {
        bail!("npm install: managed fnm is unavailable after install");
    };
    let mut missing = Vec::new();
    for package in packages {
        let name = package_name(package);
        let output =
            host.output(&fnm, ["exec", "--using=default", "--", "npm", "list", "--global", "--depth=0", "--", name])?;
        if !output.status.success() {
            missing.push(package.clone());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let mut npm_args = vec!["install".to_owned(), "--global".into(), "--".into()];
    npm_args.extend(missing);
    run_npm_checked(host, &fnm, "npm package install", npm_args)?;
    Ok(())
}

pub(crate) fn update(host: &Host) -> Result<()> {
    let Some(fnm) = fnm::find_executable(host)? else { return Ok(()) };
    run_npm_checked(host, &fnm, "npm package update", ["update", "--global"])?;
    Ok(())
}

fn package_name(package: &str) -> &str {
    if package.starts_with('@') {
        let slash = package.find('/').unwrap_or(package.len());
        let version = package[slash..].find('@').map(|index| slash + index);
        return version.map_or(package, |index| &package[..index]);
    }
    package.split_once('@').map_or(package, |(name, _)| name)
}

fn run_npm_checked<I, S>(host: &Host, fnm: &str, label: &str, npm_args: I) -> Result<std::process::Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = vec!["exec".to_owned(), "--using=default".into(), "--".into(), "npm".into()];
    args.extend(npm_args.into_iter().map(|arg| arg.as_ref().to_owned()));
    host.run(label, fnm, args)
}
