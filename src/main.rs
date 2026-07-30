use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};

mod config;
mod init;
mod operations;
mod planner;
mod platform;

#[derive(Debug, Parser)]
#[command(name = "cozydot", version, about = "Provision a Linux system from one active configuration")]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Create or safely refresh the config and bundled dotfiles
    Init {
        /// Configuration preset to materialize
        #[arg(long, value_enum, default_value = "cozydot")]
        preset: init::Preset,
    },
    /// Apply the active configuration to this host
    Apply,
    /// Run enabled ecosystem updates
    Update,
}

fn main() -> Result<()> {
    let Some(command) = Cli::parse().command else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };
    match command {
        CliCommand::Init { preset } => {
            println!("Initialized cozydot in {}", init::run(preset)?.display());
        }
        CliCommand::Apply => {
            run("Applying", |config, platform, root| planner::plan_apply(config, platform, &root.join("dotfiles")))?
        }
        CliCommand::Update => run("Updating", |config, platform, _| planner::plan_update(config, platform))?,
    }
    Ok(())
}

fn run(
    progress: &str,
    plan: impl FnOnce(&config::Config, &platform::Platform, &std::path::Path) -> Result<Vec<operations::Operation>>,
) -> Result<()> {
    let root = init::config_root()?;
    let path = root.join("cozydot.yaml");
    let config =
        config::Config::load(&path).with_context(|| "active config is missing or invalid; run 'cozydot init' first")?;
    let platform = platform::Platform::detect()?;
    let operations = plan(&config, &platform, &root)?;
    for operation in operations {
        let label = operation.label();
        println!("{progress} {label}");
        if matches!(
            operations::execute(&operation).with_context(|| format!("{} {label}", progress.to_lowercase()))?,
            operations::OperationOutcome::LoginRequired
        ) {
            println!("Login required to finish {label}");
        }
    }
    Ok(())
}
