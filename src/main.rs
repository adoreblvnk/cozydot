use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};

mod config;
mod init;
mod operations;
mod platform;
mod workflow;

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
    let host = ActiveHost::load()?;
    workflow::apply(&host.config, &host.platform, &host.root.join("dotfiles"))
}

fn dotfiles_command_workflow(replace: bool) -> Result<()> {
    let host = ActiveHost::load()?;
    workflow::dotfiles(&host.config, &host.platform, &host.root.join("dotfiles"), replace)
}

fn update_command_workflow() -> Result<()> {
    let host = ActiveHost::load()?;
    workflow::update(&host.config, &host.platform)
}

struct ActiveHost {
    root: std::path::PathBuf,
    config: config::Config,
    platform: platform::Platform,
}

impl ActiveHost {
    fn load() -> Result<Self> {
        let root = init::config_root()?;
        let config = config::Config::load(&root.join("cozydot.yaml"))
            .with_context(|| "active configuration is missing or invalid; run 'cozydot init' first")?;
        let platform = platform::Platform::detect()?;
        config.validate_for_platform(&platform)?;
        Ok(Self { root, config, platform })
    }
}

fn active_configuration_path() -> Result<std::path::PathBuf> {
    Ok(init::config_root()?.join("cozydot.yaml"))
}
