use anyhow::{bail, Context, Result};
use std::{collections::BTreeSet, ffi::OsStr, fs, path::Path};
use url::Url;

use super::{Host, TempDir, TempPath};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NerdFontsMode {
    EnsurePresent,
    Update,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NerdFontsOperation {
    families: Vec<String>,
    mode: NerdFontsMode,
}

impl NerdFontsOperation {
    pub fn new(families: Vec<String>, mode: NerdFontsMode) -> Result<Self> {
        validate_families(&families)?;
        Ok(Self { families, mode })
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        [
            "nerd-fonts".into(),
            match self.mode {
                NerdFontsMode::EnsurePresent => "ensure-present".into(),
                NerdFontsMode::Update => "update".into(),
            },
        ]
        .into_iter()
        .chain(self.families.iter().cloned())
        .collect()
    }
}

pub(crate) fn execute(host: &Host<'_>, operation: &NerdFontsOperation) -> Result<()> {
    validate_families(&operation.families).context("validate Nerd Fonts operation")?;
    for family in &operation.families {
        if operation.mode == NerdFontsMode::Update || !font_present(host, family)? {
            install_family(host, family)?;
        }
    }
    Ok(())
}

fn install_family(host: &Host<'_>, family: &str) -> Result<()> {
    let data_home = host
        .value("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"));
    if !data_home.is_absolute() {
        bail!("Nerd Fonts XDG data directory must be absolute");
    }
    let parent = data_home.join("fonts/cozydot");
    fs::create_dir_all(&parent).context("create Nerd Fonts destination directory")?;
    let destination = parent.join(family);
    let archive = TempPath::new_with_suffix(host, "nerd-font", ".tar.xz")?;
    let mut url = Url::parse(
        "https://github.com/ryanoasis/nerd-fonts/releases/latest/download/placeholder.tar.xz",
    )?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Nerd Fonts URL cannot be a base"))?
        .pop()
        .push(&format!("{family}.tar.xz"));
    host.require(
        "Nerd Font archive download",
        "curl",
        [
            "--proto".as_ref(),
            "=https".as_ref(),
            "--location".as_ref(),
            "--fail".as_ref(),
            "--silent".as_ref(),
            "--show-error".as_ref(),
            "--retry".as_ref(),
            "3".as_ref(),
            "--retry-all-errors".as_ref(),
            "--output".as_ref(),
            archive.path().as_os_str(),
            "--".as_ref(),
            url.as_str().as_ref(),
        ],
    )?;
    let listing = host.require(
        "Nerd Font archive preflight",
        "tar",
        [
            "--list",
            "--xz",
            "--file",
            &archive.path().to_string_lossy(),
        ],
    )?;
    validate_archive_listing(&listing.stdout)?;
    let stage = TempDir::new_in(&data_home, ".cozydot-font-stage")?;
    host.require(
        "Nerd Font archive extraction",
        "tar",
        [
            "--extract",
            "--xz",
            "--directory",
            &stage.path().to_string_lossy(),
            "--file",
            &archive.path().to_string_lossy(),
        ],
    )?;
    validate_extracted_tree(stage.path())?;
    let replacing = match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_dir() => true,
        Ok(_) => bail!(
            "Nerd Font destination conflict at {}",
            destination.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("inspect Nerd Font destination"),
    };
    publish_family(stage.path(), &destination, replacing)?;
    let postcondition = refresh_and_verify(host, family, stage.path(), &destination);
    if let Err(error) = postcondition {
        rollback_family(stage.path(), &destination, replacing)
            .with_context(|| format!("Nerd Font mutation failed and rollback failed: {error:#}"))?;
        refresh_cache(host, "Nerd Font rollback cache refresh", &parent)
            .with_context(|| format!("Nerd Font mutation failed: {error:#}"))?;
        return Err(error);
    }
    Ok(())
}

fn publish_family(stage: &Path, destination: &Path, replacing: bool) -> Result<()> {
    let flags = if replacing {
        rustix::fs::RenameFlags::EXCHANGE
    } else {
        rustix::fs::RenameFlags::NOREPLACE
    };
    rustix::fs::renameat_with(rustix::fs::CWD, stage, rustix::fs::CWD, destination, flags)
        .context("atomically publish Nerd Font family")
}

fn rollback_family(stage: &Path, destination: &Path, replacing: bool) -> Result<()> {
    let flags = if replacing {
        rustix::fs::RenameFlags::EXCHANGE
    } else {
        rustix::fs::RenameFlags::NOREPLACE
    };
    rustix::fs::renameat_with(rustix::fs::CWD, destination, rustix::fs::CWD, stage, flags)
        .context("atomically restore previous Nerd Font family")?;
    sync_publication_directories(stage, destination)
}

