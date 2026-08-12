pub(crate) fn latest_go(
    input: &str,
    requested: &str,
    arch: &str,
    target_os: &str,
) -> anyhow::Result<(String, String, String)> {
    use anyhow::Context;
    let value: serde_json::Value = serde_json::from_str(input).context("parse Go release JSON")?;
    let releases = value.as_array().context("Go metadata must be an array")?;
    let version = releases
        .iter()
        .filter_map(|release| release["version"].as_str())
        .filter(|v| stable_go_version(v))
        .map(|v| v.trim_start_matches("go"))
        .find(|v| {
            requested == "latest"
                || *v == requested
                || v.strip_prefix(requested).is_some_and(|rest| rest.starts_with('.'))
        })
        .context("Go metadata has no matching stable release")?;
    let filename = format!("go{version}.{target_os}-{arch}.tar.gz");
    let file = releases
        .iter()
        .find(|release| release["version"].as_str() == Some(&format!("go{version}")))
        .and_then(|release| release["files"].as_array())
        .and_then(|files| files.iter().find(|file| file["filename"].as_str() == Some(&filename)))
        .context("Go metadata has no matching architecture archive")?;
    let sha256 = file["sha256"].as_str().context("Go archive metadata has no SHA-256 checksum")?;
    Ok((version.to_owned(), filename, sha256.to_owned()))
}

pub(crate) fn gnome_version(input: &str, shell_version: &str) -> anyhow::Result<u64> {
    use anyhow::{Context, bail};
    let value: serde_json::Value = serde_json::from_str(input).context("parse GNOME extension JSON")?;
    let versions = value["shell_version_map"].as_object().context("GNOME response has no shell_version_map")?;
    let mut candidate = shell_version;
    loop {
        if let Some(version) = versions.get(candidate).and_then(|entry| entry["version"].as_u64()) {
            return Ok(version);
        }
        let Some((parent, _)) = candidate.rsplit_once('.') else {
            bail!("GNOME response has no extension version for shell {shell_version}");
        };
        candidate = parent;
    }
}

pub(crate) fn gnome_shell_version(input: &str) -> anyhow::Result<String> {
    use anyhow::Context;
    input
        .split_whitespace()
        .map(|part| part.trim_matches(|character: char| !character.is_ascii_digit() && character != '.'))
        .find(|part| {
            !part.is_empty()
                && part
                    .split('.')
                    .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .map(str::to_owned)
        .context("GNOME Shell version output has no numeric version")
}

fn stable_go_version(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("go") else {
        return false;
    };
    let parts = rest.split('.').collect::<Vec<_>>();
    (parts.len() == 2 || parts.len() == 3)
        && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}
