use crate::style::STATUS;
use anyhow::Result;

pub(crate) fn self_update() -> Result<()> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("adoreblvnk")
        .repo_name("cozydot")
        .bin_name("cozydot")
        .show_download_progress(true)
        .current_version(env!("CARGO_PKG_VERSION"))
        .build()?
        .update()?;
    if status.is_up_to_date() {
        anstream::println!("{STATUS}✓{STATUS:#} cozydot is already up to date (v{})", status.version());
    } else {
        anstream::println!("{STATUS}✓{STATUS:#} Updated cozydot to v{}", status.version());
    }
    Ok(())
}
