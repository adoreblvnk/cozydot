//! Typed host mutations and their centralized execution boundary.

mod appimaged;
mod apt;
mod binary;
mod desktop;
mod docker;
mod fnm;
mod gnome;
mod go;
mod host;
pub(crate) mod macos;
mod packages;
mod parsers;
pub(crate) mod privileged_file;
mod repo;
mod rustup;
mod shell;
mod snapd;
mod users;
mod uv;
mod virtualbox;
mod vscode;

pub use binary::{BinaryPackageOperation, BinarySourceOperation};
pub use desktop::{DesktopEnvironment, DesktopSetting};
pub use go::GoToolchainSelector;
pub use packages::fonts::NerdFontsMode;
pub use repo::AptRepo;

pub(crate) use host::{Host, TempPath, executable_file, path_program, real_executable_file, required_real_executable};
pub(super) use parsers::{gnome_shell_version, select_gnome_extension_version};

use crate::{config::AptUpgradeCommand, platform::Architecture};
use anyhow::{Result, bail};
use std::path::PathBuf;

/// A typed host operation accepted by the centralized executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    SudoGroup,
    AddDebianAptComponents { release: String },
    AptUpdate,
    UnattendedUpgrades { enabled: bool },
    Snapd { enabled: bool },
    AptPackages { packages: Vec<String> },
    AptUpdateAndInstall { packages: Vec<String> },
    FlatpakAddFlathubRemote,
    InstallRustup,
    InstallFnm,
    InstallUv,
    RustToolchain { selector: String },
    RustToolchainUpdate,
    GoToolchain { selector: GoToolchainSelector, architecture: Architecture },
    GoToolchainUpdate { selector: GoToolchainSelector, architecture: Architecture },
    NodeToolchain { selector: String },
    NodeToolchainUpdate { selector: String },
    PythonToolchain { version: String },
    PythonToolchainUpdate,
    InstallCargoBinstall,
    InstallCargoUpdate,
    AptRepo(Box<AptRepo>),
    AptPurgeThenInstall { purge: Vec<String>, install: Vec<String> },
    FlatpakInstall { refs: Vec<String> },
    CargoInstall { packages: Vec<String> },
    CargoPackageUpdate,
    NpmInstall { packages: Vec<String> },
    NpmPackageUpdate,
    Appimaged { architecture: Architecture },
    BinaryPackage(BinaryPackageOperation),
    NerdFonts { families: Vec<String>, mode: NerdFontsMode },
    Dotfiles { root: PathBuf, packages: Vec<String>, replace: bool },
    DockerGroup,
    DockerLocalLoggingDriver { max_size: Option<String> },
    VirtualBoxGroup,
    VsCodeInstallExtensions { extensions: Vec<String> },
    DesktopSetting { target: DesktopEnvironment, setting: DesktopSetting },
    GnomeExtensions { extensions: Vec<String> },
    InstallDashToDock,
    InstallRoundedWindowCorners,
    AptUpgrade { command: AptUpgradeCommand },
    FlatpakUpdateApps,
    InstallHomebrew,
    InstallHomebrewPackages { formulae: Vec<String>, casks: Vec<String> },
    ValidateMacosSudoAccess,
    InstallCommandLineToolsForXcode,
    UserNerdFonts { families: Vec<String>, mode: packages::fonts::NerdFontsMode },
    MacDefaults { settings: Vec<macos::MacDefault> },
    HomebrewUpdate { formulae: bool, casks: bool },
}

/// Reports whether the requested state is active or requires another login to take effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationOutcome {
    Completed,
    LoginRequired,
}

