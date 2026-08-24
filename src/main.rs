//! Provision Linux & macOS from one config.

use anyhow::{Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand};
use std::{
    io,
    path::{Path, PathBuf},
};

use crate::style::ERROR;

mod config;
mod init;
mod operations;
mod paths;
mod platform;
mod style;
mod workflow;

#[derive(Debug, Parser)]
#[command(name = "cozydot", version, about = "Declarative Linux and macOS post-install, update, and dotfile manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize or synchronize config & bundled dotfiles.
    Init {
        /// Select a config preset.
        #[arg(long, value_enum, default_value = "cozydot")]
        preset: init::Preset,
    },
    /// Validate active config.
    Check,
    /// Apply active config to this host.
    Apply,
    /// Apply configured dotfiles.
    Dotfiles {
        /// Back up conflicts before replacing them with Cozydot links.
        #[arg(short = 'r', long)]
        replace: bool,
    },
    /// Update configured software.
    Update,
}

struct ActiveHost {
    config_dir: PathBuf,
    config: config::Config,
    platform: platform::Platform,
}

impl ActiveHost {
    fn load() -> Result<Self> {
        let config_dir = paths::config_dir()?;
        let config = load_active_config(&config_dir.join("cozydot.yaml"))?;
        let platform = platform::Platform::detect()?;
        config.validate_for_platform(&platform)?;
        Ok(Self { config_dir, config, platform })
    }
}

fn load_active_config(path: &Path) -> Result<config::Config> {
    match config::Config::load(path) {
        Err(error)
            if error.chain().any(|cause| {
                cause.downcast_ref::<io::Error>().is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
            }) =>
        {
            Err(anyhow!("active config not found at {}; run `cozydot init`", path.display()))
        }
        result => result,
    }
}

fn main() {
    let result = (|| -> Result<()> {
        let Some(command) = Cli::parse().command else {
            Cli::command().print_help()?;
            println!();
            return Ok(());
        };
        match command {
            Command::Init { preset } => println!("Initialized Cozydot at {}", init::init(preset)?.display()),
            Command::Check => {
                let path = paths::config_dir()?.join("cozydot.yaml");
                load_active_config(&path)?;
                println!("Validated {}", path.display());
            }
            Command::Apply => {
                let host = ActiveHost::load()?;
                workflow::apply(&host.config, &host.platform, &host.config_dir.join("dotfiles"))?;
            }
            Command::Dotfiles { replace } => {
                let host = ActiveHost::load()?;
                workflow::dotfiles(&host.config, &host.platform, &host.config_dir.join("dotfiles"), replace)?;
            }
            Command::Update => {
                let host = ActiveHost::load()?;
                workflow::update(&host.config, &host.platform)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        anstream::eprintln!("{ERROR}error:{ERROR:#} {error}");
        for cause in error.chain().skip(1) {
            let cause = cause.to_string().replace('\n', "\n  ");
            eprintln!("\nCaused by:\n  {cause}");
        }
        std::process::exit(1);
    }
}
