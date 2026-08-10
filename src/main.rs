use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};

mod config;
mod init;
mod operations;
mod planner;
mod platform;

#[derive(Debug, Parser)]
#[command(name = "cozydot", version, about = "Provision Linux and macOS from one active configuration")]
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
    /// Validate the active configuration
    Check,
    /// Apply configured dotfiles
    Dotfiles {
        /// Back up conflicting files before replacing them with Cozydot links
        #[arg(short = 'r', long)]
        replace: bool,
    },
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
        CliCommand::Init { preset } => init_command_workflow(preset)?,
        CliCommand::Apply => apply_command_workflow()?,
        CliCommand::Check => check_command_workflow()?,
        CliCommand::Dotfiles { replace } => dotfiles_command_workflow(replace)?,
        CliCommand::Update => update_command_workflow()?,
    }
    Ok(())
}

fn init_command_workflow(preset: init::Preset) -> Result<()> {
    println!("Initialized cozydot in {}", init::initialization_workflow(preset)?.display());
    Ok(())
}

fn check_command_workflow() -> Result<()> {
    let path = active_configuration_path()?;
    config::Config::load(&path)
        .with_context(|| "active configuration is missing or invalid; run 'cozydot init' first")?;
    println!("Checked {}", path.display());
    Ok(())
}

fn apply_command_workflow() -> Result<()> {
    let root = init::config_root()?;
    let path = root.join("cozydot.yaml");
    let config = config::Config::load(&path)
        .with_context(|| "active configuration is missing or invalid; run 'cozydot init' first")?;
    let platform = platform::Platform::detect()?;
    let operations = planner::plan_apply(&config, &platform, &root.join("dotfiles"))?;
    execute_operation_plan("Applying", operations)
}

fn dotfiles_command_workflow(replace: bool) -> Result<()> {
    let root = init::config_root()?;
    let path = root.join("cozydot.yaml");
    let config = config::Config::load(&path)
        .with_context(|| "active configuration is missing or invalid; run 'cozydot init' first")?;
    let platform = platform::Platform::detect()?;
    let operations = planner::plan_standalone_dotfiles(&config, &platform, &root.join("dotfiles"), replace)?;
    execute_operation_plan("Applying", operations)
}

fn update_command_workflow() -> Result<()> {
    let path = active_configuration_path()?;
    let config = config::Config::load(&path)
        .with_context(|| "active configuration is missing or invalid; run 'cozydot init' first")?;
    let platform = platform::Platform::detect()?;
    let operations = planner::plan_update(&config, &platform)?;
    execute_operation_plan("Updating", operations)
}

fn active_configuration_path() -> Result<std::path::PathBuf> {
    Ok(init::config_root()?.join("cozydot.yaml"))
}

fn execute_operation_plan(progress: &str, operations: Vec<operations::Operation>) -> Result<()> {
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
