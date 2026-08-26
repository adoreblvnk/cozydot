use anyhow::Result;

use crate::operations::host;

pub(crate) fn install(entries: &[String]) -> Result<()> {
    host::require_cli("Skills", "skills")?;
    for entry in entries {
        // `skills add` parses owner/repo, repo@skill & URLs natively
        let args = ["add", entry, "--global", "--agent", "universal", "--yes"];
        host::run(&format!("skill add {entry}"), "skills", args)?;
    }
    Ok(())
}
