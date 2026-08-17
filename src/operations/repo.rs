use super::{Host, TempPath};
use crate::platform::Architecture;
use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, fs, path::PathBuf};

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
    ) -> Self {
        let name = name.into();
        let source_list_path = PathBuf::from(format!("{SOURCES_DIRECTORY}/{name}.list"));
        Self { key_url, source_url, architecture, suite, components, key_path, source_list_path }
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
    let preserve_armor = repo.key_path.extension() == Some(OsStr::new("asc"));

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

    host.run(
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
    let key_list = host.run(
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
    if !key_list
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

    const ONELINE_SOURCE: &str = "/etc/apt/sources.list";
    const DEB822_SOURCE: &str = "/etc/apt/sources.list.d/debian.sources";
    const COMPONENTS: [&str; 3] = ["contrib", "non-free", "non-free-firmware"];

    // TODO: review this
    pub(crate) fn add(host: &Host) -> Result<()> {
        for directory in ["/etc/apt", "/etc/apt/sources.list.d"] {
            host.run("Debian APT source directory symlink check", "sudo", ["test", "!", "-L", directory])?;
        }

        reject_symlink(host, DEB822_SOURCE)?;
        let deb822 = probe_regular(host, DEB822_SOURCE)?;
        if !deb822 {
            host.run("Debian APT deb822 source absence check", "sudo", ["test", "!", "-e", DEB822_SOURCE])?;
        }
        let source = if deb822 { DEB822_SOURCE } else { ONELINE_SOURCE };
        if !deb822 {
            reject_symlink(host, source)?;
            if !probe_regular(host, source)? {
                bail!("Debian APT source file does not exist: {source}");
            }
        }

        let original = read(host, source)?;
        let text = std::str::from_utf8(&original).context("Debian APT source is not UTF-8")?;
        let replacement = if deb822 { add_deb822_components(text) } else { add_oneline_components(text) };
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
        let output = host.output("sudo", ["test", "-f", path])?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => bail!("Debian APT source regular-file check failed for {path}"),
        }
    }

    fn reject_symlink(host: &Host, path: &str) -> Result<()> {
        let output = host.output("sudo", ["test", "-L", path])?;
        match output.status.code() {
            Some(0) => bail!("Debian APT source path is a symlink: {path}"),
            Some(1) => Ok(()),
            _ => bail!("Debian APT source symlink check failed for {path}"),
        }
    }

    fn read(host: &Host, path: &str) -> Result<Vec<u8>> {
        Ok(host.run("Debian APT source read", "sudo", [OsStr::new("cat"), OsStr::new("--"), OsStr::new(path)])?.stdout)
    }

    fn add_oneline_components(text: &str) -> String {
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
