//! In-place terminal spinner for step status reporting.

use anyhow::{Context, Result};
use std::{
    io::{IsTerminal, Write, stderr},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use crate::style::STATUS;

// braille frames from cli-spinners
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct PanicGuard<'a>(&'a AtomicBool);

impl Drop for PanicGuard<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
        // carriage return & erase to line end
        eprint!("\r\x1b[K");
        let _ = stderr().flush();
    }
}

// modeled on GitHub CLI (gh) & npm prefix spinner style
pub(crate) fn run<T>(status: &'static str, subject: &str, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    // check TTY & CI environment before animating
    if !stderr().is_terminal() || std::env::var_os("CI").is_some() {
        let result = operation().with_context(|| format!("{} {subject}", status.to_lowercase()));
        if result.is_ok() {
            anstream::eprintln!("{STATUS}✓{STATUS:#} {status} {subject}");
        }
        return result;
    }

    let stop = AtomicBool::new(false);
    thread::scope(|s| {
        let guard = PanicGuard(&stop);
        let handle = s.spawn(|| {
            for frame in SPINNER_FRAMES.iter().cycle() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                anstream::eprint!("\r{STATUS}{frame}{STATUS:#} {status} {subject}");
                let _ = stderr().flush();
                thread::sleep(Duration::from_millis(80));
            }
        });

        let result = operation().with_context(|| format!("{} {subject}", status.to_lowercase()));
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();
        std::mem::forget(guard);

        if result.is_ok() {
            // overwrite animated frame with completed checkmark
            anstream::eprintln!("\r{STATUS}✓{STATUS:#} {status} {subject}\x1b[K");
        } else {
            // erase animated line on failure
            eprint!("\r\x1b[K");
            let _ = stderr().flush();
        }
        result
    })
}
