use super::super::Host;
use anyhow::Result;

const FLATHUB_NAME: &str = "flathub";
const FLATHUB_DESCRIPTOR_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
const FLATHUB_URL: &str = "https://dl.flathub.org/repo/";

pub fn add_flathub_remote(host: &Host) -> Result<()> {
    host.run_checked(
        "Flathub remote add",
        "flatpak",
        ["--user", "remote-add", "--if-not-exists", FLATHUB_NAME, FLATHUB_DESCRIPTOR_URL],
    )?;
    let url_arg = format!("--url={FLATHUB_URL}");
    host.run_checked(
        "Flathub remote modify",
        "flatpak",
        [
            "--user",
            "remote-modify",
            &url_arg,
            "--gpg-verify",
            "--enumerate",
            "--use-for-deps",
            "--enable",
            "--no-filter",
            FLATHUB_NAME,
        ],
    )?;
    Ok(())
}

pub fn install(host: &Host, refs: &[String]) -> Result<()> {
    let mut missing = Vec::new();
    for reference in refs {
        let output = host.run("flatpak", ["--user", "info", "--show-ref", "--", reference])?;
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
        "--app".into(),
        "--noninteractive".into(),
        "-y".into(),
        "flathub".into(),
        "--".into(),
    ];
    args.extend(missing);
    host.run_checked("Flatpak application install", "flatpak", args)?;
    Ok(())
}

pub fn update(host: &Host) -> Result<()> {
    host.run_checked("Flatpak application update", "flatpak", ["--user", "update", "--app", "--noninteractive", "-y"])?;
    Ok(())
}
