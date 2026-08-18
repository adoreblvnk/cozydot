//! Define & run typed host operations.

mod appimage;
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
mod systemd;
mod users;
mod uv;
mod vscode;

pub use binary::{BinaryPackageOperation, BinarySourceOperation};
pub use desktop::{DesktopEnvironment, DesktopSetting};
pub use go::GoToolchainSelector;
pub use packages::fonts::NerdFontsMode;
pub use repo::AptRepo;

pub(crate) use host::{
    Host, TempPath, executable_file, path_program, regular_executable_file, require_regular_executable,
};
pub(super) use parsers::{gnome_shell_version, select_gnome_extension_version};

use crate::{config::AptUpgradeCommand, platform::Architecture};
use anyhow::Result;
use std::path::PathBuf;

/// Typed host operation handled by central executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    SudoGroupEnsure,
    DebianAptComponentsAdd,
    AptUpdate,
    UnattendedUpgradesSet { enabled: bool },
    SnapdSet { enabled: bool },
    AptPackagesInstall { packages: Vec<String> },
    AptPackagesPurge { packages: Vec<String> },
    FlatpakFlathubRemoteAdd,
    RustInstall { selector: String },
    FnmInstall,
    UvInstall,
    RustToolchainUpdate,
    GoToolchainInstall { selector: GoToolchainSelector, architecture: Architecture },
    GoToolchainUpdate { selector: GoToolchainSelector, architecture: Architecture },
    NodeVersionInstall { selector: String },
    PythonVersionInstall { selector: String },
    PythonVersionUpgrade,
    CargoBinstallInstall,
    CargoUpdateInstall,
    AptRepoAdd(Box<AptRepo>),
    FlatpakAppsInstall { refs: Vec<String> },
    CargoCratesInstall { crates: Vec<String> },
    CargoCratesUpdate,
    NpmPackagesInstall { packages: Vec<String> },
    NpmPackagesUpdate,
    AppimagedInstall { architecture: Architecture },
    BinaryPackageInstall(BinaryPackageOperation),
    NerdFontsInstall { families: Vec<String> },
    NerdFontsUpdate { families: Vec<String> },
    DotfilesApply { root: PathBuf, packages: Vec<String>, replace: bool },
    DockerGroupEnsure,
    DockerLocalLoggingDriverSet { max_size: Option<String> },
    VirtualBoxGroupEnsure,
    VsCodeExtensionsInstall { extensions: Vec<String> },
    DesktopSettingSet { environment: DesktopEnvironment, setting: DesktopSetting },
    GnomeExtensionsApply { extensions: Vec<String> },
    GnomeDashToDockInstall,
    GnomeRoundedWindowCornersInstall,
    AptUpgrade { command: AptUpgradeCommand },
    FlatpakUpdate,
    HomebrewInstall,
    HomebrewPackagesInstall { formulae: Vec<String>, casks: Vec<String> },
    MacOSSudoAccessValidate,
    CommandLineToolsForXcodeInstall,
    UserNerdFontsInstall { families: Vec<String> },
    UserNerdFontsUpdate { families: Vec<String> },
    MacDefaultsWrite { settings: Vec<macos::MacDefault> },
    HomebrewUpdateAndUpgrade { formulae: bool, casks: bool },
}

/// Whether change is active now or needs another login.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationOutcome {
    Completed,
    LoginRequired,
}

pub(crate) fn run(operation: &Operation) -> Result<OperationOutcome> {
    run_on(operation, Host::new()?)
}

