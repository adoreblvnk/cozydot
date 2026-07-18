use std::{fs, io::ErrorKind, path::Path, process::Command};

use anyhow::{bail, Context, Result};

use self::os_release::OsRelease;

mod os_release;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    Amd64,
    Arm64,
    Arm32,
    Riscv64,
}

impl Architecture {
    pub fn normalize(value: &str) -> Result<Self> {
        match value {
            "x86_64" | "amd64" => Ok(Self::Amd64),
            "aarch64" | "arm64" => Ok(Self::Arm64),
            "arm32" | "armv7" | "armv7l" | "armhf" => Ok(Self::Arm32),
            "riscv64" => Ok(Self::Riscv64),
            _ => bail!(
                "unsupported architecture {value:?}; supported architectures: amd64, arm64, arm32, riscv64"
            ),
        }
    }

    pub fn canonical(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Arm32 => "arm32",
            Self::Riscv64 => "riscv64",
        }
    }

    pub fn debian(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Arm32 => "armhf",
            Self::Riscv64 => "riscv64",
        }
    }

    pub fn go(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
            Self::Arm32 => "arm",
            Self::Riscv64 => "riscv64",
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
            Self::Riscv64 => "riscv64gc-unknown-linux-gnu",
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
        let os = read_system_os_release()?;
        let uname = Command::new("uname")
            .arg("-m")
            .output()
            .context("run uname -m")?;
        let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
        let desktop = desktop(
            std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .as_str(),
        );
        Self::from_os_release(&os, desktop, &arch)
    }

    pub fn from_parts(
        distro: String,
        upstream: String,
        codename: String,
        desktop: String,
        arch: &str,
    ) -> Result<Self> {
        Self::from_release_parts(distro, upstream, codename.clone(), codename, desktop, arch)
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
        let distro = os.get("ID").unwrap_or_default().to_owned();
        let upstream: String = upstream(&distro, os.get("ID_LIKE"))?.into();
        let distro_codename = os.get("VERSION_CODENAME").unwrap_or_default().to_owned();
        let base_codename = match upstream.as_str() {
            "ubuntu" => {
                extra_codename(os, "UBUNTU_CODENAME").unwrap_or_else(|| distro_codename.clone())
            }
            "debian" => {
                extra_codename(os, "DEBIAN_CODENAME").unwrap_or_else(|| distro_codename.clone())
            }
            _ => unreachable!(),
        };
        Self::from_release_parts(
            distro,
            upstream,
            distro_codename,
            base_codename,
            desktop,
            arch,
        )
    }

    pub fn managed_apt_sources(&self, configured_components: &[&str]) -> Result<ManagedAptSources> {
        if !matches!(self.distro.as_str(), "ubuntu" | "debian" | "kali") {
            bail!(
                "system.apt.sources: managed is unsupported for distribution {:?}; use preserve",
                self.distro
            );
        }
        let components = managed_components(self, configured_components)?;
        let architecture = self.architecture;
        if matches!(self.distro.as_str(), "ubuntu" | "debian")
            && (self.distro_codename.is_empty()
                || !self
                    .distro_codename
                    .bytes()
                    .enumerate()
                    .all(|(index, byte)| {
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
                let main_archive = architecture == Architecture::Amd64
                    || architecture == Architecture::Arm64 && release == "resolute";
                let keyring = "/usr/share/keyrings/ubuntu-archive-keyring.gpg";
                let stanzas = if main_archive {
                    vec![
                        ManagedAptStanza {
                            uri: "https://archive.ubuntu.com/ubuntu".into(),
                            suites: vec![
                                release.into(),
                                format!("{release}-updates"),
                                format!("{release}-backports"),
                            ],
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
                if architecture == Architecture::Riscv64 && release != "trixie" {
                    bail!(
                        "system.apt.sources: Debian {release} does not support architecture riscv64"
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
            "kali" => {
                if architecture == Architecture::Riscv64 {
                    bail!("system.apt.sources: Kali rolling does not support architecture riscv64");
                }
                (
                    "kali-rolling".into(),
                    vec![ManagedAptStanza {
                        uri: "https://http.kali.org/kali".into(),
                        suites: vec!["kali-rolling".into()],
                        signed_by: "/usr/share/keyrings/kali-archive-keyring.gpg".into(),
                    }],
                )
            }
            _ => unreachable!(),
        };
        Ok(ManagedAptSources {
            distro: self.distro.clone(),
            release,
            architecture,
            components,
            stanzas,
        })
    }
}
fn parse_uname_machine(success: bool, stdout: &[u8]) -> Result<String> {
    if !success {
        bail!("uname -m failed");
    }
    let machine = std::str::from_utf8(stdout)
        .context("uname -m output is not UTF-8")?
        .trim();
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
    let defaults: &[&str] = if platform.distro == "kali" {
        &["main", "contrib", "non-free", "non-free-firmware"]
    } else {
        &["main"]
    };
    let components = if configured.is_empty() {
        defaults
    } else {
        configured
    };
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

fn read_os_release(path: &Path) -> Result<OsRelease> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read os-release at {}", path.display()))?;
    parse_os_release(path, &text)
}

fn parse_os_release(path: &Path, text: &str) -> Result<OsRelease> {
    OsRelease::parse(text).with_context(|| format!("parse os-release at {}", path.display()))
}

fn read_system_os_release() -> Result<OsRelease> {
    read_system_os_release_from(
        Path::new("/etc/os-release"),
        Path::new("/usr/lib/os-release"),
    )
}

fn read_system_os_release_from(etc_path: &Path, usr_path: &Path) -> Result<OsRelease> {
    match fs::read_to_string(etc_path) {
        Ok(text) => parse_os_release(etc_path, &text),
        Err(error) if error.kind() == ErrorKind::NotFound => read_os_release(usr_path),
        Err(error) => {
            Err(error).with_context(|| format!("read os-release at {}", etc_path.display()))
        }
    }
}

fn extra_codename(os: &OsRelease, key: &str) -> Option<String> {
    os.get(key).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_host_labels() {
        for (input, expected) in [
            ("x86_64", Architecture::Amd64),
            ("amd64", Architecture::Amd64),
            ("aarch64", Architecture::Arm64),
            ("arm64", Architecture::Arm64),
            ("arm32", Architecture::Arm32),
            ("armv7", Architecture::Arm32),
            ("armv7l", Architecture::Arm32),
            ("armhf", Architecture::Arm32),
            ("riscv64", Architecture::Riscv64),
        ] {
            assert_eq!(Architecture::normalize(input).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn translates_ecosystem_architectures() {
        let cases = [
            (
                Architecture::Amd64,
                "amd64",
                "amd64",
                "amd64",
                "amd64",
                "x86_64-unknown-linux-gnu",
            ),
            (
                Architecture::Arm64,
                "arm64",
                "arm64",
                "arm64",
                "arm64",
                "aarch64-unknown-linux-gnu",
            ),
            (
                Architecture::Arm32,
                "arm32",
                "armhf",
                "arm",
                "armv6l",
                "armv7-unknown-linux-gnueabihf",
            ),
            (
                Architecture::Riscv64,
                "riscv64",
                "riscv64",
                "riscv64",
                "riscv64",
                "riscv64gc-unknown-linux-gnu",
            ),
        ];
        for (architecture, canonical, debian, go, go_archive, rust_target) in cases {
            assert_eq!(architecture.canonical(), canonical);
            assert_eq!(architecture.debian(), debian);
            assert_eq!(architecture.go(), go);
            assert_eq!(architecture.go_archive(), go_archive);
            assert_eq!(architecture.rust_target(), rust_target);
        }
    }

    #[test]
    fn parses_trimmed_uname_machine_output() {
        assert_eq!(parse_uname_machine(true, b" armv7l\n").unwrap(), "armv7l");
    }

    #[test]
    fn rejects_failed_empty_or_non_utf8_uname_output() {
        assert_eq!(
            parse_uname_machine(false, b"x86_64\n")
                .unwrap_err()
                .to_string(),
            "uname -m failed"
        );
        assert_eq!(
            parse_uname_machine(true, b" \n").unwrap_err().to_string(),
            "uname -m returned an empty machine architecture"
        );
        assert_eq!(
            parse_uname_machine(true, &[0xff]).unwrap_err().to_string(),
            "uname -m output is not UTF-8"
        );
    }

    #[test]
    fn rejects_unknown_architectures_with_supported_values() {
        let error = Architecture::normalize("sparc64").unwrap_err().to_string();
        assert_eq!(
            error,
            "unsupported architecture \"sparc64\"; supported architectures: amd64, arm64, arm32, riscv64"
        );
    }

    #[test]
    fn rejects_armv6_as_arm32() {
        let error = Architecture::normalize("armv6l").unwrap_err().to_string();
        assert_eq!(
            error,
            "unsupported architecture \"armv6l\"; supported architectures: amd64, arm64, arm32, riscv64"
        );
    }

    #[test]
    fn rejects_release_only_aliases_as_host_labels() {
        assert!(Architecture::normalize("x64").is_err());
        assert!(Architecture::normalize("riscv64gc").is_err());
        assert!(Architecture::normalize("arm").is_err());
    }

    #[test]
    fn normalizes_desktop_to_supported_canonical_values() {
        for (input, expected) in [
            ("gnome", "gnome"),
            ("GNOME", "gnome"),
            ("ubuntu:GNOME", "gnome"),
            ("X-Cinnamon", "cinnamon"),
            ("plasma:X-Cinnamon:GNOME", "cinnamon"),
            ("unknown:ubuntu-GNOME:X-Cinnamon", "gnome"),
            ("KDE", "none"),
            ("plasma", "none"),
            ("arbitrary text", "none"),
            ("", "none"),
        ] {
            assert_eq!(normalize_desktop(input), expected, "{input:?}");
        }
    }

    #[test]
    fn from_parts_enforces_canonical_desktop_invariant() {
        let platform = Platform::from_parts(
            "ubuntu".into(),
            "ubuntu".into(),
            "noble".into(),
            "KDE:ubuntu:GNOME".into(),
            "amd64",
        )
        .unwrap();
        assert_eq!(platform.desktop, "gnome");
    }

    #[test]
    fn distro_map() {
        assert_eq!(
            upstream("linuxmint", Some("ubuntu debian")).unwrap(),
            "ubuntu"
        );
        assert_eq!(upstream("linuxmint", Some("debian")).unwrap(), "debian");
        assert_eq!(upstream("deepin", Some("debian")).unwrap(), "debian");
        assert!(upstream("linuxmint", None).is_err());
        assert!(upstream("arch", None).is_err());
    }

    #[test]
    fn os_release_file_preserves_distribution_and_base_codenames() {
        let detected = |contents: &str| {
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), contents).unwrap();
            let os = read_os_release(file.path()).unwrap();
            Platform::from_os_release(&os, "none".into(), "amd64").unwrap()
        };
        let mint = detected(
            "ID=linuxmint\nID_LIKE=\"debian ubuntu\"\nVERSION_CODENAME=wilma\nUBUNTU_CODENAME=\"noble\"\n",
        );
        assert_eq!(mint.upstream, "ubuntu");
        assert_eq!(mint.distro_codename, "wilma");
        assert_eq!(mint.base_codename, "noble");

        let lmde = detected(
            "ID=linuxmint\nID_LIKE=debian\nVERSION_CODENAME=\"gigi\"\nDEBIAN_CODENAME=\"bookworm\"\n",
        );
        assert_eq!(lmde.upstream, "debian");
        assert_eq!(lmde.distro_codename, "gigi");
        assert_eq!(lmde.base_codename, "bookworm");

        let deepin = detected("ID=deepin\nID_LIKE=debian\nVERSION_CODENAME=crimson\n");
        assert_eq!(deepin.upstream, "debian");
        assert_eq!(deepin.distro_codename, "crimson");
        assert_eq!(deepin.base_codename, "crimson");
    }

    #[test]
    fn os_release_file_requires_a_supported_linux_mint_id_like() {
        for id_like in [None, Some("arch"), Some("ubuntuish debianish")] {
            let file = tempfile::NamedTempFile::new().unwrap();
            let contents = format!(
                "ID=linuxmint\nVERSION_CODENAME=wilma\n{}",
                id_like
                    .map(|value| format!("ID_LIKE=\"{value}\"\n"))
                    .unwrap_or_default()
            );
            std::fs::write(file.path(), contents).unwrap();
            let os = read_os_release(file.path()).unwrap();
            assert!(Platform::from_os_release(&os, "none".into(), "amd64").is_err());
        }
    }

    #[test]
    fn os_release_read_errors_keep_context() {
        let directory = tempfile::tempdir().unwrap();
        let error = read_os_release(&directory.path().join("missing-os-release")).unwrap_err();
        assert!(error.to_string().starts_with("read os-release at "));
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn os_release_falls_back_to_usr_only_when_etc_is_missing() {
        let directory = tempfile::tempdir().unwrap();
        let etc = directory.path().join("etc-os-release");
        let usr = directory.path().join("usr-os-release");
        std::fs::write(&usr, "ID=debian\nVERSION_CODENAME=bookworm\n").unwrap();

        let release = read_system_os_release_from(&etc, &usr).unwrap();
        assert_eq!(release.get("ID"), Some("debian"));

        std::fs::write(&etc, "ID=ubuntu\nVERSION_CODENAME=noble\n").unwrap();
        let release = read_system_os_release_from(&etc, &usr).unwrap();
        assert_eq!(release.get("ID"), Some("ubuntu"));

        std::fs::write(&etc, "malformed\n").unwrap();
        let error = read_system_os_release_from(&etc, &usr).unwrap_err();
        assert!(error
            .to_string()
            .starts_with(&format!("parse os-release at {}", etc.display())));
    }

    #[test]
    fn os_release_rejects_malformed_utf8_between_valid_lines() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"ID=ubuntu\n\xff\nVERSION_CODENAME=noble\n").unwrap();

        let error = read_os_release(file.path()).unwrap_err();
        assert!(error.to_string().starts_with("read os-release at "));
        assert!(format!("{error:#}").contains("stream did not contain valid UTF-8"));
    }

    #[test]
    fn managed_apt_release_architecture_and_component_tables_are_strict() {
        let platform = |distro: &str, release: &str, architecture: &str| {
            Platform::from_parts(
                distro.into(),
                if distro == "ubuntu" {
                    "ubuntu"
                } else {
                    "debian"
                }
                .into(),
                release.into(),
                "none".into(),
                architecture,
            )
            .unwrap()
        };

        let noble_arm64 = platform("ubuntu", "noble", "arm64")
            .managed_apt_sources(&["main"])
            .unwrap();
        assert_eq!(noble_arm64.stanzas.len(), 1);
        assert_eq!(
            noble_arm64.stanzas[0].uri,
            "https://ports.ubuntu.com/ubuntu-ports"
        );
        let resolute_arm64 = platform("ubuntu", "resolute", "arm64")
            .managed_apt_sources(&["main", "universe"])
            .unwrap();
        assert_eq!(resolute_arm64.stanzas.len(), 2);
        assert_eq!(
            resolute_arm64.stanzas[0].uri,
            "https://archive.ubuntu.com/ubuntu"
        );
        assert!(resolute_arm64
            .render_deb822()
            .contains("Architectures: arm64"));

        assert!(platform("debian", "bookworm", "riscv64")
            .managed_apt_sources(&["main"])
            .unwrap_err()
            .to_string()
            .contains("does not support architecture riscv64"));
        assert!(platform("debian", "bullseye", "amd64")
            .managed_apt_sources(&["non-free-firmware"])
            .unwrap_err()
            .to_string()
            .contains("unsupported"));
        assert!(platform("debian", "trixie", "riscv64")
            .managed_apt_sources(&["main", "non-free-firmware"])
            .is_ok());
        assert!(platform("kali", "kali-rolling", "riscv64")
            .managed_apt_sources(&[])
            .unwrap_err()
            .to_string()
            .contains("does not support architecture riscv64"));
        assert!(platform("ubuntu", "plucky", "amd64")
            .managed_apt_sources(&["main"])
            .unwrap_err()
            .to_string()
            .contains("unsupported"));
    }
}
