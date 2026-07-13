use super::Host;
use anyhow::{Context, Result};
use std::collections::BTreeSet;

const FLATHUB_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";

pub fn ensure_flathub(host: &Host<'_>) -> Result<()> {
    host.require(
        "Flathub remote ensure",
        "flatpak",
        [
            "--user",
            "remote-add",
            "--if-not-exists",
            "flathub",
            FLATHUB_URL,
        ],
    )?;
    Ok(())
}

pub fn ensure_apps(host: &Host<'_>, refs: &[String]) -> Result<()> {
    validate_refs(refs)?;
    let output = host.run(
        "flatpak",
        ["--user", "list", "--app", "--columns=application"],
    )?;
    if !output.status.success() {
        anyhow::bail!(
            "Flatpak installed application query: flatpak failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let installed = installed_apps(&output.stdout)?;
    let missing = refs
        .iter()
        .filter(|app| !installed.contains(app.as_str()))
        .cloned()
        .collect::<Vec<_>>();
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
    host.require("Flatpak application installation", "flatpak", args)?;
    Ok(())
}

pub fn update_apps(host: &Host<'_>, refs: &[String]) -> Result<()> {
    validate_refs(refs)?;
    let mut args = vec![
        "--user".to_owned(),
        "update".into(),
        "--noninteractive".into(),
        "-y".into(),
        "--".into(),
    ];
    args.extend(refs.iter().cloned());
    host.require("Flatpak configured application update", "flatpak", args)?;
    Ok(())
}

fn validate_refs(refs: &[String]) -> Result<()> {
    if refs.is_empty() {
        anyhow::bail!("Flatpak application sequence must not be empty");
    }
    let mut unique = BTreeSet::new();
    for app in refs {
        validate_app_id(app)?;
        if !unique.insert(app.as_str()) {
            anyhow::bail!("duplicate Flatpak application ID: {app:?}");
        }
    }
    Ok(())
}

fn installed_apps(output: &[u8]) -> Result<BTreeSet<&str>> {
    let output = std::str::from_utf8(output)
        .context("flatpak returned non-UTF-8 installed application state")?;
    let mut installed = BTreeSet::new();
    for app in output.lines() {
        validate_app_id(app).map_err(|_| {
            anyhow::anyhow!("flatpak returned malformed installed application ID: {app:?}")
        })?;
        if !installed.insert(app) {
            anyhow::bail!("flatpak returned duplicate installed application ID: {app:?}");
        }
    }
    Ok(installed)
}

fn validate_app_id(app: &str) -> Result<()> {
    let mut count = 0;
    for segment in app.split('.') {
        count += 1;
        let mut bytes = segment.bytes();
        let valid = bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if !valid {
            anyhow::bail!("invalid canonical Flatpak application ID: {app:?}");
        }
    }
    if count < 3 {
        anyhow::bail!("invalid canonical Flatpak application ID: {app:?}");
    }
    Ok(())
}
