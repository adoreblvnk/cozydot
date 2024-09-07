```
TIDES (The Idempotent Dev Environment Setup) is an automated post-install, update, & config (dotfile) manager for Linux

Usage: tides [Options] [Command]

Options:
  -n, --no-colour        Do not output any colour. Useful when redirecting output to a logfile
                         directory
  -c, --config <CONFIG>  Set a config file that exists in $CONFIG_DIR (default: <tides_path>/configs/)
  -S, --secrets <SECRET> Set a secrets file. Add to .gitignore if secrets file is in tides directory
  -s, --skip-slow        Allow tides to skip slow running actions. Will break your system if set on
                         tides' first run
      --list-configs     List all available configs in $CONFIG_DIR (default: <tides_path>/configs/)
                         directory
  -h, --help             Print help information
  -V, --version          Print version information

Commands:

     check     Purges bloat (default) & installs dependencies. Installs Python (via pyenv), Cargo
               (via rustup), appimaged, & fonts (Geist & Nerdfonts)
  i, install   Installs all apt (& alternative sources), flatpak, cargo, binary (AppImage)
               packages, & coding languages (node & golang)
  u, update    Updates & upgrades apt, flatpak, cargo packages. Updates other packages & cleans
               system (see configs/default.yaml for details)
  c, configure Configures apps installed, restore / backup dotfiles via Stow (dotfile manager),
               customise desktop environment (Cinnamon / GNOME), & performs other system configs
  h, help      Print help information

Configuration:
  Customise the actions each command by modifying the configs/default.yaml file.
  The full config schema of tides is available at README.md.
  Preset configs are available in <tides_path>/configs/ directory. Add new configs in $CONFIG_DIR
   (default: <tides_path>/configs/) or list them with tides --list-configs

Example:
  tides --config virtual_machine configure

Project Homepage: https://github.com/adoreblvnk/tides
```
