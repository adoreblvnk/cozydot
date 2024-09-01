```
TIDES (The Idempotent Dev Environment Setup) is a post-install, update, config, & dotfile manager script f
or Linux systems

Usage: tides [Options] [Command]

Options:
      --no-colour        Do not output any colour. Useful when redirecting output to a logfile
      --list-configs     List all available configs in $CONFIG_DIR (default: <tides_path>/configs/)
                         directory
  -c, --config <CONFIG>  Set a config file that exists in $CONFIG_DIR (default: <tides_path>/configs/)
                         directory
      --secrets <SECRET> Set a secrets file. Add to .gitignore if secrets file is in tides directory
  -h, --help             Print help information
  -V, --version          Print version information

Commands:

     check     Purges bloat (default) & installs dependencies. Installs Python (via pyenv), Cargo
               (via rustup), fonts (Geist & Nerdfonts), & firmware packages
  i, install   Installs all apt (& alternative sources), flatpak, cargo, & binary (AppImage)
               packages
  u, update    Updates & upgrades apt, flatpak, cargo packages. Updates other packages & cleans
               system (see `config.yaml` for details)
  c, configure Configures apps installed, restore / backup dotfiles via Stow (dotfile manager), &
               performs other system configs.
  h, help      Print help information

Configuration:
  Customise the actions each command by modifying the configs/default.yaml file.
  The full config schema of tides is available at README.md.
  Preset configs are available in <tides_path>/configs/ directory. Add new configs in $CONFIG_DIR
   (default: <tides_path>/configs/) & list them with tides --list-configs

Example:
  tides --config virtual_machine configure

Project Homepage: https://github.com/adoreblvnk/tides
```
