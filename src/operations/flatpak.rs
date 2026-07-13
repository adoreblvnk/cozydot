use super::Host;
use anyhow::{Context, Result};
use std::collections::BTreeSet;

const FLATHUB_NAME: &str = "flathub";
const FLATHUB_DESCRIPTOR_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
const FLATHUB_REPOSITORY_URL: &str = "https://dl.flathub.org/repo/";

pub fn ensure_flathub(host: &Host<'_>) -> Result<()> {
    let output = host.run(
        "flatpak",
        [
            "--user",
            "remotes",
            "--show-disabled",
            "--columns=name,url,options",
        ],
    )?;
    if !output.status.success() {
        anyhow::bail!(
            "Flathub remote query: flatpak failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if let Some((url, options)) = user_remotes(&output.stdout)?.get(FLATHUB_NAME) {
        let insecure = options.iter().find(|option| {
            matches!(
                **option,
                "disabled" | "no-gpg-verify" | "no-enumerate" | "no-deps" | "no-use-for-deps"
            )
        });
        if *url != FLATHUB_REPOSITORY_URL || insecure.is_some() {
            let options = options.iter().copied().collect::<Vec<_>>().join(",");
            anyhow::bail!(
                "Flathub remote mismatch: expected URL {FLATHUB_REPOSITORY_URL:?} with GPG verification, enumeration, and dependency use enabled; found URL {url:?} and options {options:?}. Repair or remove the per-user {FLATHUB_NAME:?} remote and retry"
            );
        }
        return Ok(());
    }
    host.require(
        "Flathub remote ensure",
        "flatpak",
        [
            "--user",
            "remote-add",
            "--if-not-exists",
            FLATHUB_NAME,
            FLATHUB_DESCRIPTOR_URL,
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
        "--app".into(),
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
        "--app".into(),
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
        installed.insert(app);
    }
    Ok(installed)
}

fn user_remotes(output: &[u8]) -> Result<std::collections::BTreeMap<&str, (&str, BTreeSet<&str>)>> {
    let output =
        std::str::from_utf8(output).context("flatpak returned non-UTF-8 per-user remote state")?;
    let mut remotes = std::collections::BTreeMap::new();
    for line in output.lines() {
        let mut fields = line.split('\t');
        let (Some(name), Some(url), Some(options), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            anyhow::bail!("flatpak returned malformed per-user remote state: {line:?}");
        };
        if name.is_empty() || url.is_empty() {
            anyhow::bail!("flatpak returned malformed per-user remote state: {line:?}");
        }
        if remotes
            .insert(
                name,
                (
                    url,
                    options
                        .split(',')
                        .filter(|option| !option.is_empty())
                        .collect(),
                ),
            )
            .is_some()
        {
            anyhow::bail!("flatpak returned duplicate per-user remote name: {name:?}");
        }
    }
    Ok(remotes)
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
