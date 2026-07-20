use std::process::Command;

use anyhow::{bail, Context, Result};
use etc_os_release::OsRelease;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    Amd64,
    Arm64,
    Arm32,
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
        }
    }

    pub fn debian(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Arm32 => "armhf",
        }
    }

    pub fn go(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Arm32 => "arm",
        }
    }

    pub fn go_archive(self) -> &'static str {
        match self {
            Self::Arm32 => "armv6l",
            other => other.go(),
        }
    }

    pub fn rust_target(self) -> &'static str {
        match self {
            Self::Amd64 => "x86_64-unknown-linux-gnu",
            Self::Arm64 => "aarch64-unknown-linux-gnu",
            Self::Arm32 => "armv7-unknown-linux-gnueabihf",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub distro: String,
    pub upstream: String,
    pub distro_codename: String,
    pub base_codename: String,
    pub desktop: String,
    pub architecture: Architecture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAptSources {
    pub distro: String,
    pub release: String,
    pub architecture: Architecture,
    pub components: Vec<String>,
    pub stanzas: Vec<ManagedAptStanza>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAptStanza {
    pub uri: String,
    pub suites: Vec<String>,
    pub signed_by: String,
}

impl ManagedAptSources {
    pub fn render_deb822(&self) -> String {
        let components = self.components.join(" ");
        self.stanzas
            .iter()
            .map(|stanza| {
                format!(
                    "Types: deb\nURIs: {}\nSuites: {}\nComponents: {}\nArchitectures: {}\nSigned-By: {}\n",
                    stanza.uri,
                    stanza.suites.join(" "),
                    components,
                    self.architecture.debian(),
                    stanza.signed_by,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Platform {
    pub fn detect() -> Result<Self> {
        let os = OsRelease::open().context("read os-release")?;
        let uname = Command::new("uname").arg("-m").output().context("run uname -m")?;
        let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
        let desktop = desktop(std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().as_str());
        Self::from_os_release(&os, desktop, &arch)
    }

    pub fn from_release_parts(
        distro: String,
        upstream: String,
        distro_codename: String,
        base_codename: String,
        desktop: String,
        arch: &str,
    ) -> Result<Self> {
        let architecture = Architecture::normalize(arch)?;
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

    pub fn managed_apt_sources(&self, configured_components: &[&str]) -> Result<ManagedAptSources> {
        if !matches!(self.distro.as_str(), "ubuntu" | "debian" | "kali") {
            bail!("system.apt.sources: managed is unsupported for distribution {:?}; use preserve", self.distro);
        }
        let components = managed_components(self, configured_components)?;
        let architecture = self.architecture;
        if matches!(self.distro.as_str(), "ubuntu" | "debian")
            && (self.distro_codename.is_empty()
                || !self.distro_codename.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || index != 0 && matches!(byte, b'.' | b'_' | b'+' | b'-')
                }))
        {
            bail!("system.apt.sources: managed requires a valid platform codename");
        }
        let (release, stanzas) = match self.distro.as_str() {
            "ubuntu" => {
                let release = self.distro_codename.as_str();
                if !matches!(release, "jammy" | "noble" | "questing" | "resolute") {
                    bail!(
                        "system.apt.sources: managed Ubuntu release {:?} is unsupported; supported releases are jammy, noble, questing, and resolute",
                        release
                    );
                }
                let main_archive =
                    architecture == Architecture::Amd64 || architecture == Architecture::Arm64 && release == "resolute";
                let keyring = "/usr/share/keyrings/ubuntu-archive-keyring.gpg";
                let stanzas = if main_archive {
                    vec![
                        ManagedAptStanza {
                            uri: "https://archive.ubuntu.com/ubuntu".into(),
                            suites: vec![release.into(), format!("{release}-updates"), format!("{release}-backports")],
                            signed_by: keyring.into(),
                        },
                        ManagedAptStanza {
                            uri: "https://security.ubuntu.com/ubuntu".into(),
                            suites: vec![format!("{release}-security")],
                            signed_by: keyring.into(),
                        },
                    ]
                } else {
                    vec![ManagedAptStanza {
                        uri: "https://ports.ubuntu.com/ubuntu-ports".into(),
                        suites: vec![
                            release.into(),
                            format!("{release}-updates"),
                            format!("{release}-backports"),
                            format!("{release}-security"),
                        ],
                        signed_by: keyring.into(),
                    }]
                };
                (release.to_owned(), stanzas)
            }
            "debian" => {
                let release = self.distro_codename.as_str();
                if !matches!(release, "bullseye" | "bookworm" | "trixie") {
                    bail!(
                        "system.apt.sources: managed Debian release {:?} is unsupported; supported releases are bullseye, bookworm, and trixie",
                        release
                    );
                }
                let keyring = "/usr/share/keyrings/debian-archive-keyring.gpg";
                (
                    release.to_owned(),
                    vec![
                        ManagedAptStanza {
                            uri: "https://deb.debian.org/debian".into(),
                            suites: vec![release.into(), format!("{release}-updates")],
                            signed_by: keyring.into(),
                        },
                        ManagedAptStanza {
                            uri: "https://security.debian.org/debian-security".into(),
                            suites: vec![format!("{release}-security")],
                            signed_by: keyring.into(),
                        },
                    ],
                )
            }
            "kali" => (
                "kali-rolling".into(),
                vec![ManagedAptStanza {
                    uri: "https://http.kali.org/kali".into(),
                    suites: vec!["kali-rolling".into()],
                    signed_by: "/usr/share/keyrings/kali-archive-keyring.gpg".into(),
                }],
            ),
            _ => unreachable!(),
        };
        Ok(ManagedAptSources { distro: self.distro.clone(), release, architecture, components, stanzas })
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
        "ubuntu" | "pop" | "zorin" => Ok("ubuntu"),
        "debian" | "kali" | "tails" | "deepin" => Ok("debian"),
        "linuxmint" => {
            let families = id_like.unwrap_or_default().split_ascii_whitespace();
            let mut ubuntu = false;
            let mut debian = false;
            for family in families {
                ubuntu |= family == "ubuntu";
                debian |= family == "debian";
            }
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

fn managed_components(platform: &Platform, configured: &[&str]) -> Result<Vec<String>> {
    let defaults: &[&str] =
        if platform.distro == "kali" { &["main", "contrib", "non-free", "non-free-firmware"] } else { &["main"] };
    let components = if configured.is_empty() { defaults } else { configured };
    let supported: &[&str] = match platform.distro.as_str() {
        "ubuntu" => &["main", "restricted", "universe", "multiverse"],
        "debian" if platform.distro_codename == "bullseye" => &["main", "contrib", "non-free"],
        "debian" => &["main", "contrib", "non-free", "non-free-firmware"],
        "kali" => &["main", "contrib", "non-free", "non-free-firmware"],
        _ => &[],
    };
    let mut result = Vec::new();
    for (index, component) in components.iter().enumerate() {
        if !supported.contains(component) {
            bail!(
                "system.apt.components[{index}]: component {component:?} is unsupported for {} {}",
                platform.distro,
                platform.distro_codename
            );
        }
        if result.iter().any(|existing| existing == component) {
            bail!("system.apt.components: duplicate component {component:?}");
        }
        result.push((*component).to_owned());
    }
    Ok(result)
}

fn desktop(s: &str) -> String {
    normalize_desktop(s)
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
