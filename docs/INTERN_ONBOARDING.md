# Intern Onboarding

Cozydot is a Rust CLI that provisions Debian-family Linux and Apple Silicon macOS from one typed YAML configuration. This guide describes the runtime architecture and command order.

## Runtime architecture

Host-changing commands load the active host before entering a workflow:

1. Resolve the configuration root at `${XDG_CONFIG_HOME:-$HOME/.config}/cozydot`.
2. Load and validate `cozydot.yaml`.
3. Detect and normalize the platform.
4. Validate the configuration for that platform.
5. Enter the Linux or macOS workflow.
6. Construct and run each typed `Operation` in dependency order.
7. Stop on the first failure.

The implementation is divided by responsibility:

```text
src/main.rs            CLI entry points and active-host loading
src/init.rs            presets, bundled files, and managed hashes
src/config.rs          typed YAML and validation
src/platform.rs        platform detection and normalization
src/workflow/mod.rs    Linux and macOS command order
src/operations/mod.rs  typed operations and dispatch
src/operations/        live-state checks and host changes
```

Workflows keep execution order visible in `src/workflow/mod.rs`. Executors inspect live state and perform the concrete host changes.

## Init

`cozydot init` does not load or detect the active host.

1. Resolve and create the configuration root without accepting symlinked managed paths.
2. Read `.managed-files` when it exists.
3. Select the embedded `cozydot`, `cli`, or `vm` preset.
4. Write `cozydot.yaml` when it is missing or still matches its managed hash.
5. Group bundled dotfiles by Stow package.
6. Synchronize a bundled package only when its complete managed contents are unchanged.
7. Write files atomically and preserve user-edited or unmanaged files.
8. Write `.managed-files` after all selected files are synchronized.

## Apply

`cozydot apply` ensures configured state without running enabled update policies. Missing configuration is skipped.

### Linux

1. Derive applicable repos, aggregate repo package changes, APT requirements, tool installations, binary mappings, and desktop prerequisites.
2. On Debian, ensure configured `sudo` group membership and add official APT components.
3. On Ubuntu, set unattended upgrades and snapd state, then install restricted extras when configured.
4. Run the early APT update when required by configured APT, Ubuntu, or Deb binary state.
5. Install missing base and derived prerequisites, running APT update immediately before install.
6. Install configured direct APT packages.
7. Add each applicable APT repo, then run one APT update.
8. Purge aggregate repo conflicts and install aggregate repo packages.
9. Add the Flathub remote and install configured Flatpak applications.
10. Install rustup, the Rust toolchain, cargo-binstall, cargo-update, FNM, the configured Node.js version, uv, the configured Python version, and the Go toolchain in that order when required.
11. Install configured Cargo crates and npm packages.
12. Install applicable Deb binaries.
13. Install appimaged, then applicable AppImages.
14. Install configured Nerd Font families.
15. Apply shared and Linux dotfile packages.
16. Apply Docker, VirtualBox, and Visual Studio Code integrations.
17. Set configured Linux desktop settings.

### macOS

1. Derive required tool installations and whether dotfiles require Stow.
2. Validate sudo access when configured.
3. Install Command Line Tools for Xcode when configured.
4. Install Homebrew.
5. Install configured formulae and casks, adding `stow` when dotfiles are configured.
6. Install rustup, the Rust toolchain, cargo-binstall, cargo-update, FNM, the configured Node.js version, uv, the configured Python version, and the Go toolchain in that order when required.
7. Install configured Cargo crates and npm packages.
8. Install configured user Nerd Font families.
9. Apply shared and macOS dotfile packages.
10. Install configured Visual Studio Code extensions.
11. Write configured macOS defaults.

## Dotfiles

`cozydot dotfiles` applies shared packages plus packages for the detected platform. Linux selects `linux.dotfiles.packages`; macOS selects `macos.dotfiles.packages`.

1. Combine shared and platform dotfile packages in declaration order.
2. Stop without an operation when no packages are configured.
3. Verify the dotfiles root and every selected package directory.
4. Resolve intended destinations and collect all unmanaged conflicts before mutation.
5. Report every conflict and change nothing when `--replace` is absent.
6. Verify GNU Stow is available.
7. With `--replace`, move conflicts under `${XDG_STATE_HOME:-$HOME/.local/state}/cozydot/dotfile-backups`.
8. Apply each package with Stow in declaration order.

`cozydot apply` uses the same dotfiles operation without replacement.

## Update

`cozydot update` runs only enabled update controls. It does not replay apply intent.

### Linux

1. Derive base prerequisites and add Flatpak when its update is enabled.
2. When an APT policy is configured, run APT update followed by the selected `upgrade` or `full-upgrade` command.
3. Install missing update prerequisites, running APT update immediately before install.
4. Update installed Flatpak applications when enabled.
5. Install rustup and update Rust toolchains when enabled.
6. Update the Go toolchain when enabled.
7. Install FNM and update the Node.js version when enabled.
8. Install uv and upgrade the Python versions when enabled.
9. Update installed Cargo crates when enabled.
10. Update global npm packages when enabled.
11. Update configured Nerd Font families when enabled.

### macOS

1. Install Homebrew.
2. Run Homebrew update and upgrade the selected formulae and casks when enabled.
3. Install rustup and update Rust toolchains when enabled.
4. Update the Go toolchain when enabled.
5. Install FNM for a Node.js version or npm update, then update the Node.js version when enabled.
6. Install uv and upgrade the Python versions when enabled.
7. Update installed Cargo crates when enabled.
8. Update global npm packages when enabled.
9. Update configured user Nerd Font families when enabled.
