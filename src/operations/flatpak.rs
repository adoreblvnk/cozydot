use super::Host;
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};

const FLATHUB_NAME: &str = "flathub";
const FLATHUB_DESCRIPTOR_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
const FLATHUB_REPOSITORY_URL: &str = "https://dl.flathub.org/repo/";

pub fn ensure_flathub(host: &Host<'_>) -> Result<()> {
    if !validate_flathub(&inspect_user_remotes(host)?)? {
        host.require(
            "Flathub remote ensure",
            "flatpak",
            ["--user", "remote-add", FLATHUB_NAME, FLATHUB_DESCRIPTOR_URL],
        )?;
        require_flathub(&inspect_user_remotes(host)?)?;
    }
    host.require(
        "Flathub dependency use enablement",
        "flatpak",
        ["--user", "remote-modify", "--use-for-deps", FLATHUB_NAME],
    )?;
    require_flathub(&inspect_user_remotes(host)?)?;
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

struct UserRemote {
    url: String,
    options: BTreeSet<String>,
    filter: String,
}

fn inspect_user_remotes(host: &Host<'_>) -> Result<BTreeMap<String, UserRemote>> {
    let output = host.run(
        "flatpak",
        [
            "--user",
            "remotes",
            "--show-disabled",
            "--columns=name,url,options,filter",
        ],
    )?;
    if !output.status.success() {
        anyhow::bail!(
            "Flathub remote query: flatpak failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    user_remotes(&output.stdout)
}

fn require_flathub(remotes: &BTreeMap<String, UserRemote>) -> Result<()> {
    if !validate_flathub(remotes)? {
        anyhow::bail!(
            "Flathub remote mismatch: expected the per-user {FLATHUB_NAME:?} remote to exist after mutation"
        );
    }
    Ok(())
}

fn validate_flathub(remotes: &BTreeMap<String, UserRemote>) -> Result<bool> {
    let Some(remote) = remotes.get(FLATHUB_NAME) else {
        return Ok(false);
    };
    let insecure = remote.options.iter().find(|option| {
        matches!(
            option.as_str(),
            "disabled" | "no-gpg-verify" | "no-enumerate"
        )
    });
    if remote.url != FLATHUB_REPOSITORY_URL || insecure.is_some() || remote.filter != "-" {
        let options = remote.options.iter().cloned().collect::<Vec<_>>().join(",");
        anyhow::bail!(
            "Flathub remote mismatch: expected URL {FLATHUB_REPOSITORY_URL:?} with GPG verification and enumeration enabled and no local filter; found URL {:?}, options {options:?}, and filter {:?}. Repair or remove the per-user {FLATHUB_NAME:?} remote and retry",
            remote.url,
            remote.filter
        );
    }
    Ok(true)
}

fn user_remotes(output: &[u8]) -> Result<BTreeMap<String, UserRemote>> {
    let output =
        std::str::from_utf8(output).context("flatpak returned non-UTF-8 per-user remote state")?;
    let mut remotes = BTreeMap::new();
    for line in output.lines() {
        let mut fields = line.split('\t');
        let (Some(name), Some(url), Some(options), Some(filter), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            anyhow::bail!("flatpak returned malformed per-user remote state: {line:?}");
        };
        if name.is_empty() || url.is_empty() || filter.is_empty() || url::Url::parse(url).is_err() {
            anyhow::bail!("flatpak returned malformed per-user remote state: {line:?}");
        }
        let options = if options.is_empty() {
            BTreeSet::new()
        } else {
            let parsed = options.split(',').collect::<BTreeSet<_>>();
            if parsed.contains("") || parsed.len() != options.split(',').count() {
                anyhow::bail!("flatpak returned malformed per-user remote state: {line:?}");
            }
            parsed.into_iter().map(str::to_owned).collect()
        };
        if remotes
            .insert(
                name.to_owned(),
                UserRemote {
                    url: url.to_owned(),
                    options,
                    filter: filter.to_owned(),
                },
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
