use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::path::Path;

use crate::operations::host::{Host, privileged_file::write_atomic, stdout_line};

const DOCKER_DAEMON_CONFIG: &str = "/etc/docker/daemon.json";

pub(crate) fn set_local_logging_driver(host: &Host, max_size: Option<&str>) -> Result<()> {
    host.require_cli("Docker", "docker")?;
    let mut daemon_config = read_daemon_config(host)?;
    daemon_config.insert("log-driver".into(), Value::String("local".into()));
    if let Some(max_size) = max_size {
        let log_options = daemon_config.entry("log-opts").or_insert_with(|| Value::Object(Map::new()));
        let log_options = log_options.as_object_mut().context("Docker daemon config log-opts must be a JSON object")?;
        log_options.insert("max-size".into(), Value::String(max_size.to_owned()));
    }
    let mut bytes = serde_json::to_vec_pretty(&daemon_config).context("serialize Docker daemon configuration")?;
    bytes.push(b'\n');
    write_atomic(host, Path::new(DOCKER_DAEMON_CONFIG), &bytes, "Docker daemon config write")?;
    Ok(())
}

fn read_daemon_config(host: &Host) -> Result<Map<String, Value>> {
    let stat_output = host.output("sudo", ["stat", "--format=%f", DOCKER_DAEMON_CONFIG])?;
    if !stat_output.status.success() {
        host.run("Docker daemon config absence check", "sudo", ["test", "!", "-e", DOCKER_DAEMON_CONFIG])?;
        host.run("Docker daemon config symlink absence check", "sudo", ["test", "!", "-L", DOCKER_DAEMON_CONFIG])?;
        return Ok(Map::new());
    }
    let mode = stdout_line(&stat_output.stdout, "sudo stat")?;
    let mode = u32::from_str_radix(mode, 16).context("sudo stat returned malformed mode output")?;
    if mode & 0o170000 != 0o100000 {
        bail!("Docker daemon config destination is not a regular file");
    }
    let output = host.run("Docker daemon config read", "sudo", ["cat", DOCKER_DAEMON_CONFIG])?;
    let text = std::str::from_utf8(&output.stdout).context("Docker daemon config is not valid UTF-8")?;
    serde_json::from_str(text).context("Docker daemon config must be a JSON object")
}
