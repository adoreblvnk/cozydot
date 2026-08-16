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
    /// Apply active config to this host.
    Apply,
    /// Check active config.
    Check,
    /// Apply configured dotfiles.
    Dotfiles {
        /// Back up conflicts before replacing with Cozydot links.
        #[arg(short = 'r', long)]
        replace: bool,
    },
    /// Run enabled updates.
    Update,
}

fn main() -> Result<()> {
    let Some(command) = Cli::parse().command else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };
    match command {
        Command::Init { preset } => println!("Initialized cozydot in {}", init::init(preset)?.display()),
        Command::Apply => apply()?,
        Command::Check => check()?,
        Command::Dotfiles { replace } => dotfiles(replace)?,
        Command::Update => update()?,
    }
    Ok(())
}

fn check() -> Result<()> {
    let path = init::config_root()?.join("cozydot.yaml");
    config::Config::load(&path)
        .with_context(|| "active configuration is missing or invalid; run 'cozydot init' first")?;
    println!("Checked {}", path.display());
    Ok(())
}

fn apply() -> Result<()> {
    let host = ActiveHost::load()?;
    workflow::apply(&host.config, &host.platform, &host.root.join("dotfiles"))
}

fn dotfiles(replace: bool) -> Result<()> {
    let host = ActiveHost::load()?;
    workflow::dotfiles(&host.config, &host.platform, &host.root.join("dotfiles"), replace)
}

fn update() -> Result<()> {
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
