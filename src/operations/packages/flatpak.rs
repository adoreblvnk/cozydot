use super::super::Host;
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
    let mut missing = Vec::new();
    for reference in refs {
        let output = host.output("flatpak", ["--user", "info", "--show-ref", "--", reference])?;
        if !output.status.success() {
            missing.push(reference.clone());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let mut args = vec![
        "--user".to_owned(),
        "install".into(),
        "--noninteractive".into(),
        "-y".into(),
        "flathub".into(),
        "--".into(),
    ];
    args.extend(missing);
    host.run("Flatpak application install", "flatpak", args)?;
    Ok(())
}

pub fn update(host: &Host) -> Result<()> {
    host.run("Flatpak update", "flatpak", ["--user", "update", "--noninteractive", "-y"])?;
    Ok(())
}
