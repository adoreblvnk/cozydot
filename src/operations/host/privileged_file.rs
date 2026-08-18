use std::{ffi::OsStr, fs, io::Write, path::Path};

use anyhow::{Context, Result};

use super::{Host, TempPath};

pub(crate) fn write_atomic(host: &Host, destination: &Path, contents: &[u8], label: &str) -> Result<()> {
    let local = TempPath::new("privileged-write")?;
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
    host.run(
        label,
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
        host.run(
            label,
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
        host.run(label, "sudo", [OsStr::new("sync"), OsStr::new("--"), staged_arg])?;
        host.run(label, "sudo", [OsStr::new("test"), OsStr::new("!"), OsStr::new("-d"), destination_arg])?;
        host.run(label, "sudo", [OsStr::new("mv"), OsStr::new("-fT"), OsStr::new("--"), staged_arg, destination_arg])?;
        host.run(label, "sudo", [OsStr::new("sync"), OsStr::new("--"), parent_arg])?;
        Ok(())
    })();
    if result.is_err() {
        let _ = host.output("sudo", [OsStr::new("rm"), OsStr::new("-f"), OsStr::new("--"), staged_arg]);
    }
    result
}
