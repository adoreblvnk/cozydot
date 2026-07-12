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
            child.stdin.take().unwrap().write_all(input.as_bytes())?
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
