use anyhow::{bail, Context, Result};
use serde_json::Value;

pub fn github_asset(input: &str, pattern: &str) -> Result<String> {
    let value: Value = serde_json::from_str(input).context("parse GitHub release JSON")?;
    let assets = value["assets"]
        .as_array()
        .context("GitHub response has no assets")?;
    let matches = assets
        .iter()
        .filter_map(|asset| {
            Some((
                asset["name"].as_str()?,
                asset["browser_download_url"].as_str()?,
            ))
        })
        .filter(|(name, _)| wildcard_match(pattern, name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [(_, url)] => Ok((*url).to_owned()),
        [] => bail!("no GitHub release asset matches {pattern:?}"),
        _ => bail!("multiple GitHub release assets match {pattern:?}"),
    }
}

pub fn latest_go(input: &str, requested: &str, arch: &str) -> Result<(String, String, String)> {
    let value: Value = serde_json::from_str(input).context("parse Go release JSON")?;
    let releases = value.as_array().context("Go metadata must be an array")?;
    let version = releases
        .iter()
        .filter_map(|release| release["version"].as_str())
        .filter(|version| stable_go_version(version))
        .map(|version| version.trim_start_matches("go"))
        .find(|version| {
            requested == "latest"
                || *version == requested
                || version
                    .strip_prefix(requested)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
        .context("Go metadata has no matching stable release")?;
    let filename = format!("go{version}.linux-{arch}.tar.gz");
    let checksum = releases
        .iter()
        .find(|release| release["version"].as_str() == Some(&format!("go{version}")))
        .and_then(|release| release["files"].as_array())
        .and_then(|files| {
            files
                .iter()
                .find(|file| file["filename"].as_str() == Some(&filename))
        })
        .and_then(|file| file["sha256"].as_str())
        .context("Go metadata has no matching archive checksum")?;
    Ok((version.to_owned(), filename, checksum.to_owned()))
}

pub fn gnome_version(input: &str, shell_version: &str) -> Result<u64> {
    let value: Value = serde_json::from_str(input).context("parse GNOME extension JSON")?;
    let versions = value["shell_version_map"]
        .as_object()
        .context("GNOME response has no shell_version_map")?;
    let mut candidate = shell_version;
    loop {
        if let Some(version) = versions
            .get(candidate)
            .and_then(|entry| entry["version"].as_u64())
        {
            return Ok(version);
        }
        let Some((parent, _)) = candidate.rsplit_once('.') else {
            bail!("GNOME response has no extension version for shell {shell_version}");
        };
        candidate = parent;
    }
}

pub fn gnome_shell_version(input: &str) -> Result<String> {
    input
        .split_whitespace()
        .map(|part| {
            part.trim_matches(|character: char| !character.is_ascii_digit() && character != '.')
        })
        .find(|part| {
            !part.is_empty()
                && part.split('.').all(|component| {
                    !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
                })
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
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let parts = pattern.split('*').collect::<Vec<_>>();
    let mut remaining = text;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let Some(at) = remaining.find(part) else {
            return false;
        };
        if index == 0 && !pattern.starts_with('*') && at != 0 {
            return false;
        }
        remaining = &remaining[at + part.len()..];
    }
    pattern.ends_with('*') || parts.last().is_some_and(|last| text.ends_with(last))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wildcard_is_anchored_and_supports_multiple_stars() {
        assert!(wildcard_match(
            "Obsidian-*.AppImage",
            "Obsidian-1.8.10.AppImage"
        ));
        assert!(wildcard_match(
            "*linux-amd64-*.deb",
            "gcm-linux-amd64-2.6.deb"
        ));
        assert!(!wildcard_match(
            "Obsidian-*.AppImage",
            "xObsidian-1.AppImage"
        ));
    }
    #[test]
    fn stable_go_versions_exclude_prereleases() {
        assert!(stable_go_version("go1.26.1"));
        assert!(!stable_go_version("go1.27rc2"));
    }

    #[test]
    fn partial_go_versions_resolve_to_a_matching_stable_patch() {
        let metadata = r#"[
            {"version":"go1.26.2","files":[{"filename":"go1.26.2.linux-amd64.tar.gz","sha256":"aa"}]},
            {"version":"go1.25.9","files":[{"filename":"go1.25.9.linux-amd64.tar.gz","sha256":"bb"}]}
        ]"#;
        assert_eq!(
            latest_go(metadata, "1.26", "amd64").unwrap(),
            (
                "1.26.2".into(),
                "go1.26.2.linux-amd64.tar.gz".into(),
                "aa".into()
            )
        );
        assert!(latest_go(metadata, "1.2", "amd64").is_err());
    }

    #[test]
    fn gnome_extension_version_matches_the_running_shell() {
        let metadata = r#"{"shell_version_map":{"48":{"version":13},"50":{"version":23}}}"#;
        let shell = gnome_shell_version("GNOME Shell 48.4\n").unwrap();
        assert_eq!(shell, "48.4");
        assert_eq!(gnome_version(metadata, &shell).unwrap(), 13);
        assert!(gnome_version(metadata, "47.2").is_err());
    }
}
