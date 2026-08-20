//! Provision Linux & macOS from one config.

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
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Safely initialize or synchronize config & bundled dotfiles.
    Init {
        /// Choose config preset.
        #[arg(long, value_enum, default_value = "cozydot")]
        preset: init::Preset,
    },
    /// Check active config.
    Check,
    /// Apply active config to this host.
    Apply,
    /// Apply configured dotfiles.
    Dotfiles {
        /// Back up conflicts before replacing with Cozydot links.
        #[arg(short = 'r', long)]
        replace: bool,
    },
    /// Run enabled updates.
    Update,
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

fn main() -> Result<()> {
    let Some(command) = Cli::parse().command else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };
    match command {
        Command::Init { preset } => println!("Initialized cozydot in {}", init::init(preset)?.display()),
        Command::Check => {
            let path = init::config_root()?.join("cozydot.yaml");
            let context = "active configuration is missing or invalid; run 'cozydot init' first";
            config::Config::load(&path).with_context(|| context)?;
            println!("Checked {}", path.display());
        }
        Command::Apply => {
            let host = ActiveHost::load()?;
            workflow::apply(&host.config, &host.platform, &host.root.join("dotfiles"))?;
        }
        Command::Dotfiles { replace } => {
            let host = ActiveHost::load()?;
            workflow::dotfiles(&host.config, &host.platform, &host.root.join("dotfiles"), replace)?;
        }
        Command::Update => {
            let host = ActiveHost::load()?;
            workflow::update(&host.config, &host.platform)?;
        }
    }
    Ok(())
}
