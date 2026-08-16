use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::path::Path;

use super::{Host, host::one_record, privileged_file::write_atomic};

const DOCKER_DAEMON_CONFIG: &str = "/etc/docker/daemon.json";

pub(crate) fn set_local_logging_driver(host: &Host, max_size: Option<&str>) -> Result<()> {
    preflight(host)?;
    let mut requested = read_daemon_config(host)?;
    let object = requested.as_object_mut().context("Docker daemon config must be a JSON object")?;
    object.insert("log-driver".into(), Value::String("local".into()));
    if let Some(max_size) = max_size {
        let log_options = object
            .entry("log-opts")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .context("Docker daemon config log-opts must be a JSON object")?;
        log_options.insert("max-size".into(), Value::String(max_size.to_owned()));
    }
    let mut bytes = serde_json::to_vec_pretty(&requested).context("serialize Docker daemon configuration")?;
    bytes.push(b'\n');
    write_atomic(host, Path::new(DOCKER_DAEMON_CONFIG), &bytes, "Docker daemon config write")?;
    Ok(())
}

fn preflight(host: &Host) -> Result<()> {
    host.require("Docker existing-product preflight", "docker", ["--version"])
        .with_context(|| "Docker integration requires an existing usable docker CLI")?;
    Ok(())
}

fn read_daemon_config(host: &Host) -> Result<Value> {
    let kind = host.run("sudo", ["stat", "--format=%f", "--", DOCKER_DAEMON_CONFIG])?;
    if !kind.status.success() {
        host.require("Docker daemon config absence check", "sudo", ["test", "!", "-e", DOCKER_DAEMON_CONFIG])?;
        host.require("Docker daemon config symlink absence check", "sudo", ["test", "!", "-L", DOCKER_DAEMON_CONFIG])?;
        return Ok(Value::Object(Map::new()));
    }
    let mode = one_record(&kind.stdout, "sudo stat")?;
    let mode = u32::from_str_radix(mode, 16).context("sudo stat returned malformed mode output")?;
    if mode & 0o170000 != 0o100000 {
        bail!("Docker daemon config destination is not a regular file");
    }
    let output = host.require("Docker daemon config inspection", "sudo", ["cat", "--", DOCKER_DAEMON_CONFIG])?;
    let text = std::str::from_utf8(&output.stdout).context("Docker daemon config is not valid UTF-8")?;
    let value: Value = serde_json::from_str(text).context("Docker daemon config is invalid JSON")?;
    if !value.is_object() {
        bail!("Docker daemon config must be a JSON object");
    }
    Ok(value)
}
