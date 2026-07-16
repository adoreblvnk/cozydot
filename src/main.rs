use anyhow::{bail, Context, Result};
use cozydot::{
    config::Config,
    init, planner,
    platform::Platform,
    runner::{execute, ProcessRunner},
};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if args.next().is_some() {
        print_help();
        bail!("expected one command");
    }
    match command.as_deref() {
        Some("-h" | "--help") | None => print_help(),
        Some("-V" | "--version") => println!("cozydot {}", env!("CARGO_PKG_VERSION")),
        Some("init") => println!("Initialized cozydot in {}", init::run()?.display()),
        Some("apply") => apply()?,
        Some(other) => {
            print_help();
            bail!("unknown command: {other}");
        }
    }
    Ok(())
}

fn apply() -> Result<()> {
    let root = init::config_root()?;
    let path = root.join("cozydot.yaml");
    let cfg = Config::load(&path)
        .with_context(|| "active config is missing or invalid; run 'cozydot init' first")?;
    let platform = Platform::detect()?;
    let plan = planner::plan(&cfg, &platform, &root.join("dotfiles"))?;
    let steps = planner::lower_neutral::lower(&plan)?;
    let mut runner = ProcessRunner {
        dry_run: std::env::var_os("COZYDOT_DRY_RUN").is_some(),
    };
    execute(&mut runner, &steps)?;
    Ok(())
}

fn print_help() {
    println!("cozydot provisions a Linux system from one active configuration\n\nUsage: cozydot <COMMAND>\n\nCommands:\n  init     Create or safely refresh the config and bundled dotfiles\n  apply    Apply the active configuration to this host\n\nOptions:\n  -h, --help       Print help\n  -V, --version    Print version");
}