impl Operation {
    /// Supplies the user-facing label used for progress and error context.
    pub fn label(&self) -> &'static str {
        match self {
            Self::SudoGroup => "sudo group membership",
            Self::AddDebianAptComponents { .. } => "Debian APT components",
            Self::AptUpdate => "APT update",
            Self::UnattendedUpgrades { .. } => "unattended upgrades",
            Self::Snapd { .. } => "snapd",
            Self::AptPackages { .. } => "APT packages",
            Self::AptUpdateAndInstall { .. } => "APT update and install",
            Self::FlatpakAddFlathubRemote => "Flathub remote add",
            Self::InstallRustup => "rustup install",
            Self::InstallFnm => "FNM install",
            Self::InstallUv => "uv install",
            Self::RustToolchain { .. } => "Rust toolchain",
            Self::RustToolchainUpdate => "Rust toolchain updates",
            Self::GoToolchain { .. } => "Go toolchain",
            Self::GoToolchainUpdate { .. } => "Go toolchain updates",
            Self::NodeToolchain { .. } => "Node.js toolchain",
            Self::NodeToolchainUpdate { .. } => "Node.js toolchain updates",
            Self::PythonToolchain { .. } => "Python toolchain",
            Self::PythonToolchainUpdate => "Python toolchain updates",
            Self::InstallCargoBinstall => "cargo-binstall install",
            Self::InstallCargoUpdate => "cargo-update install",
            Self::AptRepo(_) => "APT repo",
            Self::AptPurgeThenInstall { .. } => "APT purge and install",
            Self::FlatpakInstall { .. } => "Flatpak application install",
            Self::CargoInstall { .. } => "Cargo package install",
            Self::CargoPackageUpdate => "Cargo package updates",
            Self::NpmInstall { .. } => "npm package install",
            Self::NpmPackageUpdate => "npm package updates",
            Self::Appimaged { .. } => "appimaged",
            Self::BinaryPackage(_) => "binary package",
            Self::NerdFonts { .. } => "Nerd Fonts",
            Self::Dotfiles { .. } => "dotfiles",
            Self::DockerGroup => "Docker group membership",
            Self::DockerLocalLoggingDriver { .. } => "Docker local logging driver",
            Self::VirtualBoxGroup => "VirtualBox group membership",
            Self::VsCodeInstallExtensions { .. } => "Visual Studio Code extension install",
            Self::DesktopSetting { .. } => "desktop setting",
            Self::GnomeExtensions { .. } => "GNOME extensions",
            Self::InstallDashToDock => "Dash to Dock install",
            Self::InstallRoundedWindowCorners => "Rounded Window Corners install",
            Self::AptUpgrade { .. } => "APT upgrade",
            Self::FlatpakUpdateApps => "Flatpak application updates",
            Self::InstallHomebrew => "Homebrew install",
            Self::InstallHomebrewPackages { .. } => "Homebrew package install",
            Self::ValidateMacosSudoAccess => "macOS sudo access",
            Self::InstallCommandLineToolsForXcode => "Command Line Tools for Xcode install",
            Self::UserNerdFonts { .. } => "user Nerd Fonts",
            Self::MacDefaults { .. } => "macOS defaults",
            Self::HomebrewUpdate { .. } => "Homebrew updates",
        }
    }
}

pub(crate) fn run(operation: &Operation) -> Result<OperationOutcome> {
    run_on(operation, Host::new()?)
}

