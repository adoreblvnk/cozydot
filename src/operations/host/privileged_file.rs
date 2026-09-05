use std::{fs, io::Write, path::Path};

use anyhow::{Context, Result};

use super::*;

pub(crate) fn write_atomic(destination: &Path, contents: &[u8], label: &str) -> Result<()> {
    let local = temp_path("privileged-write", "")?;
    let context = "open local atomic-write staging file";
    let mut file = fs::OpenOptions::new().write(true).truncate(true).open(&local).context(context)?;
    file.write_all(contents).context("write local atomic-write staging file")?;
    file.sync_all().context("sync local atomic-write staging file")?;
    drop(file);
    let parent = destination.parent().context("atomic-write destination has no parent")?;
    let file_name = destination.file_name().context("atomic-write destination has no filename")?.to_string_lossy();
    let nonce = local.file_name().unwrap_or_default().to_string_lossy();
    let staged = parent.join(format!(".{file_name}.{nonce}.tmp"));
    let parent_path = parent.to_str().context("atomic-write parent is not UTF-8")?;
    let local_path = local.to_str().context("atomic-write staging file path is not UTF-8")?;
    let staged_path = staged.to_str().context("atomic-write staged path is not UTF-8")?;
    let destination_path = destination.to_str().context("atomic-write destination is not UTF-8")?;

    let install_d = ["install", "-d", "-o", "root", "-g", "root", "-m", "0755", "--", parent_path];
    run(label, "sudo", install_d)?;
    // stage beside target for atomic rename, then sync file & parent
    let result = (|| {
        let install_file = ["install", "-o", "root", "-g", "root", "-m", "0644", "--", local_path, staged_path];
        run(label, "sudo", install_file)?;
        run(label, "sudo", ["sync", "--", staged_path])?;
        run(label, "sudo", ["test", "!", "-d", destination_path])?;
        run(label, "sudo", ["mv", "-fT", "--", staged_path, destination_path])?;
        run(label, "sudo", ["sync", "--", parent_path])?;
        Ok(())
    })();
    if result.is_err() {
        // preserve the original failure; stale staging cleanup is best effort
        let _ = output("sudo", ["rm", "-f", "--", staged_path]);
    }
    result
}
