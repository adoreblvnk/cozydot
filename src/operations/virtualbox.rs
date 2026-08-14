use anyhow::Result;

use super::{Host, users};

pub(crate) fn virtualbox_group(host: &Host) -> Result<()> {
    users::ensure_product_group(host, "VirtualBox", "VBoxManage", "vboxusers")
}
