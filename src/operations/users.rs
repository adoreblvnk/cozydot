use anyhow::{Result, bail};

use super::{Host, host::one_record};

pub(crate) fn sudo_group(host: &Host) -> Result<()> {
    let (username, _) = effective_user(host)?;
    host.require("administrative group membership", "sudo", ["usermod", "-aG", "sudo", "--", &username])?;
    Ok(())
}

pub(crate) fn ensure_product_group(host: &Host, label: &str, program: &str, group: &str) -> Result<()> {
    let (username, _) = effective_user(host)?;
    let groups = host.require(&format!("{label} group membership query"), "id", ["-nG", "--", &username])?;
    if one_record(&groups.stdout, "id -nG")?.split_ascii_whitespace().any(|current| current == group) {
        return Ok(());
    }
    preflight(host, label, program)?;
    host.require(&format!("{label} group creation"), "sudo", ["groupadd", "-f", group])?;
    host.require(&format!("{label} group membership"), "sudo", ["usermod", "-aG", group, "--", username.as_str()])?;
    Ok(())
}

fn effective_user(host: &Host) -> Result<(String, u32)> {
    let uid = rustix::process::geteuid().as_raw();
    let output = host.require("effective user query", "getent", ["passwd", &uid.to_string()])?;
    let record = one_record(&output.stdout, "getent passwd")?;
    let fields = record.split(':').collect::<Vec<_>>();
    if fields.len() != 7 || fields[0].is_empty() || fields[2].parse::<u32>().ok() != Some(uid) {
        bail!("getent passwd returned a malformed effective-user record");
    }
    Ok((fields[0].to_owned(), uid))
}

fn preflight(host: &Host, label: &str, program: &str) -> Result<()> {
    use anyhow::Context;

    host.require(&format!("{label} existing-product preflight"), program, ["--version"])
        .with_context(|| format!("{label} integration requires an existing usable {program} CLI"))?;
    Ok(())
}
