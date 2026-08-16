use super::{Host, TempPath};
use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, fs, io::Write, path::Path};

pub(crate) fn write_atomic(host: &Host, destination: &Path, contents: &[u8], operation: &str) -> Result<()> {
    write_atomic_with_mode(host, destination, contents, operation, "0644")
}

/// Atomically publish to trusted system path; doesn't validate destination ancestors.
pub(crate) fn write_atomic_with_mode(
    host: &Host,
    destination: &Path,
    contents: &[u8],
    operation: &str,
    mode: &str,
) -> Result<()> {
    if !matches!(mode, "0600" | "0644") {
        bail!("unsupported privileged file mode");
    }
    let local = TempPath::new(host, "privileged-write")?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(local.path())
        .context("open local atomic-write staging file")?;
    file.write_all(contents).context("write local atomic-write staging file")?;
    file.sync_all().context("sync local atomic-write staging file")?;
    drop(file);
    let parent = destination.parent().context("atomic-write destination has no parent")?;
    let file_name = destination.file_name().context("atomic-write destination has no filename")?.to_string_lossy();
    let nonce = local.path().file_name().context("atomic-write staging file has no filename")?.to_string_lossy();
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
    // stage beside target for atomic rename, then sync file & parent
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
        host.require(operation, "sudo", [OsStr::new("sync"), OsStr::new("--"), staged_arg])?;
        host.require(operation, "sudo", [OsStr::new("test"), OsStr::new("!"), OsStr::new("-d"), destination_arg])?;
        host.require(
            operation,
            "sudo",
            [OsStr::new("mv"), OsStr::new("-fT"), OsStr::new("--"), staged_arg, destination_arg],
        )?;
        sync_parent_directory(host, destination, operation)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = host.run("sudo", [OsStr::new("rm"), OsStr::new("-f"), OsStr::new("--"), staged_arg]);
    }
    result
}

pub(crate) fn sync_parent_directory(host: &Host, destination: &Path, operation: &str) -> Result<()> {
    let parent = destination.parent().context("atomic-write destination has no parent")?;
    host.require(operation, "sudo", [OsStr::new("sync"), OsStr::new("--"), parent.as_os_str()])?;
    Ok(())
}
