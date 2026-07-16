use super::*;

pub(super) fn resolve_rust_release(
    host: &Host<'_>,
    selector: &RustToolchainSelector,
    target: &str,
) -> Result<ToolResolution> {
    let requested = rust_selector_name(selector);
    let url = match selector {
        RustToolchainSelector::DatedNightly(value) => format!(
            "https://static.rust-lang.org/dist/{}/channel-rust-nightly.toml",
            value.trim_start_matches("nightly-")
        ),
        _ => format!("https://static.rust-lang.org/dist/channel-rust-{requested}.toml"),
    };
    let output = host.require(
        "Rust release availability",
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
            "--",
            &url,
        ],
    )?;
    let (date, release) = parse_rust_manifest(
        std::str::from_utf8(&output.stdout).context("Rust release manifest is not UTF-8")?,
        target,
    )?;
    let resolved = match selector {
        RustToolchainSelector::Nightly => format!("nightly-{date}"),
        RustToolchainSelector::DatedNightly(value) => {
            if value.trim_start_matches("nightly-") != date {
                bail!("dated Rust manifest does not match the requested date");
            }
            value.clone()
        }
        RustToolchainSelector::Stable => {
            if !numeric_release(&release) {
                bail!("Rust stable manifest resolved a non-stable release");
            }
            release.clone()
        }
        RustToolchainSelector::Beta => {
            if !release.contains("-beta") {
                bail!("Rust beta manifest resolved a non-beta release");
            }
            release.clone()
        }
        RustToolchainSelector::Version(requested) => {
            if !numeric_release(&release) || !version_matches(&release, requested) {
                bail!("Rust version manifest does not match the requested release");
            }
            release.clone()
        }
    };
    let resolution = ToolResolution { resolved, release };
    let record = ToolRecord {
        version: TOOL_STATE_VERSION,
        status: ToolStatus::Pending,
        tool: ToolKind::Rust,
        requested: requested.into(),
        resolved: resolution.resolved.clone(),
        release: resolution.release.clone(),
        platform: target.into(),
    };
    validate_tool_record(&record)?;
    Ok(resolution)
}

pub(super) fn parse_rust_manifest(input: &str, target: &str) -> Result<(String, String)> {
    let target_section = format!("[pkg.rust.target.{target}]");
    let mut section = "";
    let mut manifest_version: Option<String> = None;
    let mut date: Option<String> = None;
    let mut version: Option<String> = None;
    let mut available = None;
    for line in input.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if section.is_empty() {
            if let Some(value) = quoted_assignment(line, "manifest-version")? {
                set_once(&mut manifest_version, value, "Rust manifest version")?;
            } else if let Some(value) = quoted_assignment(line, "date")? {
                set_once(&mut date, value, "Rust manifest date")?;
            }
        } else if section == "[pkg.rust]" {
            if let Some(value) = quoted_assignment(line, "version")? {
                let release = value
                    .split_whitespace()
                    .next()
                    .context("Rust manifest package version is empty")?;
                set_once(&mut version, release.to_owned(), "Rust package version")?;
            }
        } else if section == target_section {
            if line == "available = true" {
                set_once(&mut available, true, "Rust target availability")?;
            } else if line == "available = false" {
                set_once(&mut available, false, "Rust target availability")?;
            }
        }
    }
    if manifest_version.as_deref() != Some("2") || available != Some(true) {
        bail!("Rust release manifest is unsupported or unavailable for target {target}");
    }
    let date = date.context("Rust release manifest is missing date")?;
    validate_rust_selector(&RustToolchainSelector::DatedNightly(format!(
        "nightly-{date}"
    )))?;
    let version = version.context("Rust release manifest is missing Rust package version")?;
    if !valid_rust_release(&version) {
        bail!("Rust release manifest has an invalid Rust package version");
    }
    Ok((date, version))
}

fn quoted_assignment(line: &str, key: &str) -> Result<Option<String>> {
    let prefix = format!("{key} = \"");
    let Some(value) = line.strip_prefix(&prefix) else {
        return Ok(None);
    };
    let value = value
        .strip_suffix('"')
        .with_context(|| format!("Rust manifest {key} must be a simple quoted string"))?;
    if value.is_empty() || value.contains(['"', '\\']) || value.chars().any(char::is_control) {
        bail!("Rust manifest {key} must be a simple quoted string");
    }
    Ok(Some(value.into()))
}

fn set_once<T>(slot: &mut Option<T>, value: T, field: &str) -> Result<()> {
    if slot.replace(value).is_some() {
        bail!("{field} is duplicated");
    }
    Ok(())
}

pub(super) fn resolve_go_release(
    host: &Host<'_>,
    requested: &str,
    architecture: Architecture,
) -> Result<GoRelease> {
    let metadata = host.require(
        "Go release resolution",
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
            "--",
            "https://go.dev/dl/?mode=json&include=all",
        ],
    )?;
    let (version, filename, checksum) = json_helpers::latest_go(
        std::str::from_utf8(&metadata.stdout).context("Go release metadata is not UTF-8")?,
        requested,
        architecture.go_archive(),
    )?;
    Ok(GoRelease {
        resolution: ToolResolution {
            resolved: version.clone(),
            release: version,
        },
        filename,
        checksum,
    })
}

pub(super) fn resolve_python_version(
    host: &Host<'_>,
    uv: &str,
    requested: &str,
    architecture: Architecture,
) -> Result<ToolResolution> {
    let output = host.require(
        "Python release availability",
        uv,
        [
            "python",
            "list",
            requested,
            "--all-versions",
            "--only-downloads",
            "--output-format",
            "json",
            "--no-config",
            "--no-progress",
        ],
    )?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("uv returned malformed Python release JSON")?;
    let entries = value
        .as_array()
        .context("uv Python release state must be an array")?;
    let expected_arch = match architecture {
        Architecture::Amd64 => "x86_64",
        Architecture::Arm64 => "aarch64",
        Architecture::Arm32 => "armv7",
        Architecture::Riscv64 => "riscv64",
    };
    let mut matches = Vec::new();
    for (index, value) in entries.iter().enumerate() {
        let entry = value
            .as_object()
            .with_context(|| format!("uv Python release {index} must be an object"))?;
        let string = |field: &str| -> Result<&str> {
            entry
                .get(field)
                .with_context(|| format!("uv Python release {index} is missing {field}"))?
                .as_str()
                .with_context(|| format!("uv Python release {index} {field} must be a string"))
        };
        let version = string("version")?;
        validate_numeric_version(version, 3, 3, "uv Python release")?;
        let url = HttpsUrl::parse(string("url")?)?;
        if url.as_str() != string("url")? {
            bail!("uv Python release URL is not canonical");
        }
        if string("implementation")? == "cpython"
            && string("os")? == "linux"
            && string("variant")? == "default"
            && string("arch")? == expected_arch
            && string("libc")? == "gnu"
            && version_matches(version, requested)
        {
            matches.push(version.to_owned());
        }
    }
    matches.sort_by_key(|version| numeric_version_key(version));
    matches.dedup();
    let resolved = matches
        .pop()
        .with_context(|| format!("uv has no managed Python release matching {requested:?}"))?;
    Ok(ToolResolution {
        release: resolved.clone(),
        resolved,
    })
}

fn numeric_version_key(value: &str) -> (u64, u64, u64) {
    let mut parts = value
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or_default());
    (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    )
}
