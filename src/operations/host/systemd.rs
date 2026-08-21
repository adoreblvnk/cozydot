use anyhow::Result;

use super::Host;

pub(crate) fn is_enabled_or_active(host: &Host, unit: &str) -> Result<bool> {
    let enabled = host.output("systemctl", ["is-enabled", unit])?.status.success();
    let active = host.output("systemctl", ["is-active", unit])?.status.success();
    Ok(enabled || active)
}
