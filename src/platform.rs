use anyhow::{bail, Context, Result};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub distro: String,
    pub upstream: String,
    pub codename: String,
    pub desktop: String,
    pub uname_arch: String,
    pub go_arch: String,
    pub linux_arch: String,
    pub x64_arch: String,
    pub arm64_suffix: String,
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
        let (uname_arch, go_arch, linux_arch, x64_arch, arm64_suffix) = match arch {
            "x86_64" => ("x86_64", "amd64", "amd64", "x64", ""),
            "aarch64" => ("aarch64", "arm64", "aarch64", "arm64", "-arm64"),
            _ => bail!("unsupported architecture: {arch}"),
        };
        Ok(Self {
            distro,
            upstream,
            codename,
            desktop,
            uname_arch: uname_arch.into(),
            go_arch: go_arch.into(),
            linux_arch: linux_arch.into(),
            x64_arch: x64_arch.into(),
            arm64_suffix: arm64_suffix.into(),
        })
    }
    pub fn expand(&self, input: &str) -> String {
        [
            ("UPSTREAM_DISTRO", &self.upstream),
            ("VERSION_CODENAME", &self.codename),
            ("UNAME_ARCH", &self.uname_arch),
            ("GO_ARCH", &self.go_arch),
            ("LINUX_ARCH", &self.linux_arch),
            ("X64_ARCH", &self.x64_arch),
            ("ARM64_SUFFIX", &self.arm64_suffix),
        ]
        .into_iter()
        .fold(input.to_owned(), |s, (k, v)| {
            s.replace(&format!("${{{k}}}"), v)
                .replace(&format!("${k}"), v)
        })
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
    fn mappings() {
        let p = Platform::from_parts(
            "ubuntu".into(),
            "ubuntu".into(),
            "noble".into(),
            "gnome".into(),
            "aarch64",
        )
        .unwrap();
        assert_eq!(
            (
                p.go_arch.as_str(),
                p.x64_arch.as_str(),
                p.arm64_suffix.as_str()
            ),
            ("arm64", "arm64", "-arm64")
        );
    }
    #[test]
    fn distro_map() {
        assert_eq!(upstream("linuxmint").unwrap(), "ubuntu");
        assert!(upstream("arch").is_err());
    }
}
