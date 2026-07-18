use super::*;

pub(super) fn resolve(host: &Host<'_>, operation: &BinaryPackageOperation) -> Result<Candidate> {
    match &operation.source {
        BinarySourceOperation::ChecksummedUrl { url, sha256 } => Ok(Candidate {
            tag: None,
            asset_name: fixed_asset_name(url.as_str()),
            url: url.clone(),
            effective: Some(*sha256),
            retry_actual: None,
        }),
        BinarySourceOperation::GithubLatest {
            repository,
            selector,
            sha256,
        } => {
            let endpoint = format!(
                "https://api.github.com/repos/{}/releases/latest",
                repository.as_str()
            );
            let output = host.require(
                "resolve binary package release",
                "curl",
                [
                    "--proto",
                    "=https",
                    "--location",
                    "--fail",
                    "--silent",
                    "--show-error",
                    "--retry",
                    "3",
                    "--retry-all-errors",
                    "--header",
                    GITHUB_ACCEPT,
                    "--header",
                    GITHUB_API_VERSION,
                    "--header",
                    USER_AGENT,
                    &endpoint,
                ],
            )?;
            select_asset(
                std::str::from_utf8(&output.stdout)
                    .context("GitHub release metadata is not UTF-8")?,
                selector,
                *sha256,
                operation,
            )
        }
    }
}
pub(super) fn select_asset(
    input: &str,
    selector: &BinaryPackageSelector,
    declared: Option<BinarySha256>,
    operation: &BinaryPackageOperation,
) -> Result<Candidate> {
    let value: Value = serde_json::from_str(input).context("parse GitHub release JSON")?;
    let object = value
        .as_object()
        .context("GitHub release JSON must be an object")?;
    for field in ["draft", "prerelease"] {
        match object.get(field) {
            Some(Value::Bool(false)) => {}
            Some(Value::Bool(true)) => bail!("GitHub release {field} must be false"),
            Some(_) => bail!("GitHub release {field} must be boolean false"),
            None => bail!("GitHub release is missing {field}"),
        }
    }
    let tag = object
        .get("tag_name")
        .context("GitHub release is missing tag_name")?
        .as_str()
        .context("GitHub release tag_name must be a string")?;
    validate_safe_scalar(tag, "release tag")?;
    let assets = object
        .get("assets")
        .context("GitHub release is missing assets")?
        .as_array()
        .context("GitHub release assets must be an array")?;
    let mut named = Vec::new();
    for (index, value) in assets.iter().enumerate() {
        named.push((index, value, parse_asset_name(value, index)?));
    }
    let pattern = Regex::new(&selector.pattern).context("compile binary asset regex")?;
    let matches = named
        .into_iter()
        .filter(|(_, _, name)| pattern.is_match(name))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        bail!(
            "binary package {:?} ({}) selector matched {} assets",
            operation.name,
            operation.architecture.canonical(),
            matches.len()
        );
    }
    let (index, asset, name) = matches[0];
    let object = asset
        .as_object()
        .context("selected GitHub release asset must be an object")?;
    let url = HttpsUrl::parse(
        object
            .get("browser_download_url")
            .with_context(|| {
                format!("GitHub release asset {index} is missing browser_download_url")
            })?
            .as_str()
            .with_context(|| {
                format!("GitHub release asset {index} browser_download_url must be a string")
            })?,
    )?;
    let api = match object.get("digest") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(BinarySha256(parse_digest(value)?)),
        Some(_) => bail!("GitHub release asset {index} digest must be a string or null"),
    };
    if declared.is_some() && api.is_some() && declared != api {
        bail!("declared and GitHub API SHA-256 checksums differ");
    }
    Ok(Candidate {
        tag: Some(tag.into()),
        asset_name: name.into(),
        url,
        effective: declared.or(api),
        retry_actual: None,
    })
}
fn parse_asset_name(value: &Value, index: usize) -> Result<&str> {
    let name = value
        .as_object()
        .with_context(|| format!("GitHub release asset {index} must be an object"))?
        .get("name")
        .with_context(|| format!("GitHub release asset {index} is missing name"))?
        .as_str()
        .with_context(|| format!("GitHub release asset {index} name must be a string"))?;
    validate_asset_name(name)?;
    Ok(name)
}
pub(super) fn candidate_for_retry(
    operation: &BinaryPackageOperation,
    resolved: &Resolved,
) -> Result<Candidate> {
    Ok(Candidate {
        tag: resolved.tag.clone(),
        asset_name: resolved.asset_name.clone(),
        url: HttpsUrl::parse(&resolved.url)?,
        effective: resolved
            .effective_sha256
            .as_deref()
            .map(BinarySha256::parse)
            .transpose()?
            .or(match operation.source {
                BinarySourceOperation::ChecksummedUrl { sha256, .. } => Some(sha256),
                _ => None,
            }),
        retry_actual: Some(BinarySha256::parse(&resolved.actual_sha256)?),
    })
}
pub(super) fn same_remote_identity(candidate: &Candidate, resolved: &Resolved) -> bool {
    candidate.tag == resolved.tag
        && candidate.asset_name == resolved.asset_name
        && candidate.url.as_str() == resolved.url
        && candidate.effective.map(BinarySha256::as_hex) == resolved.effective_sha256
}

pub(super) fn download_candidate(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    candidate: Candidate,
) -> Result<Downloaded> {
    let suffix = match operation.format {
        BinaryPackageFormat::Deb => ".deb",
        BinaryPackageFormat::AppImage => ".AppImage",
    };
    let temporary = TempPath::new_with_suffix(host, &operation.name, suffix)?;
    host.require(
        "download binary package",
        "curl",
        [
            "--proto".as_ref(),
            "=https".as_ref(),
            "--location".as_ref(),
            "--fail".as_ref(),
            "--silent".as_ref(),
            "--show-error".as_ref(),
            "--retry".as_ref(),
            "3".as_ref(),
            "--retry-all-errors".as_ref(),
            "--output".as_ref(),
            temporary.path().as_os_str(),
            "--".as_ref(),
            candidate.url.as_str().as_ref(),
        ],
    )?;
    let metadata = fs::symlink_metadata(temporary.path())?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        bail!("binary package downloaded an empty or non-regular artifact");
    }
    let actual = BinarySha256(sha256_file(temporary.path())?);
    if candidate
        .effective
        .is_some_and(|expected| expected != actual)
        || candidate
            .retry_actual
            .is_some_and(|expected| expected != actual)
    {
        bail!("binary package SHA-256 checksum mismatch");
    }
    Ok(Downloaded {
        temporary,
        resolved: Resolved {
            tag: candidate.tag,
            asset_name: candidate.asset_name,
            url: candidate.url.as_str().into(),
            actual_sha256: actual.as_hex(),
            effective_sha256: candidate.effective.map(BinarySha256::as_hex),
        },
    })
}

pub(super) fn fixed_asset_name(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .rev()
                .find(|segment| !segment.is_empty() && valid_asset_name(segment))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "artifact".into())
}
