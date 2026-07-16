use super::v1::{
    AptSourcesIntent, AptUpdatePolicy, DesktopAction, DesktopTarget, DirectPackageIntent,
    FlatpakRemote, GoSelector, IntegrationAction, NodeSelector, PlanV1, PlannedAction,
    Prerequisite, RustSelector, SystemAction, ToolInstall, ToolUpdate, UpdateAction,
};
use crate::{
    config::v1::{DirectFormat, Theme},
    operations::{
        AptUpgradePolicy, CargoBinstallBootstrapOperation, CargoPackageMode, CargoPackageOperation,
        DesktopEnvironment, DesktopSetting, DesktopSettingOperation, DesktopTheme,
        DirectPackageFormat, DirectPackageMode, DirectPackageOperation, DirectPackageSelector,
        DockerLocalLogOperation, DotfilesOperation, EnsureAdminOperation, GithubRepository,
        GnomeDockOperation, GnomeExtensionsOperation, GnomeRoundedCornersOperation,
        GoToolchainOperation, GoToolchainSelector, ManagedAptSourcesOperation, NerdFontsOperation,
        NodeToolchainOperation, NodeToolchainSelector, NpmPackageMode, NpmPackageOperation,
        Operation, PythonToolchainOperation, RustToolchainOperation, RustToolchainSelector,
        ToolMutationMode, UbuntuSnapOperation, UnattendedUpgradesOperation,
        VsCodeExtensionOperation,
    },
    runner::Step,
};
use anyhow::{Context, Result};
use std::collections::BTreeSet;

pub fn lower(plan: &PlanV1) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    let cargo_binstall_architecture = plan.actions.iter().find_map(|action| match action {
        PlannedAction::Tool(ToolInstall::Rust { architecture, .. }) => Some(*architecture),
        _ => None,
    });
    for action in &plan.actions {
        lower_action(action, cargo_binstall_architecture, &mut steps)?;
    }
    Ok(steps)
}

