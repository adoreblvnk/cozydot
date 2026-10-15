use anyhow::{Context, Result};

use crate::operations::{host, toolchains::fnm};

pub(crate) fn install(entries: &[String]) -> Result<()> {
    let fnm = fnm::find_executable()?;
    let (program, prefix) = match fnm.as_deref() {
        Some(fnm) if !host::has_executable_on_path("skills") => (fnm, &["exec", "--using=default", "--", "skills"][..]),
        _ => ("skills", &[][..]),
    };

    let check_args = [prefix, &["--version"]].concat();
    host::run("Skills CLI version check", program, check_args)
        .context("Skills integration requires an existing usable skills CLI")?;

    for entry in entries {
        // `skills add` parses owner/repo, repo@skill & URLs natively
        let args = [prefix, &["add", entry, "--global", "--agent", "universal", "--yes", "--full-depth"]].concat();
        host::run(&format!("skill add {entry}"), program, args)?;
    }
    Ok(())
}
