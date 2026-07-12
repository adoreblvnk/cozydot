use anyhow::{bail, Context, Result};
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

fn main() -> Result<()> {
    if let Ok(exe) = std::env::current_exe() {
        std::env::set_var("COZYDOT_EXE", exe);
    }
    let root = installation_root();
    let mut config = "default".to_owned();
    let mut runner = ProcessRunner {
        dry_run: std::env::var_os("COZYDOT_DRY_RUN").is_some(),
    };
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_none() {
        print_usage();
        bail!("No argument(s)");
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            "-V" | "--version" => {
                println!("cozydot {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-n" | "--no-color" => {}
            "-c" | "--config" => {
                config = args
                    .next()
                    .filter(|s| !s.starts_with('-'))
                    .context("no value supplied to --config")?;
                validate_config_name(&config)?;
                let path = config_path(&root, &config);
                if !path.is_file() {
                    bail!("Config selected {config} does not exist");
                }
            }
            "--list-configs" => list_configs(&root)?,
            "check" => {
                run_command("check", &root, &config, &mut runner)?;
            }
            "i" | "install" => {
                run_command("install", &root, &config, &mut runner)?;
            }
            "u" | "update" => {
                run_command("update", &root, &config, &mut runner)?;
            }
            "c" | "configure" => {
                run_command("configure", &root, &config, &mut runner)?;
            }
            _ => {
                print_usage();
                bail!("Invalid argument(s)");
            }
        }
    }
    Ok(())
}

fn run_command(
    name: &str,
    root: &Path,
    config_name: &str,
    runner: &mut ProcessRunner,
) -> Result<()> {
    let path = config_path(root, config_name);
    let cfg = Config::load(&path)?;
    let p = Platform::detect(
        cfg.string("metadata.distro").as_deref().unwrap_or("auto"),
        cfg.string("metadata.DE").as_deref().unwrap_or("auto"),
    )?;
    let steps = planner::plan(name, &cfg, &p, root)?;
    execute(runner, &steps)?;
    let ran_check = name == "check"
        || (name == "install" && cfg.bool("install.check"))
        || (name == "update" && cfg.bool("update.check"))
        || (name == "configure" && cfg.bool("configure.check"));
    if ran_check && cfg.tagged_enabled("check.purge") && !runner.dry_run {
        Config::disable_purge(&path)?;
    }
    println!("Finished cozydot {name}");
    Ok(())
}

fn print_usage() {
    println!(
        "cozydot is an automated post-install, update, and config manager for Linux\n\n\
Usage: cozydot [Options] [Command]\n\n\
Options:\n  -n, --no-color\n  -c, --config <CONFIG>\n      --list-configs\n  -h, --help\n  -V, --version\n\n\
Commands:\n  check\n  i, install\n  u, update\n  c, configure"
    );
}

fn installation_root() -> PathBuf {
    std::env::var_os("COZYDOT_ROOT")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .filter(|p| p.join("configs").is_dir() && p.join("dotfiles").is_dir())
        })
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn config_path(root: &Path, name: &str) -> PathBuf {
    root.join("configs").join(format!("{name}.yaml"))
}

fn validate_config_name(name: &str) -> Result<()> {
    if name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && !name.is_empty()
    {
        Ok(())
    } else {
        bail!("config must be a preset name under configs/")
    }
}

fn list_configs(root: &Path) -> Result<()> {
    println!("Available configs:");
    let mut entries = fs::read_dir(root.join("configs"))?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if e.path().extension().and_then(|x| x.to_str()) == Some("yaml") {
            let c = Config::load(&e.path())?;
            let stem = e
                .path()
                .file_stem()
                .context("config file has no file stem")?
                .to_string_lossy()
                .into_owned();
            println!(
                "  {}: {}",
                stem,
                c.string("metadata.description").unwrap_or_default()
            );
        }
    }
    Ok(())
}
