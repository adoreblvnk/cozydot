use anyhow::{bail, Context, Result};
use std::{ffi::OsStr, fs, io::Write, path::Path};

use super::{Host, TempPath};

pub(crate) fn publish_bytes(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
) -> Result<()> {
    publish_bytes_with_mode(host, destination, contents, operation, "0644")
}

pub(crate) fn publish_bytes_with_mode(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
    mode: &str,
) -> Result<()> {
    publish_bytes_with_mode_and_policy(host, destination, contents, operation, mode, false)
}

pub(super) fn publish_bytes_with_policy(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
    no_replace: bool,
) -> Result<()> {
    publish_bytes_with_mode_and_policy(host, destination, contents, operation, "0644", no_replace)
}

fn publish_bytes_with_mode_and_policy(
    host: &Host<'_>,
    destination: &Path,
    contents: &[u8],
    operation: &str,
    mode: &str,
    no_replace: bool,
) -> Result<()> {
    if !matches!(mode, "0600" | "0644") {
        bail!("unsupported privileged publication mode");
    }
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
                OsStr::new(mode),
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
        if no_replace {
            // `link(2)` is an atomic no-replace publication here: both names are in the
            // destination directory, and an existing destination makes `ln` fail rather
            // than report a skipped move as success. The staging name is removed only
            // after the destination link exists.
            host.require(
                operation,
                "sudo",
                [
                    OsStr::new("ln"),
                    OsStr::new("--"),
                    staged_arg,
                    destination_arg,
                ],
            )?;
            host.require(
                operation,
                "sudo",
                [
                    OsStr::new("rm"),
                    OsStr::new("-f"),
                    OsStr::new("--"),
                    staged_arg,
                ],
            )?;
        } else {
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
        }
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
