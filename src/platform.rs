use std::process::Command;

use anyhow::{Context, Result, bail};
use etc_os_release::OsRelease;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub distro: String,
    pub upstream: String,
    pub distro_codename: String,
    pub base_codename: String,
    pub desktop: String,
    pub architecture: Architecture,
}

impl Platform {
    pub fn detect() -> Result<Self> {
        if cfg!(target_os = "macos") {
            let uname = Command::new("uname").arg("-m").output().context("run uname -m")?;
            let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
            return Self::from_release_parts(
                "macos".into(),
                "macos".into(),
                String::new(),
                String::new(),
                "none".into(),
                &arch,
            );
        }
        let os = OsRelease::open().context("read os-release")?;
        let uname = Command::new("uname").arg("-m").output().context("run uname -m")?;
        let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
        let desktop = normalize_desktop(std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().as_str());
        Self::from_os_release(&os, desktop, &arch)
    }

    pub fn is_macos(&self) -> bool {
        self.distro == "macos"
    }

    pub fn from_release_parts(
        distro: String,
        upstream: String,
        distro_codename: String,
        base_codename: String,
        desktop: String,
        arch: &str,
    ) -> Result<Self> {
        let architecture = if distro == "macos" {
            match arch {
                "aarch64" | "arm64" => Architecture::DarwinArm64,
                _ => bail!("unsupported macOS architecture {arch:?}; only Apple Silicon (arm64) is supported"),
            }
        } else {
            Architecture::normalize(arch)?
        };
        if distro == "debian" && !matches!(distro_codename.as_str(), "bookworm" | "trixie") {
            bail!("unsupported Debian release {distro_codename:?}; supported releases are bookworm and trixie");
        }
        Ok(Self {
            distro,
            upstream,
            distro_codename,
            base_codename,
            desktop: normalize_desktop(&desktop),
            architecture,
        })
    }

    fn from_os_release(os: &OsRelease, desktop: String, arch: &str) -> Result<Self> {
        let distro = os.id().to_owned();
        let upstream: String = upstream(&distro, os.get_value("ID_LIKE"))?.into();
        let distro_codename = os.version_codename().unwrap_or_default().to_owned();
        let base_codename = match upstream.as_str() {
            "ubuntu" => os.get_value("UBUNTU_CODENAME"),
            "debian" => os.get_value("DEBIAN_CODENAME"),
            _ => unreachable!(),
        }
        .unwrap_or(&distro_codename)
        .to_owned();
        Self::from_release_parts(distro, upstream, distro_codename, base_codename, desktop, arch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    Amd64,
    Arm64,
    Arm32,
    DarwinArm64,
}

impl Architecture {
    pub fn normalize(value: &str) -> Result<Self> {
        match value {
            "x86_64" | "amd64" => Ok(Self::Amd64),
            "aarch64" | "arm64" => Ok(Self::Arm64),
            "arm32" | "armv7" | "armv7l" | "armhf" => Ok(Self::Arm32),
            _ => bail!("unsupported architecture {value:?}; supported architectures: amd64, arm64, arm32"),
        }
    }

    pub fn canonical(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Arm32 => "arm32",
            Self::DarwinArm64 => "darwin-arm64",
        }
    }

    pub fn debian(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Arm32 => "armhf",
            Self::DarwinArm64 => "arm64",
        }
    }

    pub fn go(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Arm32 => "arm",
            Self::DarwinArm64 => "arm64",
        }
    }

    pub fn go_archive(self) -> &'static str {
        match self {
            Self::Arm32 => "armv6l",
            Self::DarwinArm64 => "arm64",
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

fn upstream(id: &str, id_like: Option<&str>) -> Result<&'static str> {
    match id {
        "ubuntu" | "pop" => Ok("ubuntu"),
        "debian" => Ok("debian"),
        "linuxmint" => {
            let mut families = id_like.unwrap_or_default().split_ascii_whitespace();
            let ubuntu = families.clone().any(|family| family == "ubuntu");
            let debian = families.any(|family| family == "debian");
            match (ubuntu, debian) {
                (true, _) => Ok("ubuntu"),
                (false, true) => Ok("debian"),
                _ => bail!(
                    "unsupported linuxmint base family in ID_LIKE {:?}; expected ubuntu or debian",
                    id_like.unwrap_or_default()
                ),
            }
        }
        _ => bail!("unsupported distro: {id}"),
    }
}

fn normalize_desktop(value: &str) -> String {
    value
        .split(':')
        .find_map(|token| {
            let token = token.to_ascii_lowercase();
            if token.contains("gnome") {
                Some("gnome")
            } else if token.contains("cinnamon") {
                Some("cinnamon")
            } else {
                None
            }
        })
        .unwrap_or("none")
        .into()
}
