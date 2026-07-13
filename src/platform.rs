use anyhow::{bail, Context, Result};
use std::{collections::BTreeMap, fs, path::Path};

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
            "x86_64" | "amd64" | "x64" => Ok(Self::Amd64),
            "aarch64" | "arm64" => Ok(Self::Arm64),
            "arm32" | "armv6l" | "armv7" | "armv7l" | "armhf" => Ok(Self::Arm32),
            "riscv64" | "riscv64gc" => Ok(Self::Riscv64),
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

    pub fn release_asset_aliases(self) -> &'static [&'static str] {
        match self {
            Self::Amd64 => &["amd64", "x86_64", "x64"],
            Self::Arm64 => &["arm64", "aarch64"],
            Self::Arm32 => &["arm32", "armv7", "armv7l", "armhf"],
            Self::Riscv64 => &["riscv64", "riscv64gc"],
        }
    }

    pub fn uname(self) -> &'static str {
        match self {
            Self::Amd64 => "x86_64",
            Self::Arm64 => "aarch64",
            Self::Arm32 => "armv7l",
            Self::Riscv64 => "riscv64",
        }
    }

    fn linux_release(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "aarch64",
            Self::Arm32 => "armv7l",
            Self::Riscv64 => "riscv64",
        }
    }

    fn x64_release(self) -> &'static str {
        match self {
            Self::Amd64 => "x64",
            other => other.canonical(),
        }
    }

    fn arm64_suffix(self) -> &'static str {
        if self == Self::Arm64 {
            "-arm64"
        } else {
            ""
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub distro: String,
    pub upstream: String,
    pub codename: String,
    pub desktop: String,
    pub architecture: Architecture,
}
impl Platform {
    pub fn detect(config_distro: &str, config_desktop: &str) -> Result<Self> {
        let os = parse_os_release(Path::new("/etc/os-release"))?;
        let distro = if config_distro == "auto" {
            os.get("ID").cloned().unwrap_or_default()
        } else {
            config_distro.into()
        };
        let upstream = upstream(&distro)?.into();
        let codename = os
            .get("UBUNTU_CODENAME")
            .or_else(|| os.get("VERSION_CODENAME"))
            .cloned()
            .unwrap_or_default();
        let desktop = if config_desktop == "auto" {
            desktop(
                std::env::var("XDG_CURRENT_DESKTOP")
                    .unwrap_or_default()
                    .as_str(),
            )
        } else {
            config_desktop.into()
        };
        Self::from_parts(distro, upstream, codename, desktop, std::env::consts::ARCH)
    }
    pub fn from_parts(
        distro: String,
        upstream: String,
        codename: String,
        desktop: String,
        arch: &str,
    ) -> Result<Self> {
        let architecture = Architecture::normalize(arch)?;
        Ok(Self {
            distro,
            upstream,
            codename,
            desktop,
            architecture,
        })
    }
    pub fn expand(&self, input: &str) -> String {
        let architecture = self.architecture;
        [
            ("UPSTREAM_DISTRO", self.upstream.as_str()),
            ("VERSION_CODENAME", self.codename.as_str()),
            ("UNAME_ARCH", architecture.uname()),
            ("GO_ARCH", architecture.go_archive()),
            ("LINUX_ARCH", architecture.linux_release()),
            ("X64_ARCH", architecture.x64_release()),
            ("ARM64_SUFFIX", architecture.arm64_suffix()),
        ]
        .into_iter()
        .fold(input.to_owned(), |s, (k, v)| {
            s.replace(&format!("${{{k}}}"), v)
                .replace(&format!("${k}"), v)
        })
    }

    pub fn expand_shell_arch(&self, input: &str) -> String {
        self.expand(input)
            .replace("$(dpkg --print-architecture)", self.architecture.debian())
    }
}
fn upstream(id: &str) -> Result<&'static str> {
    match id {
        "ubuntu" | "linuxmint" | "pop" | "zorin" | "Deepin" => Ok("ubuntu"),
        "debian" | "kali" | "tails" => Ok("debian"),
        _ => bail!("unsupported distro: {id}"),
    }
}
fn desktop(s: &str) -> String {
    if s.contains("GNOME") {
        "gnome"
    } else if s.contains("Cinnamon") {
        "cinnamon"
    } else if s.is_empty() {
        "none"
    } else {
        s
    }
    .into()
}
fn parse_os_release(path: &Path) -> Result<BTreeMap<String, String>> {
    let text = fs::read_to_string(path).context("read os-release")?;
    Ok(text
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.into(), v.trim_matches('"').into()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_host_and_release_aliases() {
        for (input, expected) in [
            ("x86_64", Architecture::Amd64),
            ("amd64", Architecture::Amd64),
            ("x64", Architecture::Amd64),
            ("aarch64", Architecture::Arm64),
            ("arm64", Architecture::Arm64),
            ("arm32", Architecture::Arm32),
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
    fn exposes_common_release_asset_aliases() {
        let cases: &[(Architecture, &[&str])] = &[
            (Architecture::Amd64, &["amd64", "x86_64", "x64"]),
            (Architecture::Arm64, &["arm64", "aarch64"]),
            (Architecture::Arm32, &["arm32", "armv7", "armv7l", "armhf"]),
            (Architecture::Riscv64, &["riscv64", "riscv64gc"]),
        ];
        for &(architecture, aliases) in cases {
            assert_eq!(architecture.release_asset_aliases(), aliases);
            for alias in architecture.release_asset_aliases() {
                assert_eq!(
                    Architecture::normalize(alias).unwrap(),
                    architecture,
                    "{alias}"
                );
            }
        }
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
    fn distro_map() {
        assert_eq!(upstream("linuxmint").unwrap(), "ubuntu");
        assert!(upstream("arch").is_err());
    }
}
