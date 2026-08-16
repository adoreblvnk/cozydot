use super::{Host, TempPath};
use crate::{config::validate_repo_key_path, platform::Architecture};
use anyhow::{Context, Result, bail};
use std::{fs, path::PathBuf};

const SOURCES_DIRECTORY: &str = "/etc/apt/sources.list.d";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AptRepo {
    key_url: String,
    source_url: String,
    architecture: Architecture,
    suite: String,
    components: Vec<String>,
    key_path: PathBuf,
    source_list_path: PathBuf,
}

impl AptRepo {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        key_url: String,
        source_url: String,
        architecture: Architecture,
        suite: String,
        components: Vec<String>,
        key_path: PathBuf,
    ) -> Result<Self> {
        let name = name.into();
        validate_repo_key_path(&key_path)?;
        for value in std::iter::once(source_url.as_str())
            .chain(std::iter::once(suite.as_str()))
            .chain(components.iter().map(String::as_str))
        {
            if value.chars().any(char::is_control) {
                bail!("APT repo source values must fit on one line and contain no control characters");
            }
        }
        let source_list_path = PathBuf::from(format!("{SOURCES_DIRECTORY}/{name}.list"));
        Ok(Self { key_url, source_url, architecture, suite, components, key_path, source_list_path })
    }

    pub fn render_source(&self) -> String {
        let prefix = format!(
            "deb [arch={} signed-by={}] {} ",
            self.architecture.debian(),
            self.key_path.display(),
            self.source_url
        );
        format!("{prefix}{} {}\n", self.suite, self.components.join(" "))
    }
}

pub(crate) fn add(host: &Host, repo: &AptRepo) -> Result<()> {
    let key_path = repo.key_path.to_str().context("key path is not UTF-8")?;
    let preserve_armor = key_path.ends_with(".asc");

    let key = processed_key(host, &repo.key_url, preserve_armor)?;
    let source = repo.render_source().into_bytes();

    super::privileged_file::write_atomic(host, &repo.key_path, &key, "APT repo key write")?;
    super::privileged_file::write_atomic(host, &repo.source_list_path, &source, "APT repo source write")?;

    Ok(())
}

fn processed_key(host: &Host, url: &str, preserve_armor: bool) -> Result<Vec<u8>> {
    let downloaded = TempPath::new(host, "repo-key-download")?;
    host.curl("repo key download", url, ["--tlsv1.2", "--output", &downloaded.path().to_string_lossy()])?;

    let downloaded_bytes = fs::read(downloaded.path()).context("read downloaded repo key")?;
    if downloaded_bytes.is_empty() {
        bail!("repo key download produced empty output");
    }

    let binary_keyring = TempPath::new_with_suffix(host, "repo-key-binary", ".gpg")?;

    host.run_checked(
        "repo key conversion",
        "gpg",
        [
            "--no-options",
            "--batch",
            "--yes",
            "--output",
            &binary_keyring.path().to_string_lossy(),
            "--dearmor",
            &downloaded.path().to_string_lossy(),
        ],
    )?;

    // parsing proves download contains a public key; configured URL remains identity trust boundary
    let inspection = host.run_checked(
        "repo key validation",
        "gpg",
        [
            "--no-options",
            "--batch",
            "--no-default-keyring",
            "--keyring",
            &binary_keyring.path().to_string_lossy(),
            "--with-colons",
            "--list-keys",
        ],
    )?;
    if !inspection
        .stdout
        .split(|byte| *byte == b'\n')
        .any(|line| line.strip_prefix(b"pub:").is_some_and(|fields| !fields.is_empty()))
    {
        bail!("repo key validation found no public key");
    }

    if preserve_armor {
        Ok(downloaded_bytes)
    } else {
        let bytes = fs::read(binary_keyring.path()).context("read dearmored repo key")?;
        if bytes.is_empty() {
            bail!("repo key conversion produced empty output");
        }
        Ok(bytes)
    }
}

pub(crate) mod debian_components {
    use super::super::{Host, privileged_file};
    use anyhow::{Context, Result, bail};
    use std::{ffi::OsStr, path::Path};

    const LEGACY_SOURCE: &str = "/etc/apt/sources.list";
    const MODERN_SOURCE: &str = "/etc/apt/sources.list.d/debian.sources";
    const COMPONENTS: [&str; 3] = ["contrib", "non-free", "non-free-firmware"];

