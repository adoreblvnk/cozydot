use anyhow::{Context, Result};

use super::Host;

pub(crate) fn install_extensions(host: &Host, extensions: &[String]) -> Result<()> {
    let program = if cfg!(target_os = "macos") {
        [
            "code",
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
            "/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/bin/code",
        ]
        .into_iter()
        .find(|candidate| host.run(candidate, ["--version"]).is_ok_and(|output| output.status.success()))
        .context("VS Code integration requires the VS Code CLI; install the `code` shell command or the visual-studio-code cask")?
    } else {
        "code"
    };
    for extension in extensions {
        host.run_checked("VS Code extension install", program, ["--install-extension", extension.as_str()])?;
    }
    Ok(())
}
