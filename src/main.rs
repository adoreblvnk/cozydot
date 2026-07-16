use anyhow::{bail, Result};
use cozydot::{
    apply,
    init::{self, Preset},
};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    match command.as_deref() {
        Some("-h" | "--help") | None if args.next().is_none() => print_help(),
        Some("-V" | "--version") if args.next().is_none() => {
            println!("cozydot {}", env!("CARGO_PKG_VERSION"));
        }
        Some("init") => {
            run_init(args)?;
        }
        Some("apply") if args.next().is_none() => apply()?,
        Some(other) => {
            print_help();
            bail!("unknown command: {other}");
        }
        _ => {
            print_help();
            bail!("unexpected arguments");
        }
    }
    Ok(())
}

fn run_init(args: impl Iterator<Item = String>) -> Result<()> {
    let args = args.collect::<Vec<_>>();
    if matches!(args.as_slice(), [option] if option == "-h" || option == "--help") {
        print_init_help();
        return Ok(());
    }
    let preset = match args.as_slice() {
        [] => Preset::default(),
        [option, value] if option == "--preset" => Preset::parse(value)?,
        [option] if option.starts_with("--preset=") => {
            Preset::parse(option.trim_start_matches("--preset="))?
        }
        _ => {
            print_help();
            bail!("usage: cozydot init [--preset <PRESET>]");
        }
    };
    println!("Initialized cozydot in {}", init::run(preset)?.display());
    Ok(())
}

fn print_init_help() {
    println!("Create or safely refresh the config and bundled dotfiles\n\nUsage: cozydot init [--preset <PRESET>]\n\nPresets:\n  cozydot, full, cli, vm\n\nOptions:\n  --preset <PRESET>  Select the configuration to materialize (default: cozydot)\n  -h, --help         Print help");
}

fn print_help() {
    println!("cozydot provisions a Linux system from one active configuration\n\nUsage: cozydot <COMMAND>\n\nCommands:\n  init [--preset <PRESET>]  Create or safely refresh the config and bundled dotfiles\n  apply                     Apply the active configuration to this host\n\nPresets:\n  cozydot, full, cli, vm\n\nOptions:\n  -h, --help       Print help\n  -V, --version    Print version");
}
