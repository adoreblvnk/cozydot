use super::Host;
use anyhow::Result;

pub(crate) fn validate_sudo_access(host: &Host) -> Result<()> {
    host.run("macOS sudo access", "sudo", ["-v"])?;
    Ok(())
}

pub(crate) fn install_command_line_tools_for_xcode(host: &Host) -> Result<()> {
    if host.output("xcode-select", ["-p"]).is_ok_and(|output| output.status.success()) {
        return Ok(());
    }
    // launch Apple's interactive installer; success only means it started
    host.run("Command Line Tools for Xcode install", "xcode-select", ["--install"])?;
    Ok(())
}
