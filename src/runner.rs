use anyhow::{bail, Context, Result};
use std::{ffi::OsString, process::Command};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Option<String>,
}
impl Step {
    pub fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|s| (*s).into()).collect(),
            stdin: None,
        }
    }
    pub fn owned(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            stdin: None,
        }
    }
    pub fn bash(script: impl Into<String>, args: Vec<String>) -> Self {
        let mut argv = vec![
            "-euo".into(),
            "pipefail".into(),
            "-c".into(),
            script.into(),
            "--".into(),
        ];
        argv.extend(args);
        Self::owned("bash", argv)
    }
    pub fn input(mut self, s: String) -> Self {
        self.stdin = Some(s);
        self
    }
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }
}
fn shell_quote(s: &str) -> String {
    if s.bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"-._/:=@".contains(&c))
    {
        s.into()
    } else {
        format!("'{0}'", s.replace('\'', "'\\''"))
    }
}
pub trait Runner {
    fn run(&mut self, step: &Step) -> Result<()>;
}
pub struct ProcessRunner {
    pub dry_run: bool,
}
impl Runner for ProcessRunner {
    fn run(&mut self, step: &Step) -> Result<()> {
        println!("+ {}", step.display());
        if self.dry_run {
            return Ok(());
        }
        let mut child = Command::new(&step.program)
            .args(step.args.iter().map(OsString::from))
            .stdin(if step.stdin.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::inherit()
            })
            .spawn()
            .with_context(|| format!("start {}", step.program))?;
        if let Some(input) = &step.stdin {
            use std::io::Write;
            let mut stdin = child
                .stdin
                .take()
                .context("child stdin unavailable after requesting pipe")?;
            stdin.write_all(input.as_bytes())?
        }
        let status = child.wait()?;
        if !status.success() {
            bail!("command failed ({status}): {}", step.display())
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct RecordingRunner {
    pub steps: Vec<Step>,
}
impl Runner for RecordingRunner {
    fn run(&mut self, step: &Step) -> Result<()> {
        self.steps.push(step.clone());
        Ok(())
    }
}
pub fn execute(runner: &mut dyn Runner, steps: &[Step]) -> Result<()> {
    for step in steps {
        runner.run(step)?
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_runner_writes_stdin_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("stdin");
        let step = Step::owned(
            "sh",
            vec![
                "-c".into(),
                "cat > \"$1\"".into(),
                "--".into(),
                output.display().to_string(),
            ],
        )
        .input("first\nsecond\n".into());
        ProcessRunner { dry_run: false }.run(&step).unwrap();
        assert_eq!(std::fs::read(output).unwrap(), b"first\nsecond\n");
    }

    #[test]
    fn process_runner_reports_failure_and_execute_stops() {
        let mut runner = ProcessRunner { dry_run: false };
        let steps = [
            Step::new("sh", &["-c", "exit 23"]),
            Step::new("sh", &["-c", "exit 0"]),
        ];
        let error = execute(&mut runner, &steps).unwrap_err().to_string();
        assert!(error.contains("command failed"));
        assert!(error.contains("23"));
    }

    #[test]
    fn display_quotes_hostile_arguments_without_changing_command_source() {
        let step = Step::bash(
            "printf '%s' \"$1\"",
            vec!["x'; touch /tmp/pwn; echo '".into()],
        );
        assert_eq!(step.program, "bash");
        assert!(step.display().contains("'\\''"));
        assert!(!step.args[3].contains("touch"));
    }
}
