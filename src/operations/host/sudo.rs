use std::{process::Command, thread, time::Duration};

use anyhow::Result;

use crate::style::STATUS;

use super::run;

pub(crate) fn validate_access(subject: &str) -> Result<()> {
    run("sudo access validation", "sudo", ["-v"])?;
    anstream::eprintln!("{STATUS}✓{STATUS:#} Validating {subject}");
    thread::spawn(|| {
        loop {
            thread::sleep(Duration::from_secs(60));
            let _ = Command::new("sudo").args(["-n", "-v"]).output();
        }
    });
    Ok(())
}
