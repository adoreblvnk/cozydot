use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use cozydot::{
    apply,
    init::{self, Preset},
};

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
        preset: Preset,
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
