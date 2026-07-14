use crate::operations::{self, Operation};
use anyhow::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step(Operation);

impl Step {
    pub fn workflow(operation: Operation) -> Self {
        Self(operation)
    }

    pub fn operation(&self) -> &Operation {
        &self.0
    }

    pub fn display(&self) -> String {
        format!("workflow {}", self.0.display_args().join(" "))
    }
}

pub struct ProcessRunner {
    pub dry_run: bool,
}

impl ProcessRunner {
    fn run(&mut self, step: &Step) -> Result<()> {
        println!("+ {}", step.display());
        if self.dry_run {
            return Ok(());
        }
        operations::execute(step.operation(), &[])
    }
}

pub fn execute(runner: &mut ProcessRunner, steps: &[Step]) -> Result<()> {
    for step in steps {
        runner.run(step)?;
    }
    Ok(())
}
