use anyhow::{Context, Result};

pub mod bundle;
pub mod config;
mod domain;
pub mod init;
pub mod json_helpers;
mod operations;
pub mod planner;
pub mod platform;
mod runner;

pub fn apply() -> Result<()> {
    let root = init::config_root()?;
    let path = root.join("cozydot.yaml");
    let config = config::Config::load(&path)
        .with_context(|| "active config is missing or invalid; run 'cozydot init' first")?;
    let platform = platform::Platform::detect()?;
    let plan = planner::plan(&config, &platform, &root.join("dotfiles"))?;
    let steps = planner::lower_neutral::lower(&plan)?;
    let mut runner = runner::ProcessRunner {
        dry_run: std::env::var_os("COZYDOT_DRY_RUN").is_some(),
    };
    runner::execute(&mut runner, &steps)?;
    Ok(())
}
