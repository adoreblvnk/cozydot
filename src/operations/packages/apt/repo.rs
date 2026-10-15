use crate::config::AptRepoConfig;
use crate::operations::host::{self, privileged_file, temp_path};
use crate::platform::Arch;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

const SOURCES_DIR: &str = "/etc/apt/sources.list.d";

pub struct AptRepo {
    key_url: String,
    source_uri: String,
    arch: Arch,
    suite: String,
    components: Vec<String>,
    key_path: PathBuf,
    source_list_path: PathBuf,
}

impl AptRepo {
    pub fn new(config: &AptRepoConfig, source_uri: String, suite: String, arch: Arch) -> Self {
        let source_list_path = PathBuf::from(format!("{SOURCES_DIR}/{}.sources", config.name));
        Self {
            key_url: config.key_url.clone(),
            source_uri,
            arch,
            suite,
            components: config.components.clone(),
            key_path: PathBuf::from(&config.key_path),
            source_list_path,
        }
    }
    pub fn render_source(&self) -> String {
        format!(
            "Types: deb\nURIs: {}\nSuites: {}\nComponents: {}\nArchitectures: {}\nSigned-By: {}\n",
            self.source_uri,
            self.suite,
            self.components.join(" "),
            self.arch.debian(),
            self.key_path.display(),
        )
    }
}

pub(crate) fn add(repo: &AptRepo) -> Result<()> {
    // .asc destinations retain armor while .gpg destinations receive binary keyrings
    let preserve_armor = repo.key_path.extension() == Some(OsStr::new("asc"));

    let key = processed_key(&repo.key_url, preserve_armor)?;
    let source = repo.render_source().into_bytes();

    privileged_file::write_atomic(&repo.key_path, &key, "APT repo key write")?;
    privileged_file::write_atomic(&repo.source_list_path, &source, "APT repo source write")
}
fn processed_key(url: &str, preserve_armor: bool) -> Result<Vec<u8>> {
    let downloaded = temp_path("repo-key-download", "")?;
    let downloaded_path = downloaded.to_str().context("downloaded repo key path is not UTF-8")?;
    host::curl("repo key download", url, ["--tlsv1.2", "--output", downloaded_path])?;

    let downloaded_bytes = fs::read(&downloaded).context("read downloaded repo key")?;

    let binary_keyring = temp_path("repo-key-binary", ".gpg")?;
    let key_path = binary_keyring.to_str().context("binary keyring path is not UTF-8")?;
    let args = ["--no-options", "--batch", "--yes", "--output", key_path, "--dearmor", downloaded_path];
    host::run("repo key conversion", "gpg", args)?;

    // parsing proves download contains a public key; configured URL remains identity trust boundary
    let no_default = "--no-default-keyring";
    let args = ["--no-options", "--batch", no_default, "--keyring", key_path, "--with-colons", "--list-keys"];
    let key_list = host::run("repo key validation", "gpg", args)?;
    let mut lines = key_list.stdout.split(|byte| *byte == b'\n');
    let public_key = lines.any(|line| line.strip_prefix(b"pub:").is_some_and(|fields| !fields.is_empty()));
    ensure!(public_key, "repo key validation found no public key");

    if preserve_armor { Ok(downloaded_bytes) } else { fs::read(&binary_keyring).context("read dearmored repo key") }
}

pub(crate) mod debian_components {
    use super::*;

    const DEB822_SOURCE: &str = "/etc/apt/sources.list.d/debian.sources";
    const LEGACY_SOURCE: &str = "/etc/apt/sources.list";
    const COMPONENTS: [&str; 3] = ["contrib", "non-free", "non-free-firmware"];

    pub(crate) fn add(codename: &str) -> Result<()> {
        for directory in ["/etc/apt", "/etc/apt/sources.list.d"] {
            let metadata = fs::symlink_metadata(directory).with_context(|| format!("read {directory} metadata"))?;
            ensure!(!metadata.is_symlink() && metadata.is_dir(), "APT directory must be a real directory: {directory}");
        }

        if !Path::new(DEB822_SOURCE).exists() && Path::new(LEGACY_SOURCE).exists() {
            let content = format!(
                "Types: deb\n\
                 URIs: http://deb.debian.org/debian\n\
                 Suites: {codename} {codename}-updates\n\
                 Components: main contrib non-free non-free-firmware\n\
                 Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg\n\n\
                 Types: deb\n\
                 URIs: http://security.debian.org/debian-security\n\
                 Suites: {codename}-security\n\
                 Components: main contrib non-free non-free-firmware\n\
                 Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg\n"
            );
            privileged_file::write_atomic(Path::new(DEB822_SOURCE), content.as_bytes(), "Debian APT deb822 migration")?;
            privileged_file::write_atomic(
                Path::new(LEGACY_SOURCE),
                b"# Migrated to /etc/apt/sources.list.d/debian.sources by cozydot\n",
                "Debian APT legacy source truncate",
            )?;
            return Ok(());
        }

        let metadata = fs::symlink_metadata(DEB822_SOURCE).with_context(|| format!("stat {DEB822_SOURCE}"))?;
        ensure!(!metadata.is_symlink(), "Debian APT source path is a symlink: {DEB822_SOURCE}");
        ensure!(metadata.is_file(), "Debian APT source path is not a regular file: {DEB822_SOURCE}");

        let original = fs::read_to_string(DEB822_SOURCE).with_context(|| format!("read {DEB822_SOURCE}"))?;
        let replacement = add_deb822_components(&original);
        if replacement == original {
            return Ok(());
        }
        // catch changes since first read before replacing file; this isn't a lock
        ensure!(fs::read_to_string(DEB822_SOURCE)? == original, "Debian APT source changed concurrently before write");
        privileged_file::write_atomic(Path::new(DEB822_SOURCE), replacement.as_bytes(), "Debian APT component write")?;
        Ok(())
    }

    fn add_deb822_components(text: &str) -> String {
        let mut replacement = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let Some((name, values)) = body.split_once(':') else {
                replacement.push_str(line);
                continue;
            };
            if !name.eq_ignore_ascii_case("Components") {
                replacement.push_str(line);
                continue;
            }
            let values = values.split_ascii_whitespace().collect::<Vec<_>>();
            if !values.contains(&"main") {
                replacement.push_str(line);
                continue;
            }
            let line = append_missing(body, body.len(), &values, line.ends_with('\n'));
            replacement.push_str(&line);
        }
        replacement
    }

    fn append_missing(body: &str, end: usize, existing: &[&str], newline: bool) -> String {
        let missing = COMPONENTS.into_iter().filter(|component| !existing.contains(component)).collect::<Vec<_>>();
        if missing.is_empty() {
            return format!("{body}{}", if newline { "\n" } else { "" });
        }
        let active_end = body[..end].trim_end().len();
        let newline = if newline { "\n" } else { "" };
        format!("{} {}{}{}", &body[..active_end], missing.join(" "), &body[active_end..], newline)
    }
}
