use anyhow::{Result, bail};

use super::*;

pub(crate) fn ensure_in_sudo_group() -> Result<()> {
    let username = get_effective_username()?;
    run("administrative group membership", "sudo", ["usermod", "-aG", "sudo", "--", &username])?;
    Ok(())
}

pub(crate) fn ensure_in_group(label: &str, program: &str, group: &str) -> Result<()> {
    let username = get_effective_username()?;
    let groups = run(&format!("{label} group membership query"), "id", ["-nG", "--", &username])?;
    if stdout_line(&groups.stdout, "id -nG")?.split_ascii_whitespace().any(|current| current == group) {
        return Ok(());
    }
    // verify the product exists before creating its integration group
    require_cli(label, program)?;
    run(&format!("{label} group creation"), "sudo", ["groupadd", "-f", group])?;
    run(&format!("{label} group membership"), "sudo", ["usermod", "-aG", group, "--", username.as_str()])?;
    Ok(())
}

fn get_effective_username() -> Result<String> {
    // use effective UID from NSS instead of user-controlled env vars
    let uid = rustix::process::geteuid().as_raw();
    let output = run("effective user query", "getent", ["passwd", &uid.to_string()])?;
    let record = stdout_line(&output.stdout, "getent passwd")?;
    let mut fields = record.split(':');
    let username = fields.next().unwrap_or_default();
    if username.is_empty()
        || fields.next().is_none()
        || fields.next().and_then(|field| field.parse::<u32>().ok()) != Some(uid)
        || fields.count() != 4
    {
        bail!("getent passwd returned a malformed effective-user record");
    }
    Ok(username.to_owned())
}
