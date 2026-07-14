use super::{Host, TempPath};
use crate::config::v1::HttpsUrl;
use anyhow::{bail, Context, Result};
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

const SOURCES_DIRECTORY: &str = "/etc/apt/sources.list.d";
const KEYRINGS_DIRECTORY: &str = "/etc/apt/keyrings";

pub fn source(host: &Host<'_>, destination: &str, contents: &str) -> Result<()> {
    let destination = validate_destination(destination, SOURCES_DIRECTORY, ".list")?;
    validate_source_contents(contents)?;
    publish_bytes(
        host,
        &destination,
        contents.as_bytes(),
        "APT source publication",
    )
}

pub fn key(host: &Host<'_>, url: &str, destination: &str) -> Result<()> {
    validate_https_url(url)?;
    let destination = validate_destination(destination, KEYRINGS_DIRECTORY, ".gpg")?;
    let downloaded = TempPath::new(host, "repository-key-download")?;
    let normalized = TempPath::new(host, "repository-key-normalized")?;

    host.require(
        "repository key download",
        "curl",
        [
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--retry",
            "3",
            "--retry-all-errors",
            "--output",
            &downloaded.path().to_string_lossy(),
            url,
        ],
    )?;
    host.require(
        "repository key conversion",
        "gpg",
        [
            "--no-options",
            "--batch",
            "--yes",
            "--dearmor",
            "--output",
            &normalized.path().to_string_lossy(),
            &downloaded.path().to_string_lossy(),
        ],
    )?;
    let bytes = fs::read(normalized.path()).context("read normalized repository key")?;
    if bytes.is_empty() {
        bail!("repository key conversion produced empty output");
    }
    let inspection = host.require(
        "repository key validation",
        "gpg",
        [
            "--no-options",
            "--batch",
            "--no-default-keyring",
            "--keyring",
            &normalized.path().to_string_lossy(),
            "--list-keys",
            "--with-colons",
        ],
    )?;
    if !inspection.stdout.split(|byte| *byte == b'\n').any(|line| {
        line.strip_prefix(b"pub:")
            .is_some_and(|fields| !fields.is_empty())
    }) {
        bail!("repository key validation found no public key");
    }
    publish_bytes(host, &destination, &bytes, "repository key publication")
}

pub(crate) fn publish_bytes(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
) -> Result<()> {
    let local = TempPath::new(host, "privileged-publication")?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(local.path())
        .context("open local publication staging file")?;
    file.write_all(contents)
        .context("write local publication staging file")?;
    file.sync_all()
        .context("sync local publication staging file")?;
    drop(file);

    let parent = destination
        .parent()
        .context("publication destination has no parent")?;
    let file_name = destination
        .file_name()
        .context("publication destination has no filename")?
        .to_string_lossy();
    let nonce = local
        .path()
        .file_name()
        .context("publication staging file has no filename")?
        .to_string_lossy();
    let staged = parent.join(format!(".{file_name}.{nonce}.tmp"));
    let parent_arg = parent.as_os_str();
    let local_arg = local.path().as_os_str();
    let staged_arg = staged.as_os_str();
    let destination_arg = destination.as_os_str();

    host.require(
        operation,
        "sudo",
        [
            OsStr::new("install"),
            OsStr::new("-d"),
            OsStr::new("-o"),
            OsStr::new("root"),
            OsStr::new("-g"),
            OsStr::new("root"),
            OsStr::new("-m"),
            OsStr::new("0755"),
            OsStr::new("--"),
            parent_arg,
        ],
    )?;
    let result = (|| {
        host.require(
            operation,
            "sudo",
            [
                OsStr::new("install"),
                OsStr::new("-o"),
                OsStr::new("root"),
                OsStr::new("-g"),
                OsStr::new("root"),
                OsStr::new("-m"),
                OsStr::new("0644"),
                OsStr::new("--"),
                local_arg,
                staged_arg,
            ],
        )?;
        host.require(
            operation,
            "sudo",
            [OsStr::new("sync"), OsStr::new("--"), staged_arg],
        )?;
        host.require(
            operation,
            "sudo",
            [
                OsStr::new("test"),
                OsStr::new("!"),
                OsStr::new("-d"),
                destination_arg,
            ],
        )?;
        host.require(
            operation,
            "sudo",
            [
                OsStr::new("mv"),
                OsStr::new("-fT"),
                OsStr::new("--"),
                staged_arg,
                destination_arg,
            ],
        )?;
        sync_parent(host, destination, operation)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = host.run(
            "sudo",
            [
                OsStr::new("rm"),
                OsStr::new("-f"),
                OsStr::new("--"),
                staged_arg,
            ],
        );
    }
    result
}

pub(crate) fn sync_parent(host: &Host<'_>, destination: &Path, operation: &str) -> Result<()> {
    let parent = destination
        .parent()
        .context("publication destination has no parent")?;
    host.require(
        operation,
        "sudo",
        [OsStr::new("sync"), OsStr::new("--"), parent.as_os_str()],
    )?;
    Ok(())
}

fn validate_destination(destination: &str, directory: &str, suffix: &str) -> Result<PathBuf> {
    let path = Path::new(destination);
    if destination.as_bytes().contains(&0)
        || !path.is_absolute()
        || path.parent() != Some(Path::new(directory))
        || path.file_name().is_none_or(|name| {
            name.as_bytes().contains(&0)
                || !name.as_bytes().ends_with(suffix.as_bytes())
                || name.as_bytes().len() == suffix.len()
        })
    {
        bail!("destination must be a direct {suffix} file under {directory}");
    }
    Ok(path.to_owned())
}

fn validate_source_contents(contents: &str) -> Result<()> {
    if contents.as_bytes().contains(&0)
        || !contents.ends_with('\n')
        || contents.lines().count() != 1
        || contents
            .lines()
            .next()
            .is_none_or(|line| line.trim().is_empty())
    {
        bail!("APT source contents must be exactly one non-empty generated line");
    }
    Ok(())
}

fn validate_https_url(value: &str) -> Result<()> {
    let validated = HttpsUrl::parse(value).context("repository key URL is invalid")?;
    if validated.as_str() != value {
        bail!("repository key URL must be canonical HTTPS");
    }
    Ok(())
}
