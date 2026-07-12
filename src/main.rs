use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cozydot::{
    config::Config,
    planner,
    platform::Platform,
    runner::{execute, ProcessRunner},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    version,
    about = "Automated Linux post-install, update, and dotfile manager"
)]
struct Cli {
    #[arg(short = 'c', long, default_value = "default", value_name = "CONFIG")]
    config: String,
    #[arg(short = 'n', long)]
    no_color: bool,
    #[arg(long)]
    list_configs: bool,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    Check,
    #[command(alias = "i")]
    Install,
    #[command(alias = "u")]
    Update,
    #[command(alias = "c")]
    Configure,
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = installation_root();
    if cli.list_configs {
        return list_configs(&root);
    }
    let action = cli
        .command
        .context("a command is required (check, install, update, configure)")?;
    let name = match action {
        Action::Check => "check",
        Action::Install => "install",
        Action::Update => "update",
        Action::Configure => "configure",
    };
    let path = config_path(&root, &cli.config);
    let cfg = Config::load(&path)?;
    let p = Platform::detect(
        cfg.string("metadata.distro").as_deref().unwrap_or("auto"),
        cfg.string("metadata.DE").as_deref().unwrap_or("auto"),
    )?;
    let steps = planner::plan(name, &cfg, &p, &root)?;
    let mut runner = ProcessRunner {
        dry_run: std::env::var_os("COZYDOT_DRY_RUN").is_some(),
    };
    execute(&mut runner, &steps)?;
    println!("Finished cozydot {name}");
    Ok(())
}
fn installation_root() -> PathBuf {
    std::env::var_os("COZYDOT_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .filter(|p| p.join("configs").is_dir())
        })
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn config_path(root: &Path, name: &str) -> PathBuf {
    let p = PathBuf::from(name);
    if p.extension().is_some() || p.components().count() > 1 {
        p
    } else {
        root.join("configs").join(format!("{name}.yaml"))
    }
}
fn list_configs(root: &Path) -> Result<()> {
    println!("Available configs:");
    let mut entries = fs::read_dir(root.join("configs"))?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if e.path().extension().and_then(|x| x.to_str()) == Some("yaml") {
            let c = Config::load(&e.path())?;
            println!(
                "  {}: {}",
                e.path().file_stem().unwrap().to_string_lossy(),
                c.string("metadata.description").unwrap_or_default()
            );
        }
    }
    Ok(())
}
