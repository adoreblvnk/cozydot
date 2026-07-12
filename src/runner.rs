use crate::operations::{self, Operation};
use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandStep {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    CommandExists(String),
    CommandMissing(String),
    PackageInstalled(String),
    PackageMissing(String),
    ServiceActive(String),
    UserServiceInactive(String),
    GroupMissingUser { group: String, user: String },
    FileExists(PathBuf),
    FileMissing(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Command(CommandStep),
    Workflow(Operation),
    Conditional {
        condition: Condition,
        action: Box<Step>,
    },
    /// A fixed shell bridge for syntax that has no direct process equivalent.
    Shell(CommandStep),
}

impl Step {
    pub fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self::owned(program, args.iter().map(|value| (*value).into()).collect())
    }

    pub fn owned(program: impl Into<String>, args: Vec<String>) -> Self {
        Self::Command(CommandStep {
            program: program.into(),
            args,
            stdin: None,
        })
    }

    pub fn shell(script: impl Into<String>, args: Vec<String>) -> Self {
        let mut argv = vec![
            "-euo".into(),
            "pipefail".into(),
            "-c".into(),
            script.into(),
            "--".into(),
        ];
        argv.extend(args);
        Self::Shell(CommandStep {
            program: "bash".into(),
            args: argv,
            stdin: None,
        })
    }

    pub fn input(mut self, input: String) -> Self {
        match &mut self {
            Self::Command(command) | Self::Shell(command) => command.stdin = Some(input),
            _ => panic!("stdin is only valid for command steps"),
        }
        self
    }

    pub fn workflow(operation: Operation) -> Self {
        Self::Workflow(operation)
    }

    pub fn conditional(condition: Condition, action: Step) -> Self {
        Self::Conditional {
            condition,
            action: Box::new(action),
        }
    }

    pub fn command(&self) -> Option<&CommandStep> {
        match self {
            Self::Command(command) | Self::Shell(command) => Some(command),
            _ => None,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Self::Command(command) | Self::Shell(command) => display_command(command),
            Self::Workflow(operation) => format!("workflow {}", operation.display_args().join(" ")),
            Self::Conditional { condition, action } => {
                format!("if {}; then {}; fi", condition.display(), action.display())
            }
        }
    }
}

impl Condition {
    fn display(&self) -> String {
        match self {
            Self::CommandExists(value) => format!("command-exists {}", shell_quote(value)),
            Self::CommandMissing(value) => format!("command-missing {}", shell_quote(value)),
            Self::PackageInstalled(value) => format!("package-installed {}", shell_quote(value)),
            Self::PackageMissing(value) => format!("package-missing {}", shell_quote(value)),
            Self::ServiceActive(value) => format!("service-active {}", shell_quote(value)),
            Self::UserServiceInactive(value) => {
                format!("user-service-inactive {}", shell_quote(value))
            }
            Self::GroupMissingUser { group, user } => format!(
                "group-missing-user {} {}",
                shell_quote(group),
                shell_quote(user)
            ),
            Self::FileExists(value) => {
                format!("file-exists {}", shell_quote(&value.to_string_lossy()))
            }
            Self::FileMissing(value) => {
                format!("file-missing {}", shell_quote(&value.to_string_lossy()))
            }
        }
    }
}

fn display_command(command: &CommandStep) -> String {
    std::iter::once(command.program.as_str())
        .chain(command.args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-._/:=@".contains(&byte))
    {
        value.into()
    } else {
        format!("'{0}'", value.replace('\'', "'\\''"))
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
        match step {
            Step::Command(command) | Step::Shell(command) => run_command(command),
            Step::Workflow(operation) => operations::execute(operation, &[]),
            Step::Conditional { condition, action } => {
                if inspect(condition)? {
                    self.run(action)
                } else {
                    Ok(())
                }
            }
        }
    }
}

fn run_command(step: &CommandStep) -> Result<()> {
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
        child
            .stdin
            .take()
            .context("child stdin unavailable after requesting pipe")?
            .write_all(input.as_bytes())?;
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("command failed ({status}): {}", display_command(step));
    }
    Ok(())
}

fn inspect(condition: &Condition) -> Result<bool> {
    let status = |program: &str, args: &[&str]| -> Result<bool> {
        Ok(Command::new(program).args(args).status()?.success())
    };
    match condition {
        Condition::CommandExists(name) => {
            Ok(std::env::var_os("PATH").is_some_and(|path| command_exists_in(name, &path)))
        }
        Condition::CommandMissing(name) => {
            inspect(&Condition::CommandExists(name.clone())).map(|v| !v)
        }
        Condition::PackageInstalled(name) => status("dpkg-query", &["-W", name]),
        Condition::PackageMissing(name) => status("dpkg-query", &["-W", name]).map(|v| !v),
        Condition::ServiceActive(name) => status("systemctl", &["-q", "is-active", name]),
        Condition::UserServiceInactive(name) => {
            status("systemctl", &["--user", "-q", "is-active", name]).map(|v| !v)
        }
        Condition::GroupMissingUser { group, user } => {
            let output = Command::new("getent").args(["group", group]).output()?;
            Ok(!output.status.success()
                || !String::from_utf8_lossy(&output.stdout)
                    .split(|character| [':', ','].contains(&character))
                    .any(|member| member == user))
        }
        Condition::FileExists(path) => Ok(path.exists()),
        Condition::FileMissing(path) => Ok(!path.exists()),
    }
}

pub fn command_exists_in(name: &str, path: &OsStr) -> bool {
    std::env::split_paths(path).any(|dir| executable_regular_file(&dir.join(name)))
}

fn executable_regular_file(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub fn execute(runner: &mut dyn Runner, steps: &[Step]) -> Result<()> {
    for step in steps {
        runner.run(step)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

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
    fn display_quotes_hostile_arguments_without_changing_shell_source() {
        let step = Step::shell(
            "printf '%s' \"$1\"",
            vec!["x'; touch /tmp/pwn; echo '".into()],
        );
        let command = step.command().unwrap();
        assert_eq!(command.program, "bash");
        assert!(step.display().contains("'\\''"));
        assert!(!command.args[3].contains("touch"));
    }

    #[test]
    fn command_candidates_must_resolve_to_executable_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let command = dir.path().join("tool");
        fs::write(&command, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!command_exists_in("tool", dir.path().as_os_str()));
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(command_exists_in("tool", dir.path().as_os_str()));
        symlink(&command, dir.path().join("linked-tool")).unwrap();
        assert!(command_exists_in("linked-tool", dir.path().as_os_str()));
        symlink(dir.path(), dir.path().join("directory-tool")).unwrap();
        assert!(!command_exists_in("directory-tool", dir.path().as_os_str()));
    }
}
