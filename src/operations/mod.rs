mod appimaged;
mod apt;
mod binary;
mod host;
mod languages;
pub(crate) mod macos;
mod packages;
mod parsers;
pub(crate) mod privileged_file;

mod repository;
mod system;
mod tools;

pub use apt::AptUpgradePolicy;
pub use binary::{BinaryPackageOperation, BinarySourceOperation};
pub use packages::fonts::NerdFontsMode;
pub use repository::AptRepositoryOperation;
pub use system::{DesktopEnvironment, DesktopSetting, DesktopTheme};
pub use tools::GoToolchainSelector;

pub(crate) use host::{Host, TempPath, executable_file, path_program, real_executable_file};
pub(super) use parsers::{gnome_shell_version, gnome_version, latest_go};

use crate::platform::Architecture;
use anyhow::{Result, bail};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolchainMode {
    EnsurePresent,
    ConvergeLatest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    EnsureAdmin,
    EnsureDebianAptComponents { release: String },
    AptMetadataRefresh,
    UnattendedUpgrades { enabled: bool },
    UbuntuSnap { enabled: bool },
    AptPackages { packages: Vec<String> },
    AptBootstrapPackages { packages: Vec<String> },
    FlatpakEnsureFlathub,
    RustupBootstrap,
    FnmBootstrap,
    UvBootstrap,
    RustToolchain { selector: Option<String>, mode: ToolchainMode },
    GoToolchain { selector: GoToolchainSelector, architecture: Architecture, mode: ToolchainMode },
    NodeToolchain { selector: String, mode: ToolchainMode },
    PythonToolchain { version: String, mode: ToolchainMode },
    CargoBinstallBootstrap,
    AptRepository(Box<AptRepositoryOperation>),
    AptRepositoryPackages { conflicts: Vec<String>, packages: Vec<String> },
    FlatpakEnsureApps { refs: Vec<String> },
    CargoPackageSet { packages: Vec<String> },
    CargoPackageUpdate,
    NpmPackageSet { packages: Vec<String> },
    NpmPackageUpdate,
    Appimaged { architecture: Architecture },
    BinaryPackage(BinaryPackageOperation),
    NerdFonts { families: Vec<String>, mode: NerdFontsMode },
    Dotfiles { root: PathBuf, packages: Vec<String>, replace: bool },
    DockerGroup,
    DockerLocalLog { max_size: Option<String> },
    VirtualBoxGroup,
    VsCodeExtensionSet { extensions: Vec<String> },
    DesktopSetting { target: DesktopEnvironment, setting: DesktopSetting },
    GnomeExtensions { extensions: Vec<String> },
    GnomeDock,
    GnomeRoundedCorners,
    AptUpgrade { policy: AptUpgradePolicy },
    FlatpakUpdateApps,
    HomebrewBootstrap,
    HomebrewPackages { formulae: Vec<String>, casks: Vec<String> },
    MacEnsureAdmin,
    XcodeCommandLineTools,
    Rosetta,
    UserNerdFonts { families: Vec<String>, mode: packages::fonts::NerdFontsMode },
    MacDefaults { settings: Vec<macos::MacDefault> },
    HomebrewUpdate { formulae: bool, casks: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationOutcome {
    Completed,
    LoginRequired,
}

impl Operation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::EnsureAdmin => "administrator access",
            Self::EnsureDebianAptComponents { .. } => "Debian APT components",
            Self::AptMetadataRefresh => "APT metadata refresh",
            Self::UnattendedUpgrades { .. } => "unattended upgrades",
            Self::UbuntuSnap { .. } => "Ubuntu Snap",
            Self::AptPackages { .. } => "APT packages",
            Self::AptBootstrapPackages { .. } => "APT bootstrap packages",
            Self::FlatpakEnsureFlathub => "Flathub remote",
            Self::RustupBootstrap => "rustup bootstrap",
            Self::FnmBootstrap => "FNM bootstrap",
            Self::UvBootstrap => "uv bootstrap",
            Self::RustToolchain { .. } => "Rust toolchain",
            Self::GoToolchain { .. } => "Go toolchain",
            Self::NodeToolchain { .. } => "Node.js toolchain",
            Self::PythonToolchain { .. } => "Python toolchain",
            Self::CargoBinstallBootstrap => "cargo-binstall bootstrap",
            Self::AptRepository(_) => "APT repository",
            Self::AptRepositoryPackages { .. } => "APT repository packages",
            Self::FlatpakEnsureApps { .. } => "Flatpak applications",
            Self::CargoPackageSet { .. } => "Cargo packages",
            Self::CargoPackageUpdate => "Cargo package updates",
            Self::NpmPackageSet { .. } => "npm packages",
            Self::NpmPackageUpdate => "npm package updates",
            Self::Appimaged { .. } => "appimaged",
            Self::BinaryPackage(_) => "binary package",
            Self::NerdFonts { .. } => "Nerd Fonts",
            Self::Dotfiles { .. } => "dotfiles",
            Self::DockerGroup => "Docker group membership",
            Self::DockerLocalLog { .. } => "Docker logging",
            Self::VirtualBoxGroup => "VirtualBox group membership",
            Self::VsCodeExtensionSet { .. } => "Visual Studio Code extensions",
            Self::DesktopSetting { .. } => "desktop setting",
            Self::GnomeExtensions { .. } => "GNOME extensions",
            Self::GnomeDock => "GNOME dock",
            Self::GnomeRoundedCorners => "GNOME rounded corners",
            Self::AptUpgrade { .. } => "APT upgrade",
            Self::FlatpakUpdateApps => "Flatpak application updates",
            Self::HomebrewBootstrap => "Homebrew bootstrap",
            Self::HomebrewPackages { .. } => "Homebrew packages",
            Self::MacEnsureAdmin => "macOS administrator access",
            Self::XcodeCommandLineTools => "Xcode command line tools",
            Self::Rosetta => "Rosetta",
            Self::UserNerdFonts { .. } => "user Nerd Fonts",
            Self::MacDefaults { .. } => "macOS defaults",
            Self::HomebrewUpdate { .. } => "Homebrew updates",
        }
    }
}