fn run_on(operation: &Operation, host: Host) -> Result<OperationOutcome> {
    match operation {
        Operation::SudoGroupEnsure => completed(users::ensure_in_sudo_group(&host)),
        Operation::DebianAptComponentsAdd => completed(repo::debian_components::add(&host)),
        Operation::AptUpdate => completed(apt::update(&host)),
        Operation::UnattendedUpgradesSet { enabled } => completed(apt::set_unattended_upgrades(&host, *enabled)),
        Operation::SnapdSet { enabled } => completed(snapd::set_enabled(&host, *enabled)),
        Operation::AptPackagesInstall { packages } => completed(apt::install_packages(&host, packages)),
        Operation::AptPackagesPurge { packages } => completed(apt::purge(&host, packages)),
        Operation::FlatpakFlathubRemoteAdd => completed(packages::flatpak::add_flathub_remote(&host)),
        Operation::RustInstall { selector } => completed(rustup::install(&host, selector)),
        Operation::FnmInstall => completed(fnm::install(&host)),
        Operation::UvInstall => completed(uv::install(&host)),
        Operation::RustToolchainUpdate => completed(rustup::update_toolchains(&host)),
        Operation::GoToolchainInstall { selector, architecture } => {
            completed(go::install_toolchain(&host, selector, *architecture))
        }
        Operation::GoToolchainUpdate { selector, architecture } => {
            completed(go::update_toolchain(&host, selector, *architecture))
        }
        Operation::NodeVersionInstall { selector } => completed(fnm::install_version(&host, selector)),
        Operation::PythonVersionInstall { selector } => completed(uv::install_py(&host, selector)),
        Operation::PythonVersionUpgrade => completed(uv::upgrade_py_versions(&host)),
        Operation::CargoBinstallInstall => completed(binary::cargo_binstall::install(&host)),
        Operation::CargoUpdateInstall => completed(binary::cargo_binstall::install_cargo_update(&host)),
        Operation::AptRepoAdd(repo) => completed(repo::add(&host, repo)),
        Operation::FlatpakAppsInstall { refs } => completed(packages::flatpak::install(&host, refs)),
        Operation::CargoCratesInstall { crates } => completed(packages::cargo::install_crates(&host, crates)),
        Operation::CargoCratesUpdate => completed(packages::cargo::update_crates(&host)),
        Operation::NpmPackagesInstall { packages } => completed(packages::npm::install(&host, packages)),
        Operation::NpmPackagesUpdate => completed(packages::npm::update(&host)),
        Operation::AppimagedInstall { architecture } => completed(appimaged::install(&host, *architecture)),
        Operation::BinaryPackageInstall(package) => completed(binary::install(&host, package)),
        Operation::NerdFontsInstall { families } => {
            completed(packages::fonts::apply(&host, families, NerdFontsMode::Install))
        }
        Operation::NerdFontsUpdate { families } => {
            completed(packages::fonts::apply(&host, families, NerdFontsMode::Update))
        }
        Operation::DotfilesApply { root, packages, replace } => {
            completed(packages::dotfiles::apply(&host, root, packages, *replace))
        }
        Operation::DockerGroupEnsure => completed(users::ensure_in_group(&host, "Docker", "docker", "docker")),
        Operation::DockerLocalLoggingDriverSet { max_size } => {
            completed(docker::set_local_logging_driver(&host, max_size.as_deref()))
        }
        Operation::VirtualBoxGroupEnsure => {
            completed(users::ensure_in_group(&host, "VirtualBox", "VBoxManage", "vboxusers"))
        }
        Operation::VsCodeExtensionsInstall { extensions } => completed(vscode::install_extensions(&host, extensions)),
        Operation::DesktopSettingSet { environment, setting } => completed(desktop::set(&host, *environment, setting)),
        Operation::GnomeExtensionsApply { extensions } => gnome::apply_extensions(&host, extensions),
        Operation::GnomeDashToDockInstall => gnome::install_dash_to_dock(&host),
        Operation::GnomeRoundedWindowCornersInstall => gnome::install_rounded_window_corners(&host),
        Operation::AptUpgrade { command } => completed(apt::upgrade(&host, *command)),
        Operation::FlatpakUpdate => completed(packages::flatpak::update(&host)),
        Operation::HomebrewInstall => completed(macos::install_homebrew(&host)),
        Operation::HomebrewPackagesInstall { formulae, casks } => {
            completed(macos::install_packages(&host, formulae, casks))
        }
        Operation::MacOSSudoAccessValidate => completed(macos::validate_sudo_access(&host)),
        Operation::CommandLineToolsForXcodeInstall => completed(macos::install_command_line_tools_for_xcode(&host)),
        Operation::UserNerdFontsInstall { families } => {
            completed(packages::fonts::apply_user(&host, families, NerdFontsMode::Install))
        }
        Operation::UserNerdFontsUpdate { families } => {
            completed(packages::fonts::apply_user(&host, families, NerdFontsMode::Update))
        }
        Operation::MacDefaultsWrite { settings } => completed(macos::write_defaults(&host, settings)),
        Operation::HomebrewUpdateAndUpgrade { formulae, casks } => {
            completed(macos::update_and_upgrade(&host, *formulae, *casks))
        }
    }
}

fn completed(result: Result<()>) -> Result<OperationOutcome> {
    result.map(|()| OperationOutcome::Completed)
}
