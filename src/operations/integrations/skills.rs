use anyhow::Result;

use crate::{
    operations::{host, toolchains::fnm},
    style::WARNING,
};

pub(crate) fn install(entries: &[String]) -> Result<()> {
    let fnm = fnm::find_executable()?;
    let (program, prefix) = match fnm.as_deref() {
        Some(fnm) if !host::has_executable_on_path("skills") => (fnm, &["exec", "--using=default", "--", "skills"][..]),
        _ => ("skills", &[][..]),
    };

    let check_args = [prefix, &["--version"]].concat();
    if let Err(error) = host::run("Skills CLI version check", program, check_args) {
        anstream::eprintln!("{WARNING}warning:{WARNING:#} skills CLI is unavailable: {error}");
        return Ok(());
    }

    for entry in entries {
        // `skills add` parses owner/repo, repo@skill & URLs natively
        let args = [prefix, &["add", entry, "--global", "--agent", "universal", "--yes", "--full-depth"]].concat();
        if let Err(error) = host::run(&format!("skill add {entry}"), program, args) {
            anstream::eprintln!("{WARNING}warning:{WARNING:#} failed to install skill {entry}: {error}");
        }
    }
    Ok(())
}
