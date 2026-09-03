use anyhow::Result;

use super::*;

pub(crate) fn install_command_line_tools() -> Result<()> {
    if output("xcode-select", ["-p"]).is_ok_and(|output| output.status.success()) {
        return Ok(());
    }
    // launch Apple's interactive installer; success only means it started
    run("Command Line Tools for Xcode install", "xcode-select", ["--install"])?;
    Ok(())
}
