use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct GithubRelease {
    pub assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
pub(crate) struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

pub(crate) fn select_gnome_extension_version(input: &str, shell_version: &str) -> anyhow::Result<u64> {
    use anyhow::{Context, bail};
    use std::collections::HashMap;

    #[derive(Deserialize)]
    struct Response {
        shell_version_map: HashMap<String, ExtensionVersion>,
    }

    #[derive(Deserialize)]
    struct ExtensionVersion {
        version: u64,
    }

    let response: Response = serde_json::from_str(input).context("parse GNOME extension JSON")?;
    let mut candidate = shell_version;
    loop {
        if let Some(extension) = response.shell_version_map.get(candidate) {
            return Ok(extension.version);
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