    pub(crate) fn add(host: &Host, codename: &str) -> Result<()> {
        if !matches!(codename, "bookworm" | "trixie") {
            bail!("unsupported Debian codename {codename:?}; supported codenames are bookworm and trixie");
        }
        for directory in ["/etc/apt", "/etc/apt/sources.list.d"] {
            host.run_checked("Debian APT source directory symlink check", "sudo", ["test", "!", "-L", directory])?;
        }

        reject_symlink(host, MODERN_SOURCE)?;
        let modern = probe_regular(host, MODERN_SOURCE)?;
        if !modern {
            host.run_checked("Debian APT modern source absence check", "sudo", ["test", "!", "-e", MODERN_SOURCE])?;
        }
        let source = if modern { MODERN_SOURCE } else { LEGACY_SOURCE };
        if !modern {
            reject_symlink(host, source)?;
            if !probe_regular(host, source)? {
                bail!("Debian APT source file does not exist: {source}");
            }
        }

        let original = read(host, source)?;
        let text = std::str::from_utf8(&original).context("Debian APT source is not UTF-8")?;
        let replacement = if modern { add_deb822_components(text) } else { add_list_components(text) };
        if replacement.as_bytes() == original {
            return Ok(());
        }
        // catch changes since first read before replacing file; this isn't a lock
        if read(host, source)? != original {
            bail!("Debian APT source changed concurrently before write");
        }
        privileged_file::write_atomic(host, Path::new(source), replacement.as_bytes(), "Debian APT component write")?;
        if read(host, source)? != replacement.as_bytes() {
            bail!("Debian APT component write did not establish the required postcondition");
        }
        Ok(())
    }

    fn probe_regular(host: &Host, path: &str) -> Result<bool> {
        let output = host.run("sudo", ["test", "-f", path])?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => bail!("Debian APT source file inspection failed for {path}"),
        }
    }

    fn reject_symlink(host: &Host, path: &str) -> Result<()> {
        let output = host.run("sudo", ["test", "-L", path])?;
        match output.status.code() {
            Some(0) => bail!("Debian APT source path is a symlink: {path}"),
            Some(1) => Ok(()),
            _ => bail!("Debian APT source symlink inspection failed for {path}"),
        }
    }

    fn read(host: &Host, path: &str) -> Result<Vec<u8>> {
        Ok(host
            .run_checked(
                "Debian APT source inspection",
                "sudo",
                [OsStr::new("cat"), OsStr::new("--"), OsStr::new(path)],
            )?
            .stdout)
    }

    fn add_list_components(text: &str) -> String {
        text.split_inclusive('\n')
            .map(|line| {
                let body = line.strip_suffix('\n').unwrap_or(line);
                let comment = body.find('#').unwrap_or(body.len());
                let active = body[..comment].trim();
                let fields = active.split_ascii_whitespace().collect::<Vec<_>>();
                if fields.first() != Some(&"deb") {
                    return line.to_owned();
                }
                let uri = fields.iter().position(|field| field.starts_with("http://") || field.starts_with("https://"));
                let Some(uri) = uri else { return line.to_owned() };
                if !debian_uri(fields[uri]) || fields.len() <= uri + 2 || !fields[uri + 2..].contains(&"main") {
                    return line.to_owned();
                }
                append_missing(body, comment, &fields[uri + 2..], line.ends_with('\n'))
            })
            .collect()
    }

    fn debian_uri(uri: &str) -> bool {
        [
            "http://deb.debian.org/debian",
            "https://deb.debian.org/debian",
            "http://deb.debian.org/debian-security",
            "https://deb.debian.org/debian-security",
            "http://security.debian.org/debian-security",
            "https://security.debian.org/debian-security",
        ]
        .contains(&uri.trim_end_matches('/'))
    }

    fn add_deb822_components(text: &str) -> String {
        text.split_inclusive('\n')
            .map(|line| {
                let body = line.strip_suffix('\n').unwrap_or(line);
                let Some((name, values)) = body.split_once(':') else { return line.to_owned() };
                if !name.eq_ignore_ascii_case("Components") {
                    return line.to_owned();
                }
                let values = values.split_ascii_whitespace().collect::<Vec<_>>();
                if !values.contains(&"main") {
                    return line.to_owned();
                }
                append_missing(body, body.len(), &values, line.ends_with('\n'))
            })
            .collect()
    }

    fn append_missing(body: &str, end: usize, existing: &[&str], newline: bool) -> String {
        let missing = COMPONENTS.into_iter().filter(|component| !existing.contains(component)).collect::<Vec<_>>();
        if missing.is_empty() {
            return format!("{body}{}", if newline { "\n" } else { "" });
        }
        let active_end = body[..end].trim_end().len();
        format!(
            "{} {}{}{}",
            &body[..active_end],
            missing.join(" "),
            &body[active_end..],
            if newline { "\n" } else { "" }
        )
    }
}
