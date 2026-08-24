use anyhow::Result;

use super::*;

pub(crate) fn is_enabled_or_active(unit: &str) -> Result<bool> {
    // disabled but still-running units must be stopped too
    let enabled = output("systemctl", ["is-enabled", unit])?.status.success();
    let active = output("systemctl", ["is-active", unit])?.status.success();
    Ok(enabled || active)
}
