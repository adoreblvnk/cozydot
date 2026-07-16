use crate::operations::{self, Operation, OperationOutcome};
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
    fn run(&mut self, step: &Step) -> Result<OperationOutcome> {
        println!("+ {}", step.display());
        if self.dry_run {
            return Ok(OperationOutcome::Completed);
        }
        operations::execute(step.operation(), &[])
    }
}

pub fn execute(runner: &mut ProcessRunner, steps: &[Step]) -> Result<OperationOutcome> {
    let mut outcome = OperationOutcome::Completed;
    for step in steps {
        outcome = merge_outcomes(outcome, runner.run(step)?);
    }
    Ok(outcome)
}

fn merge_outcomes(left: OperationOutcome, right: OperationOutcome) -> OperationOutcome {
    if left == OperationOutcome::LoginRequired || right == OperationOutcome::LoginRequired {
        OperationOutcome::LoginRequired
    } else {
        OperationOutcome::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_boundaries_remain_visible_in_aggregate_outcomes() {
        assert_eq!(
            merge_outcomes(OperationOutcome::Completed, OperationOutcome::Completed),
            OperationOutcome::Completed
        );
        assert_eq!(
            merge_outcomes(OperationOutcome::LoginRequired, OperationOutcome::Completed),
            OperationOutcome::LoginRequired
        );
        assert_eq!(
            merge_outcomes(OperationOutcome::Completed, OperationOutcome::LoginRequired),
            OperationOutcome::LoginRequired
        );
    }
}
