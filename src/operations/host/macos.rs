use anyhow::Result;

use super::*;

pub(crate) fn is_cli_tools_installed() -> bool {
    output("xcode-select", ["-p"]).is_ok_and(|output| output.status.success())
}

pub(crate) fn install_cli_tools() -> Result<()> {
    // launch Apple's interactive installer; success only means it started
    run("Command Line Tools for Xcode install", "xcode-select", ["--install"])?;
    Ok(())
}
