use std::{process::Command, thread, time::Duration};

use anyhow::Result;

use super::run;

pub(crate) fn validate_access() -> Result<()> {
    run("sudo access validation", "sudo", ["-v"])?;
    thread::spawn(|| {
        loop {
            thread::sleep(Duration::from_secs(60));
            let _ = Command::new("sudo").args(["-n", "-v"]).output();
        }
    });
    Ok(())
}
