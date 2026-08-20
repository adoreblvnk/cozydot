use super::super::host::Host;
use anyhow::Result;

const FLATHUB_NAME: &str = "flathub";
const FLATHUB_REPO_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";

pub fn add_flathub_remote(host: &Host) -> Result<()> {
    host.run(
        "Flathub remote add",
        "flatpak",
        ["--user", "remote-add", "--if-not-exists", FLATHUB_NAME, FLATHUB_REPO_URL],
    )?;
    Ok(())
}

pub fn install(host: &Host, refs: &[String]) -> Result<()> {
    if refs.is_empty() {
        return Ok(());
    }
    let mut args = vec!["--user", "install", "--noninteractive", "-y", "flathub", "--"];
    args.extend(refs.iter().map(String::as_str));
    host.run("Flatpak application install", "flatpak", args)?;
    Ok(())
}

pub fn update(host: &Host) -> Result<()> {
    host.run("Flatpak update", "flatpak", ["--user", "update", "--noninteractive", "-y"])?;
    Ok(())
}
