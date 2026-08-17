use std::process::Command;

use anyhow::{Context, Result, bail};
use etc_os_release::OsRelease;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub identity: PlatformIdentity,
    pub distro_codename: String,
    pub base_codename: String,
    pub desktop: DesktopKind,
    pub architecture: Architecture,
}

impl Platform {
    pub fn detect() -> Result<Self> {
        if cfg!(target_os = "macos") {
            let uname = Command::new("uname").arg("-m").output().context("run uname -m")?;
            let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
            let architecture = match arch.as_str() {
                "aarch64" | "arm64" => Architecture::Aarch64,
                _ => bail!("unsupported macOS architecture {arch:?}; only Apple Silicon (arm64) is supported"),
            };
            return Ok(Self {
                identity: PlatformIdentity::MacOS,
                distro_codename: String::new(),
                base_codename: String::new(),
                desktop: DesktopKind::None,
                architecture,
            });
        }
        let os = OsRelease::open().context("read os-release")?;
        let uname = Command::new("uname").arg("-m").output().context("run uname -m")?;
        let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        Self::from_os_release(&os, &desktop, &arch)
    }

    fn from_os_release(os: &OsRelease, desktop: &str, arch: &str) -> Result<Self> {
        let distro = Distro::from_os_release(os.id())?;
        let family = distro.family(os.get_value("ID_LIKE"))?;
        let distro_codename = os.version_codename().unwrap_or_default().to_owned();
        let base_codename = match family {
            Family::Ubuntu => os.get_value("UBUNTU_CODENAME"),
            Family::Debian => os.get_value("DEBIAN_CODENAME"),
        }
        .unwrap_or(&distro_codename)
        .to_owned();
        if distro == Distro::Debian && !matches!(distro_codename.as_str(), "bookworm" | "trixie") {
            bail!("unsupported Debian release {distro_codename:?}; supported releases are bookworm and trixie");
        }
        if distro_codename.chars().any(char::is_control) || base_codename.chars().any(char::is_control) {
            bail!("detected distribution codenames must fit on one line and contain no control characters");
        }
        Ok(Self {
            identity: PlatformIdentity::Linux { distro, family },
            distro_codename,
            base_codename,
            desktop: DesktopKind::from_environment(desktop),
            architecture: Architecture::normalize(arch)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Distro {
    Ubuntu,
    LinuxMint,
    Pop,
    Debian,
}

impl Distro {
    fn from_os_release(value: &str) -> Result<Self> {
        match value {
            "ubuntu" => Ok(Self::Ubuntu),
            "linuxmint" => Ok(Self::LinuxMint),
            "pop" => Ok(Self::Pop),
            "debian" => Ok(Self::Debian),
            _ => bail!("unsupported distro: {value}"),
        }
    }

    fn family(self, id_like: Option<&str>) -> Result<Family> {
        match self {
            Self::Ubuntu | Self::Pop => Ok(Family::Ubuntu),
            Self::Debian => Ok(Family::Debian),
            Self::LinuxMint => {
                let mut families = id_like.unwrap_or_default().split_ascii_whitespace();
                let ubuntu = families.clone().any(|family| family == "ubuntu");
                let debian = families.any(|family| family == "debian");
                match (ubuntu, debian) {
                    (true, _) => Ok(Family::Ubuntu),
                    (false, true) => Ok(Family::Debian),
                    _ => bail!(
                        "unsupported linuxmint base family in ID_LIKE {:?}; expected ubuntu or debian",
                        id_like.unwrap_or_default()
                    ),
                }
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ubuntu => "ubuntu",
            Self::LinuxMint => "linuxmint",
            Self::Pop => "pop",
            Self::Debian => "debian",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Ubuntu,
    Debian,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformIdentity {
    MacOS,
    Linux { distro: Distro, family: Family },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopKind {
    None,
    Gnome,
    Cinnamon,
}

impl DesktopKind {
    fn from_environment(value: &str) -> Self {
        value
            .split(':')
            .find_map(|token| {
                let token = token.to_ascii_lowercase();
                if token.contains("gnome") {
                    Some(Self::Gnome)
                } else if token.contains("cinnamon") {
                    Some(Self::Cinnamon)
                } else {
                    None
                }
            })
            .unwrap_or(Self::None)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gnome => "gnome",
            Self::Cinnamon => "cinnamon",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    X86_64,
    Aarch64,
    Arm,
}

impl Architecture {
    pub fn normalize(value: &str) -> Result<Self> {
        match value {
            "x86_64" | "amd64" => Ok(Self::X86_64),
            "aarch64" | "arm64" => Ok(Self::Aarch64),
            "arm" | "arm32" | "armv7" | "armv7l" | "armhf" => Ok(Self::Arm),
            _ => bail!("unsupported architecture {value:?}; supported architectures: x86_64, aarch64, arm"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::Arm => "arm",
        }
    }

    pub fn debian(self) -> &'static str {
        match self {
            Self::X86_64 => "amd64",
            Self::Aarch64 => "arm64",
            Self::Arm => "armhf",
        }
    }

    pub fn go(self) -> &'static str {
        match self {
            Self::X86_64 => "amd64",
            Self::Aarch64 => "arm64",
            Self::Arm => "arm",
        }
    }

    pub fn go_archive(self) -> &'static str {
        match self {
            // Go calls its 32-bit ARM archive armv6l; it also runs on ARMv7
            Self::Arm => "armv6l",
            other => other.go(),
        }
    }
}

fn parse_uname_machine(success: bool, stdout: &[u8]) -> Result<String> {
    if !success {
        bail!("uname -m failed");
    }
    let machine = std::str::from_utf8(stdout).context("uname -m output is not UTF-8")?.trim();
    if machine.is_empty() {
        bail!("uname -m returned an empty machine architecture");
    }
    Ok(machine.into())
}
