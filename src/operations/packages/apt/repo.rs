use crate::operations::host::{self, privileged_file, temp_path, temp_path_with_suffix};
use crate::platform::Arch;
use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, fs, path::PathBuf};

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
    pub fn new(
        name: impl Into<String>,
        key_url: String,
        source_uri: String,
        arch: Arch,
        suite: String,
        components: Vec<String>,
        key_path: PathBuf,
    ) -> Self {
        let name = name.into();
        let source_list_path = PathBuf::from(format!("{SOURCES_DIR}/{name}.list"));
        Self { key_url, source_uri, arch, suite, components, key_path, source_list_path }
    }

    pub fn render_source(&self) -> String {
        let arch = self.arch.debian();
        let key_path = self.key_path.display();
        let prefix = format!("deb [arch={arch} signed-by={key_path}] {} ", self.source_uri);
        format!("{prefix}{} {}\n", self.suite, self.components.join(" "))
    }
}

pub(crate) fn add(repo: &AptRepo) -> Result<()> {
    let preserve_armor = repo.key_path.extension() == Some(OsStr::new("asc"));

    let key = processed_key(&repo.key_url, preserve_armor)?;
    let source = repo.render_source().into_bytes();

    privileged_file::write_atomic(&repo.key_path, &key, "APT repo key write")?;
    privileged_file::write_atomic(&repo.source_list_path, &source, "APT repo source write")?;

    Ok(())
}

fn processed_key(url: &str, preserve_armor: bool) -> Result<Vec<u8>> {
    let downloaded = temp_path("repo-key-download")?;
    host::curl("repo key download", url, ["--tlsv1.2", "--output", &downloaded.to_string_lossy()])?;

    let downloaded_bytes = fs::read(&downloaded).context("read downloaded repo key")?;

    let binary_keyring = temp_path_with_suffix("repo-key-binary", ".gpg")?;

    let key = binary_keyring.to_string_lossy();
    let input = downloaded.to_string_lossy();
    let args = ["--no-options", "--batch", "--yes", "--output", key.as_ref(), "--dearmor", input.as_ref()];
    host::run("repo key conversion", "gpg", args)?;

    // parsing proves download contains a public key; configured URL remains identity trust boundary
    let key = binary_keyring.to_string_lossy();
    let no_default = "--no-default-keyring";
    let args = ["--no-options", "--batch", no_default, "--keyring", key.as_ref(), "--with-colons", "--list-keys"];
    let key_list = host::run("repo key validation", "gpg", args)?;
    let mut lines = key_list.stdout.split(|byte| *byte == b'\n');
    let public_key = lines.any(|line| line.strip_prefix(b"pub:").is_some_and(|fields| !fields.is_empty()));
    if !public_key {
        bail!("repo key validation found no public key");
    }

    if preserve_armor { Ok(downloaded_bytes) } else { fs::read(&binary_keyring).context("read dearmored repo key") }
}

pub(crate) mod debian_components {
    use crate::operations::host::{self, privileged_file};
    use anyhow::{Context, Result, bail};
    use std::{ffi::OsStr, path::Path};

    const DEB822_SOURCE: &str = "/etc/apt/sources.list.d/debian.sources";
    const ONE_LINE_SOURCE: &str = "/etc/apt/sources.list";
    const COMPONENTS: [&str; 3] = ["contrib", "non-free", "non-free-firmware"];

    pub(crate) fn add() -> Result<()> {
        for directory in ["/etc/apt", "/etc/apt/sources.list.d"] {
            host::run("Debian APT source directory symlink check", "sudo", ["test", "!", "-L", directory])?;
        }

        reject_symlink(DEB822_SOURCE)?;
        let deb822 = probe_regular(DEB822_SOURCE)?;
        if !deb822 {
            host::run("Debian APT deb822 source absence check", "sudo", ["test", "!", "-e", DEB822_SOURCE])?;
        }
        let source = if deb822 { DEB822_SOURCE } else { ONE_LINE_SOURCE };
        if !deb822 {
            reject_symlink(source)?;
            if !probe_regular(source)? {
                bail!("Debian APT source file does not exist: {source}");
            }
        }

        let original = read(source)?;
        let text = std::str::from_utf8(&original).context("Debian APT source is not UTF-8")?;
        let replacement = if deb822 { add_deb822_components(text) } else { add_one_line_components(text) };
        if replacement.as_bytes() == original {
            return Ok(());
        }
        // catch changes since first read before replacing file; this isn't a lock
        if read(source)? != original {
            bail!("Debian APT source changed concurrently before write");
        }
        privileged_file::write_atomic(Path::new(source), replacement.as_bytes(), "Debian APT component write")?;
        Ok(())
    }

    fn reject_symlink(path: &str) -> Result<()> {
        let output = host::output("sudo", ["test", "-L", path])?;
        match output.status.code() {
            Some(0) => bail!("Debian APT source path is a symlink: {path}"),
            Some(1) => Ok(()),
            _ => bail!("Debian APT source symlink check failed for {path}"),
        }
    }

    fn probe_regular(path: &str) -> Result<bool> {
        let output = host::output("sudo", ["test", "-f", path])?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => bail!("Debian APT source regular-file check failed for {path}"),
        }
    }

    fn read(path: &str) -> Result<Vec<u8>> {
        Ok(host::run("Debian APT source read", "sudo", [OsStr::new("cat"), OsStr::new(path)])?.stdout)
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

    fn add_one_line_components(text: &str) -> String {
        let mut replacement = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let comment = body.find('#').unwrap_or(body.len());
            let active = body[..comment].trim();
            let fields = active.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.first() != Some(&"deb") {
                replacement.push_str(line);
                continue;
            }
            let uri = fields.iter().position(|field| field.starts_with("http://") || field.starts_with("https://"));
            let Some(uri) = uri else {
                replacement.push_str(line);
                continue;
            };
            if !debian_uri(fields[uri]) || fields.len() <= uri + 2 || !fields[uri + 2..].contains(&"main") {
                replacement.push_str(line);
                continue;
            }
            let line = append_missing(body, comment, &fields[uri + 2..], line.ends_with('\n'));
            replacement.push_str(&line);
        }
        replacement
    }

    fn debian_uri(uri: &str) -> bool {
        let supported = [
            "http://deb.debian.org/debian",
            "https://deb.debian.org/debian",
            "http://deb.debian.org/debian-security",
            "https://deb.debian.org/debian-security",
            "http://security.debian.org/debian-security",
            "https://security.debian.org/debian-security",
        ];
        supported.contains(&uri.trim_end_matches('/'))
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