fn refresh_and_verify(
    host: &Host<'_>,
    family: &str,
    stage: &Path,
    destination: &Path,
) -> Result<()> {
    sync_publication_directories(stage, destination)?;
    refresh_cache(
        host,
        "Nerd Font cache refresh",
        destination
            .parent()
            .context("Nerd Font destination has no parent")?,
    )?;
    if !font_present(host, family)? {
        bail!("Nerd Font mutation did not publish family {family:?}");
    }
    Ok(())
}

fn refresh_cache(host: &Host<'_>, operation: &str, directory: &Path) -> Result<()> {
    host.require(
        operation,
        "fc-cache",
        [OsStr::new("--force"), directory.as_os_str()],
    )?;
    Ok(())
}

fn sync_publication_directories(stage: &Path, destination: &Path) -> Result<()> {
    let stage_parent = stage.parent().context("Nerd Font stage has no parent")?;
    let destination_parent = destination
        .parent()
        .context("Nerd Font destination has no parent")?;
    fs::File::open(stage_parent)?
        .sync_all()
        .context("sync Nerd Font staging directory")?;
    fs::File::open(destination_parent)?
        .sync_all()
        .context("sync Nerd Font destination directory")
}

fn font_present(host: &Host<'_>, family: &str) -> Result<bool> {
    let expected = format!("{family} Nerd Font");
    let pattern = format!(":family={expected}");
    let output = host.require(
        "Nerd Font state query",
        "fc-list",
        ["--format=%{family}\\n", "--", &pattern],
    )?;
    let output =
        std::str::from_utf8(&output.stdout).context("fc-list returned non-UTF-8 font state")?;
    if output
        .chars()
        .any(|character| character == '\r' || character == '\0')
    {
        bail!("fc-list returned malformed font state");
    }
    Ok(output
        .lines()
        .flat_map(|line| line.split(','))
        .any(|installed| installed == expected))
}

fn validate_archive_listing(output: &[u8]) -> Result<()> {
    let output = std::str::from_utf8(output).context("Nerd Font archive listing is not UTF-8")?;
    if output.is_empty() {
        bail!("Nerd Font archive is empty");
    }
    for entry in output.lines() {
        let path = Path::new(entry);
        if entry.is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
            || entry.chars().any(char::is_control)
        {
            bail!("Nerd Font archive contains an unsafe path");
        }
    }
    Ok(())
}

fn validate_extracted_tree(root: &Path) -> Result<()> {
    let mut directories = vec![root.to_path_buf()];
    let mut fonts = 0_u32;
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_dir() {
                directories.push(path);
            } else if metadata.file_type().is_file() {
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                if matches!(extension, "ttf" | "otf") && metadata.len() > 0 {
                    fonts += 1;
                }
            } else {
                bail!(
                    "Nerd Font archive contains an unsupported file type at {}",
                    path.display()
                );
            }
        }
    }
    if fonts == 0 {
        bail!("Nerd Font archive contains no non-empty TTF or OTF files");
    }
    Ok(())
}

fn validate_families(families: &[String]) -> Result<()> {
    if families.is_empty() {
        bail!("Nerd Font family sequence must not be empty");
    }
    let mut seen = BTreeSet::new();
    for family in families {
        let bytes = family.as_bytes();
        if bytes
            .first()
            .is_none_or(|byte| !byte.is_ascii_alphanumeric())
            || bytes
                .last()
                .is_none_or(|byte| !byte.is_ascii_alphanumeric())
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(byte))
        {
            bail!("invalid Nerd Font family name {family:?}");
        }
        if !seen.insert(family) {
            bail!("duplicate Nerd Font family {family:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_listing_rejects_absolute_parent_and_empty_paths() {
        validate_archive_listing(b"LICENSE\nGeistMonoNerdFont-Regular.ttf\n").unwrap();
        for listing in [
            b"".as_slice(),
            b"/tmp/font.ttf\n".as_slice(),
            b"../font.ttf\n".as_slice(),
            b"dir/../../font.ttf\n".as_slice(),
        ] {
            assert!(validate_archive_listing(listing).is_err());
        }
    }

    #[test]
    fn family_names_use_the_frozen_definition_grammar() {
        validate_families(&["GeistMono".into(), "JetBrains-Mono_2.0".into()]).unwrap();
        for family in [
            "",
            ".Hidden",
            "Trailing-",
            "Has Space",
            "Cascadia+Code",
            "dir/font",
            "NerdFont!",
            "Unicode-λ",
        ] {
            assert!(validate_families(&[family.into()]).is_err(), "{family:?}");
        }
        assert!(validate_families(&["GeistMono".into(), "GeistMono".into()]).is_err());
    }
}