pub(crate) fn execute(operation: &Operation) -> Result<OperationOutcome> {
    execute_on_host(operation, Host::new()?)
}

fn execute_on_host(operation: &Operation, host: Host) -> Result<OperationOutcome> {
    if matches!(
        operation,
        Operation::HomebrewBootstrap
            | Operation::HomebrewPackages { .. }
            | Operation::MacEnsureAdmin
            | Operation::XcodeCommandLineTools
            | Operation::Rosetta
            | Operation::UserNerdFonts { .. }
            | Operation::MacDefaults { .. }
            | Operation::HomebrewUpdate { .. }
    ) && !cfg!(target_os = "macos")
    {
        bail!("macOS operation cannot execute on this host")
    }
    match operation {
        Operation::EnsureAdmin => completed(system::ensure_admin(&host)),
        Operation::EnsureDebianAptComponents { release } => {
            completed(repository::debian_components::execute(&host, release))
        }
        Operation::AptMetadataRefresh => completed(apt::metadata_refresh(&host)),
        Operation::UnattendedUpgrades { enabled } => completed(system::unattended_upgrades(&host, *enabled)),
        Operation::UbuntuSnap { enabled } => completed(system::ubuntu_snap(&host, *enabled)),
        Operation::AptPackages { packages } => completed(apt::packages(&host, packages)),
        Operation::AptBootstrapPackages { packages } => completed(apt::bootstrap_packages(&host, packages)),
        Operation::FlatpakEnsureFlathub => completed(packages::flatpak::ensure_flathub(&host)),
        Operation::RustupBootstrap => completed(languages::rustup(&host)),
        Operation::FnmBootstrap => completed(languages::fnm_bootstrap(&host)),
        Operation::UvBootstrap => completed(languages::uv_bootstrap(&host)),
        Operation::RustToolchain { selector, mode } => {
            completed(tools::execute_rust(&host, selector.as_deref(), *mode))
        }
        Operation::GoToolchain { selector, architecture, mode } => {
            completed(tools::execute_go(&host, selector, *architecture, *mode))
        }
        Operation::NodeToolchain { selector, mode } => completed(tools::execute_node(&host, selector, *mode)),
        Operation::PythonToolchain { version, mode } => completed(tools::execute_python(&host, version, *mode)),
        Operation::CargoBinstallBootstrap => completed(binary::cargo_binstall::execute(&host)),
        Operation::AptRepository(operation) => completed(repository::execute(&host, operation)),
        Operation::AptRepositoryPackages { conflicts, packages } => {
            completed(apt::repository_packages(&host, conflicts, packages))
        }
        Operation::FlatpakEnsureApps { refs } => completed(packages::flatpak::ensure_apps(&host, refs)),
        Operation::CargoPackageSet { packages } => completed(packages::cargo::ensure(&host, packages)),
        Operation::CargoPackageUpdate => completed(packages::cargo::update_all(&host)),
        Operation::NpmPackageSet { packages } => completed(packages::npm::ensure(&host, packages)),
        Operation::NpmPackageUpdate => completed(packages::npm::update_all(&host)),
        Operation::Appimaged { architecture } => completed(appimaged::execute(&host, *architecture)),
        Operation::BinaryPackage(package) => completed(binary::execute(&host, package)),
        Operation::NerdFonts { families, mode } => completed(packages::fonts::execute(&host, families, *mode)),
        Operation::Dotfiles { root, packages, replace } => {
            completed(packages::dotfiles::execute(&host, root, packages, *replace))
        }
        Operation::DockerGroup => completed(system::docker_group(&host)),
        Operation::DockerLocalLog { max_size } => completed(system::docker_local_log(&host, max_size.as_deref())),
        Operation::VirtualBoxGroup => completed(system::virtualbox_group(&host)),
        Operation::VsCodeExtensionSet { extensions } => completed(system::vscode_extensions(&host, extensions)),
        Operation::DesktopSetting { target, setting } => completed(system::desktop_setting(&host, *target, setting)),
        Operation::GnomeExtensions { extensions } => system::gnome_extensions(&host, extensions),
        Operation::GnomeDock => system::gnome_dock(&host),
        Operation::GnomeRoundedCorners => system::gnome_rounded_corners(&host),
        Operation::AptUpgrade { policy } => completed(apt::upgrade(&host, *policy)),
        Operation::FlatpakUpdateApps => completed(packages::flatpak::update_apps(&host)),
        Operation::HomebrewBootstrap => completed(macos::bootstrap(&host)),
        Operation::HomebrewPackages { formulae, casks } => completed(macos::packages(&host, formulae, casks)),
        Operation::MacEnsureAdmin => completed(macos::ensure_admin(&host)),
        Operation::XcodeCommandLineTools => completed(macos::xcode_command_line_tools(&host)),
        Operation::Rosetta => completed(macos::rosetta(&host)),
        Operation::UserNerdFonts { families, mode } => completed(packages::fonts::execute_user(&host, families, *mode)),
        Operation::MacDefaults { settings } => completed(macos::defaults(&host, settings)),
        Operation::HomebrewUpdate { formulae, casks } => completed(macos::update(&host, *formulae, *casks)),
    }
}

fn completed(result: Result<()>) -> Result<OperationOutcome> {
    result.map(|()| OperationOutcome::Completed)
}
