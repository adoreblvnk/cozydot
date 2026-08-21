use anyhow::Result;

use crate::operations::host;

pub(crate) fn install_extensions(extensions: &[String]) -> Result<()> {
    let program = if cfg!(target_os = "macos") {
        if host::output("code", ["--version"]).is_ok_and(|output| output.status.success()) {
            "code"
        } else {
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"
        }
    } else {
        "code"
    };
    for extension in extensions {
        host::run("VS Code extension install", program, ["--install-extension", extension.as_str()])?;
    }
    Ok(())
}
