use anyhow::{Context, Result, bail};

use super::super::{Host, executable_file};

pub(crate) fn install(host: &Host, packages: &[String]) -> Result<()> {
    let Some(fnm) = resolve_fnm(host)? else {
        bail!("npm package operation: managed fnm is unavailable after install");
    };
    let mut missing = Vec::new();
    for package in packages {
        let name = package_name(package);
        let output =
            host.run(&fnm, ["exec", "--using=default", "--", "npm", "list", "--global", "--depth=0", "--", name])?;
        if !output.status.success() {
            missing.push(package.clone());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let mut npm_args = vec!["install".to_owned(), "--global".into(), "--".into()];
    npm_args.extend(missing);
    run_npm_checked(host, &fnm, "npm package installation", npm_args)?;
    Ok(())
}

pub(crate) fn update_all(host: &Host) -> Result<()> {
    let Some(fnm) = resolve_fnm(host)? else { return Ok(()) };
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

fn resolve_fnm(host: &Host) -> Result<Option<String>> {
    if cfg!(target_os = "macos") {
        return super::super::macos::formula_executable(host, "fnm", "fnm").map(Some);
    }
    let data_home = host.home().join(".local/share");
    let managed = data_home.join("fnm/fnm");
    if executable_file(&managed) {
        return managed.to_str().map(str::to_owned).map(Some).context("managed fnm executable path is not UTF-8");
    }
    Ok(None)
}

fn run_npm_checked<I, S>(host: &Host, fnm: &str, operation: &str, npm_args: I) -> Result<std::process::Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = vec!["exec".to_owned(), "--using=default".into(), "--".into(), "npm".into()];
    args.extend(npm_args.into_iter().map(|arg| arg.as_ref().to_owned()));
    host.require(operation, fnm, args)
}
