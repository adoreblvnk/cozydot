mod appimaged;
mod binary;
pub(crate) mod macos;

mod repository;
mod system;
mod tools;

pub use apt::AptUpgradePolicy;
pub use binary::{BinaryPackageOperation, BinarySourceOperation};
pub use packages::fonts::NerdFontsMode;
pub use repository::AptRepositoryOperation;
pub use system::{DesktopEnvironment, DesktopSetting, DesktopTheme};
pub use tools::GoToolchainSelector;

use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Output},
};

const RUSTUP_BOOTSTRAP_FLAGS: [&str; 3] = ["-y", "--default-toolchain", "none"];

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

pub(crate) struct Host {
    home: PathBuf,
}

impl Host {
    fn new() -> Result<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from).context("HOME is not set")?;
        Ok(Self { home })
    }

    pub fn run<I, S>(&self, program: &str, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args.into_iter().map(|arg| arg.as_ref().to_os_string()).collect::<Vec<_>>();
        let mut command = Command::new(program);
        command.args(&args);
        command.output().with_context(|| format!("{program} operation: start {}", display(program, &args)))
    }

    pub fn require<I, S>(&self, operation: &str, program: &str, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run(program, args)?;
        if !output.status.success() {
            bail!(
                "{operation}: {program} failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(output)
    }

    pub fn home(&self) -> PathBuf {
        self.home.clone()
    }

    pub fn temp_dir(&self) -> PathBuf {
        self.value("TMPDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
    }

    pub fn value(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }
}

pub(crate) struct TempPath(tempfile::TempPath);

impl TempPath {
    pub fn new(host: &Host, stem: &str) -> Result<Self> {
        Self::new_with_suffix(host, stem, "")
    }

    pub fn new_with_suffix(host: &Host, stem: &str, suffix: &str) -> Result<Self> {
        Self::new_in_with_suffix(&host.temp_dir(), stem, suffix)
    }

    pub fn new_in_with_suffix(parent: &Path, stem: &str, suffix: &str) -> Result<Self> {
        tempfile::Builder::new()
            .prefix(stem)
            .suffix(suffix)
            .tempfile_in(parent)
            .map(|file| Self(file.into_temp_path()))
            .context("create operation temporary file")
    }

    pub fn path(&self) -> &Path {
        self.0.as_ref()
    }
}

fn display(program: &str, args: &[OsString]) -> String {
    std::iter::once(OsStr::new(program))
        .chain(args.iter().map(OsString::as_os_str))
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) mod privileged_file {
    use super::{Host, TempPath};
    use anyhow::{Context, Result, bail};
    use std::{ffi::OsStr, fs, io::Write, path::Path};

    pub(crate) fn publish_bytes(host: &Host, destination: &Path, contents: &[u8], operation: &str) -> Result<()> {
        publish_bytes_with_mode(host, destination, contents, operation, "0644")
    }

    pub(crate) fn publish_bytes_with_mode(
        host: &Host,
        destination: &Path,
        contents: &[u8],
        operation: &str,
        mode: &str,
    ) -> Result<()> {
        if !matches!(mode, "0600" | "0644") {
            bail!("unsupported privileged publication mode");
        }
        let local = TempPath::new(host, "privileged-publication")?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(local.path())
            .context("open local publication staging file")?;
        file.write_all(contents).context("write local publication staging file")?;
        file.sync_all().context("sync local publication staging file")?;
        drop(file);
        let parent = destination.parent().context("publication destination has no parent")?;
        let file_name = destination.file_name().context("publication destination has no filename")?.to_string_lossy();
        let nonce = local.path().file_name().context("publication staging file has no filename")?.to_string_lossy();
        let staged = parent.join(format!(".{file_name}.{nonce}.tmp"));
        let parent_arg = parent.as_os_str();
        let local_arg = local.path().as_os_str();
        let staged_arg = staged.as_os_str();
        let destination_arg = destination.as_os_str();
        host.require(
            operation,
            "sudo",
            [
                OsStr::new("install"),
                OsStr::new("-d"),
                OsStr::new("-o"),
                OsStr::new("root"),
                OsStr::new("-g"),
                OsStr::new("root"),
                OsStr::new("-m"),
                OsStr::new("0755"),
                OsStr::new("--"),
                parent_arg,
            ],
        )?;
        let result = (|| {
            host.require(
                operation,
                "sudo",
                [
                    OsStr::new("install"),
                    OsStr::new("-o"),
                    OsStr::new("root"),
                    OsStr::new("-g"),
                    OsStr::new("root"),
                    OsStr::new("-m"),
                    OsStr::new(mode),
                    OsStr::new("--"),
                    local_arg,
                    staged_arg,
                ],
            )?;
            host.require(operation, "sudo", [OsStr::new("sync"), OsStr::new("--"), staged_arg])?;
            host.require(operation, "sudo", [OsStr::new("test"), OsStr::new("!"), OsStr::new("-d"), destination_arg])?;
            host.require(
                operation,
                "sudo",
                [OsStr::new("mv"), OsStr::new("-fT"), OsStr::new("--"), staged_arg, destination_arg],
            )?;
            sync_parent(host, destination, operation)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = host.run("sudo", [OsStr::new("rm"), OsStr::new("-f"), OsStr::new("--"), staged_arg]);
        }
        result
    }

    pub(crate) fn sync_parent(host: &Host, destination: &Path, operation: &str) -> Result<()> {
        let parent = destination.parent().context("publication destination has no parent")?;
        host.require(operation, "sudo", [OsStr::new("sync"), OsStr::new("--"), parent.as_os_str()])?;
        Ok(())
    }
}

pub(crate) mod apt {
    use crate::operations::Host;
    use anyhow::Result;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AptUpgradePolicy {
        Standard,
        Full,
    }

    pub fn metadata_refresh(host: &Host) -> Result<()> {
        host.require("APT metadata refresh", "sudo", ["apt-get", "update", "-qq"])?;
        Ok(())
    }

    pub fn bootstrap_packages(host: &Host, packages: &[String]) -> Result<()> {
        if packages.is_empty() {
            anyhow::bail!("APT bootstrap package sequence must not be empty");
        }
        let missing = missing_packages(host, packages)?;
        if missing.is_empty() {
            return Ok(());
        }
        host.require("APT bootstrap metadata refresh", "sudo", ["apt-get", "update", "-qq"])?;
        install(host, "APT bootstrap package installation", missing)
    }

    pub fn packages(host: &Host, packages: &[String]) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }
        let missing = missing_packages(host, packages)?;
        if missing.is_empty() {
            return Ok(());
        }
        install(host, "APT package installation", missing)
    }

    pub fn repository_packages(host: &Host, conflicts: &[String], packages: &[String]) -> Result<()> {
        purge(host, conflicts)?;
        self::packages(host, packages)
    }

    fn missing_packages(host: &Host, packages: &[String]) -> Result<Vec<String>> {
        packages
            .iter()
            .filter_map(|package| match package_is_installed(host, package) {
                Ok(true) => None,
                Ok(false) => Some(Ok(package.clone())),
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    fn package_is_installed(host: &Host, package: &str) -> Result<bool> {
        let output = host.run("dpkg-query", ["-W", "-f=${db:Status-Status}\\n", "--", package])?;
        if !output.status.success() {
            if output.status.code() == Some(1) {
                return Ok(false);
            }
            anyhow::bail!(
                "APT package inspection failed for {package:?}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        match output.stdout.as_slice() {
            b"installed\n" => Ok(true),
            b"not-installed\n"
            | b"config-files\n"
            | b"half-installed\n"
            | b"unpacked\n"
            | b"half-configured\n"
            | b"triggers-awaited\n"
            | b"triggers-pending\n" => Ok(false),
            _ => anyhow::bail!("APT package inspection returned malformed state for {package:?}"),
        }
    }

    fn install(host: &Host, operation: &str, packages: Vec<String>) -> Result<()> {
        let mut args = vec![
            "DEBIAN_FRONTEND=noninteractive".to_owned(),
            "apt-get".to_owned(),
            "install".into(),
            "-y".into(),
            "-qq".into(),
            "--".into(),
        ];
        args.extend(packages.into_iter().map(|package| format!("{package}+")));
        host.require(operation, "sudo", args)?;
        Ok(())
    }

    pub fn purge(host: &Host, packages: &[String]) -> Result<()> {
        if packages.is_empty() {
            return Ok(());
        }
        let installed = packages
            .iter()
            .map(|package| Ok((package, package_is_installed(host, package)?)))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|(_, installed)| *installed)
            .map(|(package, _)| package.clone())
            .collect::<Vec<_>>();
        if installed.is_empty() {
            return Ok(());
        }
        let mut args = vec![
            "DEBIAN_FRONTEND=noninteractive".to_owned(),
            "apt-get".to_owned(),
            "purge".into(),
            "-y".into(),
            "-qq".into(),
            "--".into(),
        ];
        args.extend(installed);
        host.require("APT package purge", "sudo", args)?;
        Ok(())
    }

    pub fn upgrade(host: &Host, policy: AptUpgradePolicy) -> Result<()> {
        match policy {
            AptUpgradePolicy::Standard => {
                host.require(
                    "APT standard upgrade",
                    "sudo",
                    ["DEBIAN_FRONTEND=noninteractive", "apt-get", "upgrade", "-y", "-qq", "--"],
                )?;
            }
            AptUpgradePolicy::Full => {
                host.require(
                    "APT full upgrade",
                    "sudo",
                    ["DEBIAN_FRONTEND=noninteractive", "apt-get", "full-upgrade", "-y", "-qq", "--"],
                )?;
                host.require(
                    "APT purge autoremove",
                    "sudo",
                    ["DEBIAN_FRONTEND=noninteractive", "apt-get", "autoremove", "--purge", "-y", "-qq", "--"],
                )?;
            }
        }
        Ok(())
    }
}

pub(crate) mod languages {
    use anyhow::{Context, Result, bail};
    use std::{ffi::OsStr, os::unix::fs::PermissionsExt, path::PathBuf};

    use crate::operations::{Host, RUSTUP_BOOTSTRAP_FLAGS, TempPath};

    pub fn rustup(host: &Host) -> Result<()> {
        let cargo_home = host.value("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".cargo"));
        if !cargo_home.is_absolute() {
            bail!("rustup managed CARGO_HOME must be absolute");
        }
        if executable_file(&cargo_home.join("bin/rustup")) {
            return Ok(());
        }
        let installer = TempPath::new(host, "rustup")?;
        host.require(
            "rustup bootstrap download",
            "curl",
            [
                "--proto",
                "=https",
                "--tlsv1.2",
                "-sSf",
                "-o",
                &installer.path().to_string_lossy(),
                "https://sh.rustup.rs",
            ],
        )?;
        host.require(
            "rustup bootstrap",
            "sh",
            std::iter::once(installer.path().as_os_str()).chain(RUSTUP_BOOTSTRAP_FLAGS.map(OsStr::new)),
        )?;
        if !executable_file(&cargo_home.join("bin/rustup")) {
            bail!("rustup bootstrap did not publish the managed rustup executable");
        }
        Ok(())
    }

    pub fn fnm_bootstrap(host: &Host) -> Result<()> {
        let data_home =
            host.value("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/share"));
        if !data_home.is_absolute() {
            bail!("FNM managed data directory must be absolute");
        }
        let installed = data_home.join("fnm/fnm");
        if executable_file(&installed) {
            return Ok(());
        }
        let installer = TempPath::new(host, "fnm-install")?;
        host.require(
            "FNM bootstrap download",
            "curl",
            ["-fsSL", "-o", &installer.path().to_string_lossy(), "https://fnm.vercel.app/install"],
        )?;
        host.require("FNM bootstrap", "bash", [&installer.path().to_string_lossy(), "--skip-shell"])?;
        if !executable_file(&installed) {
            bail!("FNM bootstrap did not publish executable {}", installed.display());
        }
        Ok(())
    }

    pub fn uv_bootstrap(host: &Host) -> Result<()> {
        let install_dir =
            host.value("UV_INSTALL_DIR").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/bin"));
        if !install_dir.is_absolute() {
            bail!("uv managed install directory must be absolute");
        }
        let installed = install_dir.join("uv");
        if executable_file(&installed) {
            return Ok(());
        }
        let installer = TempPath::new(host, "uv-install")?;
        host.require(
            "uv bootstrap download",
            "curl",
            ["-LsSf", "-o", &installer.path().to_string_lossy(), "https://astral.sh/uv/install.sh"],
        )?;
        std::fs::create_dir_all(&install_dir).context("uv bootstrap: create install directory")?;
        host.require(
            "uv bootstrap",
            "env",
            vec![
                format!("UV_UNMANAGED_INSTALL={}", install_dir.display()),
                "sh".into(),
                installer.path().to_string_lossy().into_owned(),
            ],
        )?;
        if !executable_file(&installed) {
            bail!("uv bootstrap did not publish executable {}", installed.display());
        }
        Ok(())
    }

    fn executable_file(path: &std::path::Path) -> bool {
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
}

pub(crate) mod packages {

    pub(crate) mod cargo {
        use anyhow::{Context, Result, bail};
        use std::{
            os::unix::fs::PermissionsExt,
            path::{Path, PathBuf},
        };

        use super::super::Host;

        pub(crate) fn ensure(host: &Host, packages: &[String]) -> Result<()> {
            let cargo_home = host.value("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".cargo"));
            if !cargo_home.is_absolute() {
                bail!("Cargo package operation requires an absolute CARGO_HOME");
            }
            let cargo = path_program(&cargo_home.join("bin/cargo"), "managed Cargo executable path")?;
            let output = host.require("Cargo installed package query", &cargo, ["install", "--list"])?;
            let installed = installed_crates(&output.stdout)?;
            let missing = packages
                .iter()
                .filter(|package| !installed.contains(crate_identity(package)))
                .cloned()
                .collect::<Vec<_>>();
            if missing.is_empty() {
                return Ok(());
            }
            let binstall = resolve_binstall(&cargo_home)?
                .context("Cargo package operation: managed cargo-binstall is unavailable after bootstrap")?;
            let mut args = vec!["--no-confirm".to_owned(), "--".into()];
            args.extend(missing);
            host.require("Cargo package mutation", &binstall, args)?;
            Ok(())
        }

        pub(crate) fn update_all(host: &Host) -> Result<()> {
            let cargo_home = host.value("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".cargo"));
            if !cargo_home.is_absolute() {
                bail!("Cargo package operation requires an absolute CARGO_HOME");
            }
            let cargo_path = cargo_home.join("bin/cargo");
            if !executable_file(&cargo_path) {
                return Ok(());
            }
            let cargo = path_program(&cargo_path, "managed Cargo executable path")?;
            let output = host.require("Cargo installed package query", &cargo, ["install", "--list"])?;
            let packages = installed_crates(&output.stdout)?;
            if packages.is_empty() {
                return Ok(());
            }
            let mut args = vec!["install".to_owned(), "--locked".into(), "--".into()];
            args.extend(packages);
            host.require("Cargo package convergence", &cargo, args)?;
            Ok(())
        }

        fn installed_crates(output: &[u8]) -> Result<std::collections::BTreeSet<String>> {
            let output = std::str::from_utf8(output).context("cargo install --list returned non-UTF-8 state")?;
            let mut installed = std::collections::BTreeSet::new();
            for line in output.lines().filter(|line| !line.is_empty()) {
                if line.starts_with(char::is_whitespace) {
                    continue;
                }
                let header = line.strip_suffix(':').context("cargo install --list returned malformed state")?;
                let mut fields = header.splitn(3, char::is_whitespace).filter(|field| !field.is_empty());
                let name = fields.next().context("cargo install --list returned malformed state")?;
                let version = fields.next().context("cargo install --list returned malformed state")?;
                if !version.starts_with('v') || name.chars().any(char::is_control) {
                    bail!("cargo install --list returned malformed state");
                }
                match fields.next() {
                    None => {
                        installed.insert(name.to_owned());
                    }
                    Some(source)
                        if source.starts_with('(')
                            && source.ends_with(')')
                            && !source.chars().any(char::is_control) => {}
                    Some(_) => bail!("cargo install --list returned malformed state"),
                }
            }
            Ok(installed)
        }

        fn crate_identity(package: &str) -> &str {
            package.split_once('@').map_or(package, |(name, _)| name)
        }

        fn resolve_binstall(cargo_home: &Path) -> Result<Option<String>> {
            let managed = cargo_home.join("bin/cargo-binstall");
            if executable_file(&managed) {
                return path_program(&managed, "cargo-binstall executable path").map(Some);
            }
            Ok(None)
        }

        fn executable_file(path: &Path) -> bool {
            std::fs::metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        }

        fn path_program(path: &Path, description: &str) -> Result<String> {
            path.to_str().map(str::to_owned).with_context(|| format!("{description} is not UTF-8: {}", path.display()))
        }
    }

    pub(crate) mod npm {
        use anyhow::{Context, Result, bail};
        use std::{
            os::unix::fs::PermissionsExt,
            path::{Path, PathBuf},
        };

        use super::super::Host;

        pub(crate) fn ensure(host: &Host, packages: &[String]) -> Result<()> {
            let Some(fnm) = resolve_fnm(host)? else {
                bail!("npm package operation: managed fnm is unavailable after bootstrap");
            };
            let mut missing = Vec::new();
            for package in packages {
                let identity = package_identity(package);
                let output = host.run(
                    &fnm,
                    ["exec", "--using=default", "--", "npm", "list", "--global", "--depth=0", "--", identity],
                )?;
                if !output.status.success() {
                    missing.push(package.clone());
                }
            }
            if missing.is_empty() {
                return Ok(());
            }
            let mut npm_args = vec!["install".to_owned(), "--global".into(), "--".into()];
            npm_args.extend(missing);
            run_npm_required(host, &fnm, "npm package mutation", npm_args)?;
            Ok(())
        }

        pub(crate) fn update_all(host: &Host) -> Result<()> {
            let Some(fnm) = resolve_fnm(host)? else { return Ok(()) };
            run_npm_required(host, &fnm, "npm package update", ["update", "--global"])?;
            Ok(())
        }

        fn package_identity(package: &str) -> &str {
            if package.starts_with('@') {
                let slash = package.find('/').unwrap_or(package.len());
                let version = package[slash..].find('@').map(|index| slash + index);
                return version.map_or(package, |index| &package[..index]);
            }
            package.split_once('@').map_or(package, |(name, _)| name)
        }

        fn resolve_fnm(host: &Host) -> Result<Option<String>> {
            let data_home =
                host.value("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/share"));
            if !data_home.is_absolute() {
                bail!("npm package operation requires an absolute managed FNM data directory");
            }
            let managed = data_home.join("fnm/fnm");
            if executable_file(&managed) {
                return managed
                    .to_str()
                    .map(str::to_owned)
                    .map(Some)
                    .context("managed fnm executable path is not UTF-8");
            }
            Ok(None)
        }

        fn run_npm_required<I, S>(host: &Host, fnm: &str, operation: &str, npm_args: I) -> Result<std::process::Output>
        where
            I: IntoIterator<Item = S>,
            S: AsRef<str>,
        {
            let mut args = vec!["exec".to_owned(), "--using=default".into(), "--".into(), "npm".into()];
            args.extend(npm_args.into_iter().map(|arg| arg.as_ref().to_owned()));
            host.require(operation, fnm, args)
        }

        fn executable_file(path: &Path) -> bool {
            std::fs::metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        }
    }

    pub(crate) mod flatpak {
        use super::super::Host;
        use anyhow::Result;

        const FLATHUB_NAME: &str = "flathub";
        const FLATHUB_DESCRIPTOR_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
        const FLATHUB_URL: &str = "https://dl.flathub.org/repo/";

        pub fn ensure_flathub(host: &Host) -> Result<()> {
            host.require(
                "Flathub remote ensure",
                "flatpak",
                ["--user", "remote-add", "--if-not-exists", FLATHUB_NAME, FLATHUB_DESCRIPTOR_URL],
            )?;
            let url_arg = format!("--url={FLATHUB_URL}");
            host.require(
                "Flathub remote security canonicalization",
                "flatpak",
                [
                    "--user",
                    "remote-modify",
                    &url_arg,
                    "--gpg-verify",
                    "--enumerate",
                    "--use-for-deps",
                    "--enable",
                    "--no-filter",
                    FLATHUB_NAME,
                ],
            )?;
            Ok(())
        }

        pub fn ensure_apps(host: &Host, refs: &[String]) -> Result<()> {
            let mut missing = Vec::new();
            for app_id in refs {
                let output = host.run("flatpak", ["--user", "info", "--show-ref", "--", app_id])?;
                if !output.status.success() {
                    missing.push(app_id.clone());
                }
            }
            if missing.is_empty() {
                return Ok(());
            }
            let mut args = vec![
                "--user".to_owned(),
                "install".into(),
                "--app".into(),
                "--noninteractive".into(),
                "-y".into(),
                "flathub".into(),
                "--".into(),
            ];
            args.extend(missing);
            host.require("Flatpak application installation", "flatpak", args)?;
            Ok(())
        }

        pub fn update_apps(host: &Host) -> Result<()> {
            host.require(
                "Flatpak application update",
                "flatpak",
                ["--user", "update", "--app", "--noninteractive", "-y"],
            )?;
            Ok(())
        }
    }

    pub(crate) mod fonts {
        use anyhow::{Context, Result, bail};
        use std::{ffi::OsStr, fs, path::Path};
        use url::Url;

        use super::super::{Host, TempPath};

        const FONT_ROOT: &str = "/usr/share/fonts";

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum NerdFontsMode {
            EnsurePresent,
            Update,
        }

        pub(crate) fn execute(host: &Host, families: &[String], mode: NerdFontsMode) -> Result<()> {
            execute_at(host, families, mode, Path::new(FONT_ROOT), true)
        }

        pub(crate) fn execute_user(host: &Host, families: &[String], mode: NerdFontsMode) -> Result<()> {
            let parent = host.home().join("Library/Fonts");
            fs::create_dir_all(&parent).context("create user font directory")?;
            execute_at(host, families, mode, &parent, false)
        }

        fn execute_at(
            host: &Host,
            families: &[String],
            mode: NerdFontsMode,
            parent: &Path,
            privileged: bool,
        ) -> Result<()> {
            let mut changed = false;
            for family in families {
                let destination = parent.join(family);
                let is_present = match fs::symlink_metadata(&destination) {
                    Ok(metadata) if metadata.is_dir() => true,
                    Ok(_) => bail!("Nerd Font destination conflict at {}", destination.display()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                    Err(error) => {
                        return Err(error).context(format!("inspect Nerd Font destination {}", destination.display()));
                    }
                };
                if mode == NerdFontsMode::Update || !is_present {
                    install_family(host, family, &destination, privileged)?;
                    changed = true;
                }
            }
            if changed && privileged {
                host.require(
                    "Nerd Font cache refresh",
                    "sudo",
                    [OsStr::new("fc-cache"), OsStr::new("--force"), parent.as_os_str()],
                )?;
            }
            Ok(())
        }

        fn install_family(host: &Host, family: &str, destination: &Path, privileged: bool) -> Result<()> {
            let archive = TempPath::new_with_suffix(host, "nerd-font", ".tar.xz")?;
            let mut url =
                Url::parse("https://github.com/ryanoasis/nerd-fonts/releases/latest/download/placeholder.tar.xz")?;
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Nerd Fonts URL cannot be a base"))?
                .pop()
                .push(&format!("{family}.tar.xz"));
            host.require(
                "Nerd Font archive download",
                "curl",
                [
                    "--proto".as_ref(),
                    "=https".as_ref(),
                    "--location".as_ref(),
                    "--fail".as_ref(),
                    "--silent".as_ref(),
                    "--show-error".as_ref(),
                    "--retry".as_ref(),
                    "3".as_ref(),
                    "--retry-all-errors".as_ref(),
                    "--output".as_ref(),
                    archive.path().as_os_str(),
                    "--".as_ref(),
                    url.as_str().as_ref(),
                ],
            )?;
            let path = destination.to_str().context("font path is not UTF-8")?;
            let archive_path = archive.path().to_str().context("font archive path is not UTF-8")?;
            if privileged {
                host.require(
                    "Nerd Font destination replacement",
                    "sudo",
                    ["rm", "--recursive", "--force", "--", path],
                )?;
                host.require("Nerd Font destination creation", "sudo", ["mkdir", "--parents", "--", path])?;
                host.require(
                    "Nerd Font archive extraction",
                    "sudo",
                    ["tar", "--extract", "--xz", "--directory", path, "--file", archive_path],
                )?;
            } else {
                host.require("Nerd Font destination replacement", "rm", ["-rf", path])?;
                host.require("Nerd Font destination creation", "mkdir", ["-p", path])?;
                host.require("Nerd Font archive extraction", "tar", ["-xJf", archive_path, "-C", path])?;
            }
            Ok(())
        }
    }

    pub(crate) mod dotfiles {
        use anyhow::{Context, Result, bail};
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        use super::super::Host;

        pub(crate) fn execute(host: &Host, root: &Path, packages: &[String], replace: bool) -> Result<()> {
            let root = fs::canonicalize(root)
                .with_context(|| format!("dotfiles operation: canonicalize root {}", root.display()))?;
            if !fs::symlink_metadata(&root)?.file_type().is_dir() {
                bail!("dotfiles root is not a directory: {}", root.display());
            }

            let mut sources = Vec::with_capacity(packages.len());
            for package in packages {
                let source = root.join(package);
                let metadata = fs::symlink_metadata(&source)
                    .with_context(|| format!("dotfiles package {package:?} does not exist"))?;
                if !metadata.file_type().is_dir() {
                    bail!("dotfiles package {package:?} is not a real directory");
                }
                sources.push((package, source));
            }

            let mut conflicts = Vec::new();
            for (package, source) in &sources {
                collect_conflicts(source, host.home(), package, &mut conflicts)?;
            }
            conflicts.sort_by(|left, right| left.1.cmp(&right.1));
            conflicts.dedup_by(|left, right| left.1 == right.1);
            if !conflicts.is_empty() && !replace {
                let paths =
                    conflicts.iter().map(|(_, path)| format!("  {}", path.display())).collect::<Vec<_>>().join("\n");
                bail!(
                    "unmanaged dotfile conflicts:\n{paths}\nno dotfiles were changed; rerun with `cozydot dotfiles --replace`"
                );
            }
            host.require("Stow preflight", "stow", ["--version"]).context("dotfiles require GNU Stow")?;
            if replace {
                backup_conflicts(host, &conflicts)?;
            }

            for (package, source) in sources {
                apply_package(host, &root, package, &source)?;
            }
            Ok(())
        }

        fn apply_package(host: &Host, root: &Path, package: &str, source: &Path) -> Result<()> {
            prepare_gnupg_home(source, &host.home())?;
            host.require(
                "dotfiles Stow mutation",
                "stow",
                [
                    "--dir".as_ref(),
                    root.as_os_str(),
                    "--target".as_ref(),
                    host.home().as_os_str(),
                    "--stow".as_ref(),
                    "--".as_ref(),
                    package.as_ref(),
                ],
            )?;
            Ok(())
        }

        fn prepare_gnupg_home(source: &Path, home: &Path) -> Result<()> {
            let source = source.join(".gnupg");
            if !source.is_dir() {
                return Ok(());
            }

            let target = home.join(".gnupg");
            if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                fs::remove_file(&target).context("replace folded GnuPG dotfiles directory")?;
            }
            fs::create_dir_all(&target).context("create GnuPG home")?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).context("secure GnuPG home")?;
            Ok(())
        }

        fn collect_conflicts(
            source: &Path,
            target: PathBuf,
            package: &str,
            conflicts: &mut Vec<(String, PathBuf)>,
        ) -> Result<()> {
            let source_metadata = fs::symlink_metadata(source)
                .with_context(|| format!("inspect dotfiles source {}", source.display()))?;
            if source_metadata.file_type().is_dir() {
                match fs::symlink_metadata(&target) {
                    Ok(metadata) if metadata.file_type().is_dir() => {
                        let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
                        entries.sort_by_key(std::fs::DirEntry::file_name);
                        for entry in entries {
                            collect_conflicts(&entry.path(), target.join(entry.file_name()), package, conflicts)?;
                        }
                    }
                    Ok(_) if !resolves_to(&target, source) => conflicts.push((package.to_owned(), target)),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("inspect dotfile destination"),
                }
            } else if source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
                match fs::symlink_metadata(&target) {
                    Ok(_) if !resolves_to(&target, source) => conflicts.push((package.to_owned(), target)),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("inspect dotfile destination"),
                }
            } else {
                bail!("unsupported dotfiles source type at {}", source.display());
            }
            Ok(())
        }

        fn backup_conflicts(host: &Host, conflicts: &[(String, PathBuf)]) -> Result<()> {
            if conflicts.is_empty() {
                return Ok(());
            }
            let state_home =
                host.value("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/state"));
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("dotfiles backup timestamp is before the Unix epoch")?
                .as_nanos();
            let backup_root =
                state_home.join("cozydot/dotfile-backups").join(format!("{timestamp}-{}", std::process::id()));
            for (package, conflict) in conflicts {
                let relative = conflict
                    .strip_prefix(host.home())
                    .with_context(|| format!("dotfiles conflict escaped the home directory: {}", conflict.display()))?;
                let backup = backup_root.join(package).join(relative);
                let parent = backup.parent().context("dotfiles backup has no parent")?;
                fs::create_dir_all(parent).context("create dotfiles backup directory")?;
                fs::rename(conflict, &backup).with_context(|| {
                    format!("move dotfiles conflict {} to {}", conflict.display(), backup.display())
                })?;
                if fs::symlink_metadata(conflict).is_ok() || fs::symlink_metadata(&backup).is_err() {
                    bail!("dotfiles conflict backup did not move {} to {}", conflict.display(), backup.display());
                }
            }
            Ok(())
        }

        fn resolves_to(target: &Path, source: &Path) -> bool {
            fs::canonicalize(target)
                .and_then(|target| fs::canonicalize(source).map(|source| target == source))
                .unwrap_or(false)
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use std::os::unix::fs::{PermissionsExt, symlink};

            #[test]
            fn gnupg_package_replaces_folded_directory_with_private_home() {
                let package = tempfile::tempdir().unwrap();
                let source = package.path().join(".gnupg");
                fs::create_dir(&source).unwrap();
                let home = tempfile::tempdir().unwrap();
                symlink(&source, home.path().join(".gnupg")).unwrap();

                prepare_gnupg_home(package.path(), home.path()).unwrap();

                let metadata = fs::symlink_metadata(home.path().join(".gnupg")).unwrap();
                assert!(metadata.is_dir());
                assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
            }
        }
    }
}

pub(super) fn latest_go(input: &str, requested: &str, arch: &str, target_os: &str) -> anyhow::Result<(String, String)> {
    use anyhow::Context;
    let value: serde_json::Value = serde_json::from_str(input).context("parse Go release JSON")?;
    let releases = value.as_array().context("Go metadata must be an array")?;
    let version = releases
        .iter()
        .filter_map(|release| release["version"].as_str())
        .filter(|v| stable_go_version(v))
        .map(|v| v.trim_start_matches("go"))
        .find(|v| {
            requested == "latest"
                || *v == requested
                || v.strip_prefix(requested).is_some_and(|rest| rest.starts_with('.'))
        })
        .context("Go metadata has no matching stable release")?;
    let filename = format!("go{version}.{target_os}-{arch}.tar.gz");
    releases
        .iter()
        .find(|release| release["version"].as_str() == Some(&format!("go{version}")))
        .and_then(|release| release["files"].as_array())
        .and_then(|files| files.iter().find(|file| file["filename"].as_str() == Some(&filename)))
        .context("Go metadata has no matching architecture archive")?;
    Ok((version.to_owned(), filename))
}

pub(super) fn gnome_version(input: &str, shell_version: &str) -> anyhow::Result<u64> {
    use anyhow::{Context, bail};
    let value: serde_json::Value = serde_json::from_str(input).context("parse GNOME extension JSON")?;
    let versions = value["shell_version_map"].as_object().context("GNOME response has no shell_version_map")?;
    let mut candidate = shell_version;
    loop {
        if let Some(version) = versions.get(candidate).and_then(|entry| entry["version"].as_u64()) {
            return Ok(version);
        }
        let Some((parent, _)) = candidate.rsplit_once('.') else {
            bail!("GNOME response has no extension version for shell {shell_version}");
        };
        candidate = parent;
    }
}

pub(super) fn gnome_shell_version(input: &str) -> anyhow::Result<String> {
    use anyhow::Context;
    input
        .split_whitespace()
        .map(|part| part.trim_matches(|character: char| !character.is_ascii_digit() && character != '.'))
        .find(|part| {
            !part.is_empty()
                && part
                    .split('.')
                    .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .map(str::to_owned)
        .context("GNOME Shell version output has no numeric version")
}

fn stable_go_version(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("go") else {
        return false;
    };
    let parts = rest.split('.').collect::<Vec<_>>();
    (parts.len() == 2 || parts.len() == 3)
        && parts.iter().all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}
