use anyhow::{Context, Result, bail, ensure};
use etc_os_release::OsRelease;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub identity: PlatformIdentity,
    pub distro_codename: String,
    pub base_codename: String,
    pub desktop: DesktopKind,
    pub arch: Arch,
}

impl Platform {
    pub fn detect() -> Result<Self> {
        let uname = rustix::system::uname();
        let arch = uname.machine().to_str().context("uname machine architecture is not UTF-8")?;
        ensure!(!arch.is_empty(), "uname returned an empty machine architecture");
        if cfg!(target_os = "macos") {
            let arch = match arch {
                "aarch64" | "arm64" => Arch::Aarch64,
                _ => bail!("unsupported macOS architecture {arch:?}; only Apple Silicon (arm64) is supported"),
            };
            return Ok(Self {
                identity: PlatformIdentity::Macos,
                distro_codename: String::new(),
                base_codename: String::new(),
                desktop: DesktopKind::None,
                arch,
            });
        }
        let os = OsRelease::open().context("read os-release")?;
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        Self::from_os_release(&os, &desktop, arch)
    }

    fn from_os_release(os: &OsRelease, desktop: &str, arch: &str) -> Result<Self> {
        let distro = Distro::from_os_release(os.id())?;
        let family = distro.family(os.get_value("ID_LIKE"))?;
        let distro_codename = os.version_codename().unwrap_or_default().to_owned();
        // derivatives use their base distro codename for family repositories
        let base_codename = match family {
            Family::Ubuntu => os.get_value("UBUNTU_CODENAME"),
            Family::Debian => os.get_value("DEBIAN_CODENAME"),
        };
        let base_codename = base_codename.unwrap_or(&distro_codename).to_owned();
        if distro == Distro::Debian && !["bookworm", "trixie"].contains(&distro_codename.as_str()) {
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
            arch: Arch::normalize(arch)?,
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
            _ => bail!("unsupported distro {value:?}; supported distros: Debian, Ubuntu, Pop!_OS, Linux Mint"),
        }
    }

    fn family(self, id_like: Option<&str>) -> Result<Family> {
        match self {
            Self::Ubuntu | Self::Pop => Ok(Family::Ubuntu),
            Self::Debian => Ok(Family::Debian),
            Self::LinuxMint => {
                let id_like = id_like.unwrap_or_default();
                let ubuntu = id_like.split_ascii_whitespace().any(|family| family == "ubuntu");
                let debian = id_like.split_ascii_whitespace().any(|family| family == "debian");
                // regular Mint lists Ubuntu & Debian; LMDE lists Debian only
                match (ubuntu, debian) {
                    (true, _) => Ok(Family::Ubuntu),
                    (false, true) => Ok(Family::Debian),
                    _ => bail!("unsupported linuxmint base family in ID_LIKE {id_like:?}; expected ubuntu or debian"),
                }
            }
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
    Macos,
    Linux { distro: Distro, family: Family },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopKind {
    None,
    Gnome,
}

impl DesktopKind {
    fn from_environment(value: &str) -> Self {
        // XDG_CURRENT_DESKTOP may be vendor-prefixed or contain multiple desktop names
        if value.to_ascii_lowercase().contains("gnome") { Self::Gnome } else { Self::None }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gnome => "gnome",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    pub fn normalize(value: &str) -> Result<Self> {
        match value {
            "x86_64" | "amd64" => Ok(Self::X86_64),
            "aarch64" | "arm64" => Ok(Self::Aarch64),
            _ => bail!("unsupported architecture {value:?}; supported architectures: x86_64, aarch64"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }

    pub fn debian(self) -> &'static str {
        match self {
            Self::X86_64 => "amd64",
            Self::Aarch64 => "arm64",
        }
    }

    pub fn go(self) -> &'static str {
        match self {
            Self::X86_64 => "amd64",
            Self::Aarch64 => "arm64",
        }
    }
}
