use super::*;

pub(crate) fn is_cli_tools_installed() -> bool {
    output("xcode-select", ["-p"]).is_ok_and(|output| output.status.success())
}