fn run_on(operation: &Operation, host: Host) -> Result<OperationOutcome> {
    if matches!(
        operation,
        Operation::InstallHomebrew
            | Operation::InstallHomebrewPackages { .. }
            | Operation::ValidateMacosSudoAccess
            | Operation::InstallCommandLineToolsForXcode
            | Operation::UserNerdFonts { .. }
            | Operation::MacDefaults { .. }
            | Operation::HomebrewUpdate { .. }
    ) && !cfg!(target_os = "macos")
    {
        bail!("macOS operation cannot run on this host")
    }
    match operation {
        Operation::SudoGroup => completed(users::sudo_group(&host)),
        Operation::AddDebianAptComponents { release } => completed(repo::debian_components::add(&host, release)),
        Operation::AptUpdate => completed(apt::update(&host)),
        Operation::UnattendedUpgrades { enabled } => completed(apt::unattended_upgrades(&host, *enabled)),
        Operation::Snapd { enabled } => completed(snapd::set_snapd_enabled(&host, *enabled)),
        Operation::AptPackages { packages } => completed(apt::packages(&host, packages)),
        Operation::AptUpdateAndInstall { packages } => completed(apt::update_and_install(&host, packages)),
        Operation::FlatpakAddFlathubRemote => completed(packages::flatpak::add_flathub_remote(&host)),
        Operation::InstallRustup => completed(rustup::install_rustup(&host)),
        Operation::InstallFnm => completed(fnm::install_fnm(&host)),
        Operation::InstallUv => completed(uv::install_uv(&host)),
        Operation::RustToolchain { selector } => completed(rustup::install_default_rust_toolchain(&host, selector)),
        Operation::RustToolchainUpdate => completed(rustup::update_rust(&host)),
        Operation::GoToolchain { selector, architecture } => completed(go::install_go(&host, selector, *architecture)),
        Operation::GoToolchainUpdate { selector, architecture } => {
            completed(go::update_go(&host, selector, *architecture))
        }
        Operation::NodeToolchain { selector } => completed(fnm::install_default_node_toolchain(&host, selector)),
        Operation::NodeToolchainUpdate { selector } => completed(fnm::install_default_node_toolchain(&host, selector)),
        Operation::PythonToolchain { version } => completed(uv::install_default_python(&host, version)),
        Operation::PythonToolchainUpdate => completed(uv::update_python(&host)),
        Operation::InstallCargoBinstall => completed(binary::cargo_binstall::install(&host)),
        Operation::InstallCargoUpdate => completed(binary::cargo_binstall::install_cargo_update(&host)),
        Operation::AptRepo(repo) => completed(repo::add(&host, repo)),
        Operation::AptPurgeThenInstall { purge, install } => completed(apt::purge_then_install(&host, purge, install)),
        Operation::FlatpakInstall { refs } => completed(packages::flatpak::install(&host, refs)),
        Operation::CargoInstall { packages } => completed(packages::cargo::install(&host, packages)),
        Operation::CargoPackageUpdate => completed(packages::cargo::update_all(&host)),
        Operation::NpmInstall { packages } => completed(packages::npm::install(&host, packages)),
        Operation::NpmPackageUpdate => completed(packages::npm::update_all(&host)),
        Operation::Appimaged { architecture } => completed(appimaged::install(&host, *architecture)),
        Operation::BinaryPackage(package) => completed(binary::install(&host, package)),
        Operation::NerdFonts { families, mode } => completed(packages::fonts::apply(&host, families, *mode)),
        Operation::Dotfiles { root, packages, replace } => {
            completed(packages::dotfiles::apply(&host, root, packages, *replace))
        }
        Operation::DockerGroup => completed(docker::docker_group(&host)),
        Operation::DockerLocalLoggingDriver { max_size } => {
            completed(docker::set_docker_local_logging_driver(&host, max_size.as_deref()))
        }
        Operation::VirtualBoxGroup => completed(virtualbox::virtualbox_group(&host)),
        Operation::VsCodeInstallExtensions { extensions } => {
            completed(vscode::install_vscode_extensions(&host, extensions))
        }
        Operation::DesktopSetting { target, setting } => completed(desktop::desktop_setting(&host, *target, setting)),
        Operation::GnomeExtensions { extensions } => gnome::gnome_extensions(&host, extensions),
        Operation::InstallDashToDock => gnome::install_dash_to_dock(&host),
        Operation::InstallRoundedWindowCorners => gnome::install_rounded_window_corners(&host),
        Operation::AptUpgrade { command } => completed(apt::upgrade(&host, *command)),
        Operation::FlatpakUpdateApps => completed(packages::flatpak::update_apps(&host)),
        Operation::InstallHomebrew => completed(macos::install_homebrew(&host)),
        Operation::InstallHomebrewPackages { formulae, casks } => {
            completed(macos::install_packages(&host, formulae, casks))
        }
        Operation::ValidateMacosSudoAccess => completed(macos::validate_sudo_access(&host)),
        Operation::InstallCommandLineToolsForXcode => completed(macos::install_command_line_tools_for_xcode(&host)),
        Operation::UserNerdFonts { families, mode } => completed(packages::fonts::apply_user(&host, families, *mode)),
        Operation::MacDefaults { settings } => completed(macos::write_defaults(&host, settings)),
        Operation::HomebrewUpdate { formulae, casks } => completed(macos::update(&host, *formulae, *casks)),
    }
}

fn completed(result: Result<()>) -> Result<OperationOutcome> {
    result.map(|()| OperationOutcome::Completed)
}
