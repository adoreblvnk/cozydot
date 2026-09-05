use anyhow::Result;

use crate::operations::host;

pub(crate) fn any_missing(extensions: &[String]) -> Result<bool> {
    if extensions.is_empty() {
        return Ok(false);
    }
    let output = host::output(code_executable(), ["--list-extensions"])?;
    if !output.status.success() {
        return Ok(true);
    }
    let stdout = std::str::from_utf8(&output.stdout).unwrap_or("");
    let installed: Vec<String> = stdout.lines().map(|line| line.trim().to_lowercase()).collect();
    Ok(extensions.iter().any(|ext| !installed.contains(&ext.to_lowercase())))
}

pub(crate) fn install_extensions(extensions: &[String]) -> Result<()> {
    let code = code_executable();
    for extension in extensions {
        host::run("VS Code extension install", code, ["--install-extension", extension.as_str()])?;
    }
    Ok(())
}

fn code_executable() -> &'static str {
    if cfg!(target_os = "macos") {
        // GUI installs may expose code only inside the application bundle
        if host::output("code", ["--version"]).is_ok_and(|output| output.status.success()) {
            "code"
        } else {
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"
        }
    } else {
        "code"
    }
}
