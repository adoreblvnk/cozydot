use crate::operations::{self, Operation, OperationOutcome};
use anyhow::Result;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionPhase {
    SystemPrerequisites,
    ManagerBootstraps,
    AdministrativeVerification,
    OfficialAptSources,
    ThirdPartyRepositories,
    AptMetadataRefresh,
    SystemPackageStates,
    AptPurge,
    RepositoryPackages,
    AptPackages,
    FlatpakApplications,
    LanguageToolchains,
    LanguagePackages,
    BinaryPackages,
    Fonts,
    Dotfiles,
    Integrations,
    Desktop,
    Updates,
    FinalVerification,
}

impl ExecutionPhase {
    pub const ORDERED: [Self; 20] = [
        Self::SystemPrerequisites,
        Self::ManagerBootstraps,
        Self::AdministrativeVerification,
        Self::OfficialAptSources,
        Self::ThirdPartyRepositories,
        Self::AptMetadataRefresh,
        Self::SystemPackageStates,
        Self::AptPurge,
        Self::RepositoryPackages,
        Self::AptPackages,
        Self::FlatpakApplications,
        Self::LanguageToolchains,
        Self::LanguagePackages,
        Self::BinaryPackages,
        Self::Fonts,
        Self::Dotfiles,
        Self::Integrations,
        Self::Desktop,
        Self::Updates,
        Self::FinalVerification,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::SystemPrerequisites => "system-prerequisites",
            Self::ManagerBootstraps => "manager-bootstraps",
            Self::AdministrativeVerification => "administrative-verification",
            Self::OfficialAptSources => "official-apt-sources",
            Self::ThirdPartyRepositories => "third-party-repositories",
            Self::AptMetadataRefresh => "apt-metadata-refresh",
            Self::SystemPackageStates => "system-package-states",
            Self::AptPurge => "apt-purge",
            Self::RepositoryPackages => "repository-packages",
            Self::AptPackages => "apt-packages",
            Self::FlatpakApplications => "flatpak-applications",
            Self::LanguageToolchains => "language-toolchains",
            Self::LanguagePackages => "language-packages",
            Self::BinaryPackages => "binary-packages",
            Self::Fonts => "fonts",
            Self::Dotfiles => "dotfiles",
            Self::Integrations => "integrations",
            Self::Desktop => "desktop",
            Self::Updates => "updates",
            Self::FinalVerification => "final-verification",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkippedAction {
    UbuntuSnap,
    UbuntuCodecs,
}

impl SkippedAction {
    fn name(self) -> &'static str {
        match self {
            Self::UbuntuSnap => "ubuntu-snap",
            Self::UbuntuCodecs => "ubuntu-codecs",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    RequiresUbuntuFamily,
}

impl SkipReason {
    fn description(self) -> &'static str {
        match self {
            Self::RequiresUbuntuFamily => "requires Ubuntu family",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplicitSkip {
    pub action: SkippedAction,
    pub reason: SkipReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepKind {
    Phase(ExecutionPhase),
    Operation {
        operation: Box<Operation>,
        label: Option<String>,
    },
    Skip(ExplicitSkip),
    Summary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step(StepKind);

impl Step {
    pub fn workflow(operation: Operation) -> Self {
        Self(StepKind::Operation {
            operation: Box::new(operation),
            label: None,
        })
    }

    pub fn labeled_workflow(operation: Operation, label: impl Into<String>) -> Result<Self> {
        let label = label.into();
        if label.is_empty() || label.chars().any(char::is_control) {
            return Err(anyhow::anyhow!(
                "runner operation label must be nonempty printable text"
            ));
        }
        Ok(Self(StepKind::Operation {
            operation: Box::new(operation),
            label: Some(label),
        }))
    }

    pub fn phase(phase: ExecutionPhase) -> Self {
        Self(StepKind::Phase(phase))
    }

    pub fn skip(action: SkippedAction, reason: SkipReason) -> Self {
        Self(StepKind::Skip(ExplicitSkip { action, reason }))
    }

    pub fn summary() -> Self {
        Self(StepKind::Summary)
    }

    pub fn kind(&self) -> &StepKind {
        &self.0
    }

    pub fn operation(&self) -> &Operation {
        let StepKind::Operation { operation, .. } = &self.0 else {
            panic!("runner step is not an operation")
        };
        operation.as_ref()
    }

    pub fn display(&self) -> String {
        match &self.0 {
            StepKind::Phase(phase) => format!("phase {}", phase.name()),
            StepKind::Operation { operation, .. } => {
                format!("workflow {}", operation.display_args().join(" "))
            }
            StepKind::Skip(skip) => {
                format!("skip {} {}", skip.action.name(), skip.reason.description())
            }
            StepKind::Summary => "summary".into(),
        }
    }

    fn report_name(&self) -> String {
        match &self.0 {
            StepKind::Operation {
                label: Some(label), ..
            } => format!("{label}: {}", self.display()),
            _ => self.display(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    PhaseStarted,
    Completed,
    LoginRequired,
    Skipped,
    Planned,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepReport {
    pub step: Step,
    pub outcome: StepOutcome,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutionSummary {
    pub completed: usize,
    pub skipped: usize,
    pub login_required: usize,
    pub planned: usize,
    pub failed: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionReport {
    pub steps: Vec<StepReport>,
    pub summary: ExecutionSummary,
}

impl ExecutionReport {
    pub fn render(&self) -> String {
        let mut lines = self
            .steps
            .iter()
            .map(|report| match (&report.step.0, &report.outcome) {
                (StepKind::Phase(phase), StepOutcome::PhaseStarted) => {
                    format!("== phase: {}", phase.name())
                }
                (StepKind::Operation { .. }, StepOutcome::Completed) => {
                    format!("completed: {}", report.step.report_name())
                }
                (StepKind::Operation { .. }, StepOutcome::LoginRequired) => {
                    format!("login-required: {}", report.step.report_name())
                }
                (StepKind::Operation { .. }, StepOutcome::Planned) => {
                    format!("planned: {}", report.step.report_name())
                }
                (StepKind::Skip(skip), StepOutcome::Skipped) => format!(
                    "skipped: {} ({})",
                    skip.action.name(),
                    skip.reason.description()
                ),
                (StepKind::Operation { .. }, StepOutcome::Failed(error)) => {
                    format!("failed: {} ({error})", report.step.report_name())
                }
                _ => format!("invalid-report: {}", report.step.display()),
            })
            .collect::<Vec<_>>();
        lines.push(format!(
            "summary: {} completed, {} skipped, {} login-required, {} planned, {} failed",
            self.summary.completed,
            self.summary.skipped,
            self.summary.login_required,
            self.summary.planned,
            self.summary.failed,
        ));
        lines.join("\n")
    }

    fn push(&mut self, step: Step, outcome: StepOutcome) {
        match outcome {
            StepOutcome::Completed => self.summary.completed += 1,
            StepOutcome::Skipped => self.summary.skipped += 1,
            StepOutcome::LoginRequired => self.summary.login_required += 1,
            StepOutcome::Planned => self.summary.planned += 1,
            StepOutcome::Failed(_) => self.summary.failed += 1,
            StepOutcome::PhaseStarted => {}
        }
        self.steps.push(StepReport { step, outcome });
    }
}

#[derive(Debug)]
pub struct ExecutionFailure {
    pub report: ExecutionReport,
    source: anyhow::Error,
}

impl fmt::Display for ExecutionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.source)
    }
}

impl std::error::Error for ExecutionFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub struct ProcessRunner {
    pub dry_run: bool,
}

impl ProcessRunner {
    fn run(&mut self, operation: &Operation) -> Result<OperationOutcome> {
        operations::execute(operation, &[])
    }
}

pub fn execute(
    runner: &mut ProcessRunner,
    steps: &[Step],
) -> std::result::Result<ExecutionReport, ExecutionFailure> {
    let result = execute_with(steps, runner.dry_run, |operation| runner.run(operation));
    match &result {
        Ok(report) => println!("{}", report.render()),
        Err(failure) => println!("{}", failure.report.render()),
    }
    result
}

fn execute_with<F>(
    steps: &[Step],
    dry_run: bool,
    mut run: F,
) -> std::result::Result<ExecutionReport, ExecutionFailure>
where
    F: FnMut(&Operation) -> Result<OperationOutcome>,
{
    let mut report = ExecutionReport::default();
    for step in steps {
        match step.kind() {
            StepKind::Phase(_) => report.push(step.clone(), StepOutcome::PhaseStarted),
            StepKind::Skip(_) => report.push(step.clone(), StepOutcome::Skipped),
            StepKind::Summary => {}
            StepKind::Operation { .. } if dry_run => {
                report.push(step.clone(), StepOutcome::Planned)
            }
            StepKind::Operation { operation, .. } => match run(operation) {
                Ok(OperationOutcome::Completed) => {
                    report.push(step.clone(), StepOutcome::Completed)
                }
                Ok(OperationOutcome::LoginRequired) => {
                    report.push(step.clone(), StepOutcome::LoginRequired)
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    report.push(step.clone(), StepOutcome::Failed(message));
                    return Err(ExecutionFailure {
                        report,
                        source: error,
                    });
                }
            },
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn operation(name: &str) -> Step {
        match name {
            "apt" => Step::workflow(Operation::AptMetadataRefresh),
            "rustup" => Step::workflow(Operation::RustupBootstrap),
            "uv" => Step::workflow(Operation::UvBootstrap),
            _ => panic!("unknown test operation"),
        }
    }

    #[test]
    fn mixed_completion_skip_and_login_outcomes_are_reported_stably() {
        let steps = [
            Step::phase(ExecutionPhase::SystemPrerequisites),
            operation("apt"),
            Step::skip(SkippedAction::UbuntuSnap, SkipReason::RequiresUbuntuFamily),
            operation("rustup"),
            Step::phase(ExecutionPhase::FinalVerification),
            Step::summary(),
        ];
        let mut calls = 0;
        let report = execute_with(&steps, false, |_| {
            calls += 1;
            Ok(if calls == 1 {
                OperationOutcome::Completed
            } else {
                OperationOutcome::LoginRequired
            })
        })
        .unwrap();

        assert_eq!(
            report.summary,
            ExecutionSummary {
                completed: 1,
                skipped: 1,
                login_required: 1,
                planned: 0,
                failed: 0,
            }
        );
        assert_eq!(
            report.render(),
            "== phase: system-prerequisites\ncompleted: workflow apt-metadata-refresh\nskipped: ubuntu-snap (requires Ubuntu family)\nlogin-required: workflow rustup-bootstrap\n== phase: final-verification\nsummary: 1 completed, 1 skipped, 1 login-required, 0 planned, 0 failed"
        );
    }

    #[test]
    fn failure_stops_execution_and_keeps_the_partial_report() {
        let steps = [operation("apt"), operation("rustup"), operation("uv")];
        let mut calls = 0;
        let failure = execute_with(&steps, false, |_| {
            calls += 1;
            match calls {
                1 => Ok(OperationOutcome::Completed),
                2 => Err(anyhow!("provider failed")),
                _ => panic!("runner continued after failure"),
            }
        })
        .unwrap_err();

        assert_eq!(calls, 2);
        assert_eq!(failure.report.summary.completed, 1);
        assert_eq!(failure.report.summary.failed, 1);
        assert_eq!(failure.report.steps.len(), 2);
        assert_eq!(
            failure.report.render(),
            "completed: workflow apt-metadata-refresh\nfailed: workflow rustup-bootstrap (provider failed)\nsummary: 1 completed, 0 skipped, 0 login-required, 0 planned, 1 failed"
        );
    }

    #[test]
    fn empty_and_dry_run_reports_never_claim_unperformed_success() {
        let empty =
            execute_with(&[], false, |_| panic!("empty report executed an operation")).unwrap();
        assert_eq!(
            empty.render(),
            "summary: 0 completed, 0 skipped, 0 login-required, 0 planned, 0 failed"
        );

        let dry = execute_with(&[operation("apt")], true, |_| {
            panic!("dry run executed an operation")
        })
        .unwrap();
        assert_eq!(dry.summary.planned, 1);
        assert_eq!(dry.summary.completed, 0);
    }

    #[test]
    fn execution_phase_order_matches_the_frozen_twenty_phase_contract() {
        assert_eq!(
            ExecutionPhase::ORDERED.map(ExecutionPhase::name),
            [
                "system-prerequisites",
                "manager-bootstraps",
                "administrative-verification",
                "official-apt-sources",
                "third-party-repositories",
                "apt-metadata-refresh",
                "system-package-states",
                "apt-purge",
                "repository-packages",
                "apt-packages",
                "flatpak-applications",
                "language-toolchains",
                "language-packages",
                "binary-packages",
                "fonts",
                "dotfiles",
                "integrations",
                "desktop",
                "updates",
                "final-verification",
            ]
        );
    }
}
