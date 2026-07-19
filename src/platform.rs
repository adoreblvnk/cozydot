use std::{fs, io::ErrorKind, path::Path, process::Command};

use anyhow::{bail, Context, Result};

use self::os_release::OsRelease;

pub(crate) mod os_release {
    use anyhow::{bail, Context, Result};
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct OsRelease {
        fields: BTreeMap<String, String>,
    }

    impl OsRelease {
        pub(crate) fn parse(input: &str) -> Result<Self> {
            let mut fields = BTreeMap::new();
            for (index, line) in input.lines().enumerate() {
                let line_number = index + 1;
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let (key, value) = line
                    .split_once('=')
                    .with_context(|| format!("os-release line {line_number} is not KEY=VALUE"))?;
                validate_key(key).with_context(|| format!("invalid os-release key on line {line_number}"))?;
                let value =
                    parse_value(value).with_context(|| format!("invalid os-release value on line {line_number}"))?;
                // os-release(5) specifies that readers use the later assignment.
                fields.insert(key.to_owned(), value);
            }
            Ok(Self { fields })
        }

        pub(crate) fn get(&self, key: &str) -> Option<&str> {
            self.fields.get(key).map(String::as_str)
        }
    }

    fn validate_key(key: &str) -> Result<()> {
        let mut bytes = key.bytes();
        if !bytes.next().is_some_and(|byte| byte.is_ascii_uppercase())
            || !bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            bail!("keys must match [A-Z][A-Z0-9_]*");
        }
        Ok(())
    }

    fn parse_value(value: &str) -> Result<String> {
        if value.bytes().any(|byte| byte.is_ascii_control()) {
            bail!("control characters are not allowed");
        }
        match value.as_bytes().first().copied() {
            Some(b'\'') => parse_single_quoted(value),
            Some(b'"') => parse_double_quoted(value),
            _ => parse_unquoted(value),
        }
    }

    fn parse_single_quoted(value: &str) -> Result<String> {
        let body = value
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .context("unmatched single quote")?;
        if body.contains('\'') {
            bail!("single-quoted values cannot contain a single quote");
        }
        Ok(body.to_owned())
    }

    fn parse_double_quoted(value: &str) -> Result<String> {
        let body = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .context("unmatched double quote")?;
        let mut output = String::with_capacity(body.len());
        let mut chars = body.chars();
        while let Some(character) = chars.next() {
            match character {
                '\\' => {
                    let escaped = chars.next().context("dangling escape")?;
                    if matches!(escaped, '$' | '"' | '\\' | '`') {
                        output.push(escaped);
                    } else {
                        // POSIX double quotes preserve a backslash before other characters.
                        output.push('\\');
                        output.push(escaped);
                    }
                }
                '"' => bail!("unescaped double quote"),
                '$' | '`' => bail!("variable and command expansion are not supported"),
                _ => output.push(character),
            }
        }
        Ok(output)
    }

    fn parse_unquoted(value: &str) -> Result<String> {
        let mut output = String::with_capacity(value.len());
        let mut chars = value.chars();
        while let Some(character) = chars.next() {
            match character {
                '\\' => output.push(chars.next().context("dangling escape")?),
                '\'' | '"' => bail!("quoted and unquoted fragments cannot be concatenated"),
                '$' | '`' => bail!("variable and command expansion are not supported"),
                character
                    if character.is_ascii_whitespace()
                        || matches!(
                            character,
                            '|' | '&' | ';' | '(' | ')' | '<' | '>' | '*' | '?' | '[' | ']' | '{' | '}' | '~' | '#'
                        ) =>
                {
                    bail!("shell-special characters must be quoted or escaped")
                }
                _ => output.push(character),
            }
        }
        Ok(output)
    }
}

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
        let os = read_system_os_release()?;
        let uname = Command::new("uname").arg("-m").output().context("run uname -m")?;
        let arch = parse_uname_machine(uname.status.success(), &uname.stdout)?;
        let desktop = desktop(std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().as_str());
        Self::from_os_release(&os, desktop, &arch)
    }

    pub fn from_parts(distro: String, upstream: String, codename: String, desktop: String, arch: &str) -> Result<Self> {
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
            "ubuntu" => extra_codename(os, "UBUNTU_CODENAME").unwrap_or_else(|| distro_codename.clone()),
            "debian" => extra_codename(os, "DEBIAN_CODENAME").unwrap_or_else(|| distro_codename.clone()),
            _ => unreachable!(),
        };
        Self::from_release_parts(distro, upstream, distro_codename, base_codename, desktop, arch)
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

fn read_os_release(path: &Path) -> Result<OsRelease> {
    let text = fs::read_to_string(path).with_context(|| format!("read os-release at {}", path.display()))?;
    parse_os_release(path, &text)
}

fn parse_os_release(path: &Path, text: &str) -> Result<OsRelease> {
    OsRelease::parse(text).with_context(|| format!("parse os-release at {}", path.display()))
}

fn read_system_os_release() -> Result<OsRelease> {
    read_system_os_release_from(Path::new("/etc/os-release"), Path::new("/usr/lib/os-release"))
}

pub fn read_system_os_release_from(etc_path: &Path, usr_path: &Path) -> Result<OsRelease> {
    match fs::read_to_string(etc_path) {
        Ok(text) => parse_os_release(etc_path, &text),
        Err(error) if error.kind() == ErrorKind::NotFound => read_os_release(usr_path),
        Err(error) => Err(error).with_context(|| format!("read os-release at {}", etc_path.display())),
    }
}

fn extra_codename(os: &OsRelease, key: &str) -> Option<String> {
    os.get(key).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::Architecture;

    #[test]
    fn architecture_support_is_closed() {
        for (input, expected) in [
            ("x86_64", Architecture::Amd64),
            ("aarch64", Architecture::Arm64),
            ("armv7l", Architecture::Arm32),
        ] {
            assert_eq!(Architecture::normalize(input).unwrap(), expected);
        }
        for input in ["i686", "riscv64", "s390x"] {
            assert!(Architecture::normalize(input).is_err());
        }
    }
}
