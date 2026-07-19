use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};

mod config;
mod init;
mod operations;
mod planner;
mod platform;
mod runner;

#[derive(Debug, Parser)]
#[command(
    name = "cozydot",
    version,
    about = "Provision a Linux system from one active configuration"
)]
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };
    match command {
        CliCommand::Init { preset } => {
            println!("Initialized cozydot in {}", init::run(preset)?.display());
        }
        CliCommand::Apply => apply()?,
    }
    Ok(())
}

fn apply() -> Result<()> {
    let root = init::config_root()?;
    let path = root.join("cozydot.yaml");
    let config =
        config::Config::load(&path).with_context(|| "active config is missing or invalid; run 'cozydot init' first")?;
    let platform = platform::Platform::detect()?;
    let steps = planner::plan(&config, &platform, &root.join("dotfiles"))?;
    let mut runner_inst = runner::ProcessRunner;
    runner::execute(&mut runner_inst, &steps)?;
    Ok(())
}
