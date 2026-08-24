use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use std::path::Path;

use crate::operations::host::{self, privileged_file::write_atomic, stdout_line};

const DAEMON_CONFIG: &str = "/etc/docker/daemon.json";

pub(crate) fn set_local_logging_driver(max_size: Option<&str>) -> Result<()> {
    host::require_cli("Docker", "docker")?;
    let mut daemon_config = read_config()?;
    daemon_config.insert("log-driver".into(), Value::String("local".into()));
    if let Some(max_size) = max_size {
        let log_options = daemon_config.entry("log-opts").or_insert_with(|| Value::Object(Map::new()));
        let log_options = log_options.as_object_mut().context("Docker daemon config log-opts must be a JSON object")?;
        log_options.insert("max-size".into(), Value::String(max_size.to_owned()));
    }
    let mut bytes = serde_json::to_vec_pretty(&daemon_config).context("serialize Docker daemon configuration")?;
    bytes.push(b'\n');
    write_atomic(Path::new(DAEMON_CONFIG), &bytes, "Docker daemon config write")?;
    Ok(())
}

fn read_config() -> Result<Map<String, Value>> {
    let stat_output = host::output("sudo", ["stat", "--format=%f", DAEMON_CONFIG])?;
    if !stat_output.status.success() {
        host::run("Docker daemon config absence check", "sudo", ["test", "!", "-e", DAEMON_CONFIG])?;
        host::run("Docker daemon config symlink absence check", "sudo", ["test", "!", "-L", DAEMON_CONFIG])?;
        return Ok(Map::new());
    }
    let mode_hex = stdout_line(&stat_output.stdout, "sudo stat")?;
    let mode = u32::from_str_radix(mode_hex, 16).context("sudo stat returned malformed mode output")?;
    ensure!(mode & 0o170000 == 0o100000, "Docker daemon config destination is not a regular file");
    let output = host::run("Docker daemon config read", "sudo", ["cat", DAEMON_CONFIG])?;
    serde_json::from_slice(&output.stdout).context("Docker daemon config must be a JSON object")
}