fn lower_action(
    action: &PlannedAction,
    cargo_binstall_architecture: Option<crate::platform::Architecture>,
    steps: &mut Vec<Step>,
) -> Result<()> {
    match action {
        PlannedAction::Bootstrap(bootstrap) => {
            lower_bootstrap(&bootstrap.prerequisites, cargo_binstall_architecture, steps)?
        }
        PlannedAction::System(action) => lower_system(action, steps)?,
        PlannedAction::AptMetadataRefresh => push(steps, Operation::AptMetadataRefresh),
        PlannedAction::RemovePackages(packages) => push(
            steps,
            Operation::AptPurge {
                packages: packages.clone(),
            },
        ),
        PlannedAction::Repository(repository) => {
            push(
                steps,
                Operation::RepositoryKey {
                    url: repository.key_url.as_str().into(),
                    destination: path_string(&repository.keyring_path, "repository keyring")?,
                },
            );
            let components = repository
                .components
                .iter()
                .map(|component| component.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let source = format!(
                "deb [arch={} signed-by={}] {} {} {}\n",
                repository.architecture.debian(),
                repository.keyring_path.display(),
                repository.source_url.as_str(),
                repository.suite.value(),
                components,
            );
            push(
                steps,
                Operation::AptSource {
                    destination: path_string(&repository.source_list_path, "repository source")?,
                    contents: source,
                },
            );
        }
        PlannedAction::RepositoryPackages(group) => push(
            steps,
            Operation::AptPackages {
                packages: group.packages.clone(),
            },
        ),
        PlannedAction::AptPackages(packages) => push(
            steps,
            Operation::AptPackages {
                packages: packages.clone(),
            },
        ),
        PlannedAction::Flatpak(install) => {
            let FlatpakRemote::Flathub = install.remote;
            push(
                steps,
                Operation::FlatpakEnsureApps {
                    refs: install.refs.clone(),
                },
            );
        }
        PlannedAction::Tool(tool) => push(steps, lower_tool_install(tool)?),
        PlannedAction::CargoPackages(packages) => push(
            steps,
            Operation::CargoPackageSet(CargoPackageOperation::new(
                packages.clone(),
                CargoPackageMode::EnsurePresent,
            )?),
        ),
        PlannedAction::NpmPackages(packages) => push(
            steps,
            Operation::NpmPackageSet(NpmPackageOperation::new(
                packages.clone(),
                NpmPackageMode::EnsurePresent,
            )?),
        ),
        PlannedAction::DirectPackage(package) => push(
            steps,
            lower_direct(package, DirectPackageMode::EnsurePresent)?,
        ),
        PlannedAction::NerdFonts(families) => push(
            steps,
            Operation::NerdFonts(NerdFontsOperation::new(families.clone())?),
        ),
        PlannedAction::Dotfiles(dotfiles) => push(
            steps,
            Operation::Dotfiles(DotfilesOperation::new(
                dotfiles.root.clone(),
                dotfiles.packages.clone(),
            )?),
        ),
        PlannedAction::Integration(integration) => lower_integration(integration, steps)?,
        PlannedAction::Desktop(desktop) => push(steps, lower_desktop(desktop)?),
        PlannedAction::Update(update) => lower_update(update, steps)?,
    }
    Ok(())
}

fn lower_bootstrap(
    prerequisites: &BTreeSet<Prerequisite>,
    cargo_binstall_architecture: Option<crate::platform::Architecture>,
    steps: &mut Vec<Step>,
) -> Result<()> {
    let mut packages = BTreeSet::new();
    for prerequisite in prerequisites {
        packages.extend(match prerequisite {
            Prerequisite::HttpsDownloader => ["ca-certificates", "curl"].as_slice(),
            Prerequisite::OpenPgpRepositorySupport => ["gnupg"].as_slice(),
            Prerequisite::FlatpakFlathub => ["flatpak"].as_slice(),
            Prerequisite::Rustup => [].as_slice(),
            Prerequisite::CargoBinstall => [].as_slice(),
            Prerequisite::GoArchives => ["tar"].as_slice(),
            Prerequisite::FnmNpm => ["unzip"].as_slice(),
            Prerequisite::Uv => [].as_slice(),
            Prerequisite::Stow => ["stow"].as_slice(),
            Prerequisite::DirectDeb | Prerequisite::DirectAppImage => [].as_slice(),
            Prerequisite::NerdFonts => ["fontconfig", "xz-utils"].as_slice(),
            Prerequisite::GnomeTools => ["dconf-cli", "gnome-shell"].as_slice(),
        });
    }
    if !packages.is_empty() {
        push(
            steps,
            Operation::AptBootstrapPackages {
                packages: packages.into_iter().map(str::to_owned).collect(),
            },
        );
    }
    if prerequisites.contains(&Prerequisite::Rustup) {
        push(steps, Operation::RustupBootstrap);
    }
    if prerequisites.contains(&Prerequisite::CargoBinstall) {
        push(
            steps,
            Operation::CargoBinstallBootstrap(CargoBinstallBootstrapOperation::new(
                cargo_binstall_architecture
                    .context("cargo-binstall bootstrap requires a planned Rust architecture")?,
            )),
        );
    }
    if prerequisites.contains(&Prerequisite::FnmNpm) {
        push(steps, Operation::FnmBootstrap);
    }
    if prerequisites.contains(&Prerequisite::Uv) {
        push(steps, Operation::UvBootstrap);
    }
    if prerequisites.contains(&Prerequisite::FlatpakFlathub) {
        push(steps, Operation::FlatpakEnsureFlathub);
    }
    Ok(())
}

fn lower_system(action: &SystemAction, steps: &mut Vec<Step>) -> Result<()> {
    match action {
        SystemAction::AptSources(AptSourcesIntent::Managed(policy)) => push(
            steps,
            Operation::ManagedAptSources(ManagedAptSourcesOperation::new(
                policy.distro.clone(),
                policy.release.clone(),
                policy.architecture,
                policy.components.clone(),
            )?),
        ),
        SystemAction::EnsureAdmin => {
            push(steps, Operation::EnsureAdmin(EnsureAdminOperation::new()))
        }
        SystemAction::UnattendedUpgrades { enabled } => push(
            steps,
            Operation::UnattendedUpgrades(UnattendedUpgradesOperation::new(*enabled)),
        ),
        SystemAction::UbuntuSnap { enabled } => push(
            steps,
            Operation::UbuntuSnap(UbuntuSnapOperation::new(*enabled)),
        ),
        SystemAction::UbuntuCodecs => push(
            steps,
            Operation::AptPackages {
                packages: vec!["ubuntu-restricted-extras".into()],
            },
        ),
    }
    Ok(())
}

fn lower_tool_install(tool: &ToolInstall) -> Result<Operation> {
    Ok(match tool {
        ToolInstall::Rust {
            selector,
            architecture,
        } => Operation::RustToolchain(RustToolchainOperation::new(
            rust_selector(selector),
            *architecture,
            ToolMutationMode::EnsurePresent,
        )?),
        ToolInstall::Go {
            selector,
            architecture,
        } => Operation::GoToolchain(GoToolchainOperation::new(
            go_selector(selector),
            *architecture,
            ToolMutationMode::EnsurePresent,
        )?),
        ToolInstall::Node {
            selector,
            architecture,
        } => Operation::NodeToolchain(NodeToolchainOperation::new(
            node_selector(selector),
            *architecture,
            ToolMutationMode::EnsurePresent,
        )?),
        ToolInstall::Python {
            version,
            architecture,
        } => Operation::PythonToolchain(PythonToolchainOperation::new(
            version.clone(),
            *architecture,
        )?),
    })
}

fn lower_direct(package: &DirectPackageIntent, mode: DirectPackageMode) -> Result<Operation> {
    let format = match package.format {
        DirectFormat::Deb => DirectPackageFormat::Deb,
        DirectFormat::Appimage => DirectPackageFormat::AppImage,
    };
    Ok(Operation::DirectPackage(DirectPackageOperation::new(
        package.name.clone(),
        format,
        package.provides.clone(),
        GithubRepository::parse(package.source.repository.clone())?,
        package.source.architecture,
        DirectPackageSelector::new(
            package.source.selector.include.clone(),
            package.source.selector.exclude.clone(),
        )?,
        mode,
    )?))
}

fn lower_integration(integration: &IntegrationAction, steps: &mut Vec<Step>) -> Result<()> {
    match integration {
        IntegrationAction::DockerGroup => push(steps, Operation::DockerGroup),
        IntegrationAction::DockerLocalLog { max_size } => push(
            steps,
            Operation::DockerLocalLog(DockerLocalLogOperation::new(max_size.clone())?),
        ),
        IntegrationAction::VirtualBoxGroup => push(steps, Operation::VirtualBoxGroup),
        IntegrationAction::VsCodeExtensions(extensions) => push(
            steps,
            Operation::VsCodeExtensionSet(VsCodeExtensionOperation::new(extensions.clone())?),
        ),
    }
    Ok(())
}

fn lower_desktop(action: &DesktopAction) -> Result<Operation> {
    Ok(match action {
        DesktopAction::Theme { target, theme } => {
            Operation::DesktopSetting(DesktopSettingOperation::new(
                desktop_target(*target),
                DesktopSetting::Theme(match theme {
                    Theme::Light => DesktopTheme::Light,
                    Theme::Dark => DesktopTheme::Dark,
                }),
            )?)
        }
        DesktopAction::Terminal { target, executable } => {
            Operation::DesktopSetting(DesktopSettingOperation::new(
                desktop_target(*target),
                DesktopSetting::Terminal(executable.clone()),
            )?)
        }
        DesktopAction::IdleTimeout { target, timeout } => {
            Operation::DesktopSetting(DesktopSettingOperation::new(
                desktop_target(*target),
                DesktopSetting::IdleTimeoutSeconds(duration_seconds(timeout)?),
            )?)
        }
        DesktopAction::IdleDim { target, enabled } => {
            Operation::DesktopSetting(DesktopSettingOperation::new(
                desktop_target(*target),
                DesktopSetting::IdleDim(*enabled),
            )?)
        }
        DesktopAction::GnomeExtensions(extensions) => {
            Operation::GnomeExtensions(GnomeExtensionsOperation::new(extensions.clone())?)
        }
        DesktopAction::GnomeDock => Operation::GnomeDock(GnomeDockOperation::new()),
        DesktopAction::GnomeRoundedCorners => {
            Operation::GnomeRoundedCorners(GnomeRoundedCornersOperation::new())
        }
    })
}

fn lower_update(update: &UpdateAction, steps: &mut Vec<Step>) -> Result<()> {
    match update {
        UpdateAction::Apt { policy, .. } => push(
            steps,
            Operation::AptUpgrade {
                policy: match policy {
                    AptUpdatePolicy::Standard => AptUpgradePolicy::Standard,
                    AptUpdatePolicy::Full => AptUpgradePolicy::Full,
                },
            },
        ),
        UpdateAction::Flatpak { refs, .. } => {
            push(steps, Operation::FlatpakUpdateApps { refs: refs.clone() })
        }
        UpdateAction::Tool(tool) => push(steps, lower_tool_update(tool)?),
        UpdateAction::Cargo { packages } => push(
            steps,
            Operation::CargoPackageSet(CargoPackageOperation::new(
                packages.clone(),
                CargoPackageMode::UpdateCurrent,
            )?),
        ),
        UpdateAction::Npm { packages } => push(
            steps,
            Operation::NpmPackageSet(NpmPackageOperation::new(
                packages.clone(),
                NpmPackageMode::UpdateCurrent,
            )?),
        ),
        UpdateAction::Direct { packages } => {
            for package in packages {
                push(steps, lower_direct(package, DirectPackageMode::Update)?);
            }
        }
    }
    Ok(())
}

fn lower_tool_update(tool: &ToolUpdate) -> Result<Operation> {
    Ok(match tool {
        ToolUpdate::Rust {
            selector,
            architecture,
        } => Operation::RustToolchain(RustToolchainOperation::new(
            rust_selector(selector),
            *architecture,
            ToolMutationMode::UpdateMoving,
        )?),
        ToolUpdate::Go {
            selector,
            architecture,
        } => Operation::GoToolchain(GoToolchainOperation::new(
            go_selector(selector),
            *architecture,
            ToolMutationMode::UpdateMoving,
        )?),
        ToolUpdate::Node {
            selector,
            architecture,
        } => Operation::NodeToolchain(NodeToolchainOperation::new(
            node_selector(selector),
            *architecture,
            ToolMutationMode::UpdateMoving,
        )?),
    })
}

fn rust_selector(selector: &RustSelector) -> RustToolchainSelector {
    match selector {
        RustSelector::Stable => RustToolchainSelector::Stable,
        RustSelector::Beta => RustToolchainSelector::Beta,
        RustSelector::Nightly => RustToolchainSelector::Nightly,
        RustSelector::DatedNightly(value) => RustToolchainSelector::DatedNightly(value.clone()),
        RustSelector::Version(value) => RustToolchainSelector::Version(value.clone()),
    }
}

fn go_selector(selector: &GoSelector) -> GoToolchainSelector {
    match selector {
        GoSelector::Latest => GoToolchainSelector::Latest,
        GoSelector::Version(value) => GoToolchainSelector::Version(value.clone()),
    }
}

fn node_selector(selector: &NodeSelector) -> NodeToolchainSelector {
    match selector {
        NodeSelector::Lts => NodeToolchainSelector::Lts,
        NodeSelector::Latest => NodeToolchainSelector::Latest,
        NodeSelector::Version(value) => NodeToolchainSelector::Version(value.clone()),
    }
}

fn desktop_target(target: DesktopTarget) -> DesktopEnvironment {
    match target {
        DesktopTarget::Gnome => DesktopEnvironment::Gnome,
        DesktopTarget::Cinnamon => DesktopEnvironment::Cinnamon,
    }
}

fn duration_seconds(value: &str) -> Result<u32> {
    let (number, multiplier) = if let Some(number) = value.strip_suffix('s') {
        (number, 1_u64)
    } else if let Some(number) = value.strip_suffix('m') {
        (number, 60)
    } else {
        (
            value
                .strip_suffix('h')
                .context("invalid desktop idle duration")?,
            3600,
        )
    };
    number
        .parse::<u64>()
        .context("invalid desktop idle duration")?
        .checked_mul(multiplier)
        .and_then(|seconds| u32::try_from(seconds).ok())
        .context("desktop idle duration exceeds the supported uint32 range")
}

fn path_string(path: &std::path::Path, description: &str) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .with_context(|| format!("{description} path is not UTF-8: {}", path.display()))
}

fn push(steps: &mut Vec<Step>, operation: Operation) {
    steps.push(Step::workflow(operation));
}

#[cfg(test)]
mod tests {
    use super::duration_seconds;

    #[test]
    fn idle_durations_lower_to_checked_seconds() {
        assert_eq!(duration_seconds("0s").unwrap(), 0);
        assert_eq!(duration_seconds("15m").unwrap(), 900);
        assert_eq!(duration_seconds("2h").unwrap(), 7200);
        assert!(duration_seconds("4294967296s").is_err());
    }
}
