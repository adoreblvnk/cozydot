use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};
use tar::{Archive, EntryType};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    File,
    Directory,
}

pub fn validate(path: &Path) -> Result<()> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    validate_file(&mut file)
}

pub(crate) fn validate_file(file: &mut File) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let file = file.try_clone()?;
    let mut archive = Archive::new(GzDecoder::new(file));
    let mut members = HashMap::<PathBuf, Kind>::new();
    let mut binary = 0;
    let mut config = 0;

    for entry in archive.entries().context("read release archive")? {
        let entry = entry.context("read release member")?;
        let raw = entry.path().context("read release member path")?;
        let normalized = safe_path(&raw)?;
        let kind = match entry.header().entry_type() {
            EntryType::Regular => Kind::File,
            EntryType::Directory => Kind::Directory,
            _ => bail!(
                "release contains a link or special file: {}",
                normalized.display()
            ),
        };
        if normalized != Path::new("cozydot")
            && normalized != Path::new("configs/default.yaml")
            && normalized != Path::new("configs")
            && normalized != Path::new("dotfiles")
            && !normalized.starts_with("dotfiles/")
        {
            bail!("unexpected release path: {}", normalized.display());
        }
        if members.insert(normalized.clone(), kind).is_some() {
            bail!("duplicate release path: {}", normalized.display());
        }
        for ancestor in normalized.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            if members.get(ancestor) == Some(&Kind::File) {
                bail!("release path collision: {}", normalized.display());
            }
        }
        if kind == Kind::File
            && members
                .keys()
                .any(|p| p.starts_with(&normalized) && p != &normalized)
        {
            bail!("release path collision: {}", normalized.display());
        }
        binary += usize::from(normalized == Path::new("cozydot") && kind == Kind::File);
        config +=
            usize::from(normalized == Path::new("configs/default.yaml") && kind == Kind::File);
    }
    if binary != 1 || config != 1 {
        bail!("release must contain exactly one cozydot and configs/default.yaml");
    }
    Ok(())
}

pub fn extract_assets(archive_path: &Path, destination: &Path) -> Result<()> {
    let mut file = File::open(archive_path)?;
    extract_assets_file(&mut file, destination)
}

pub(crate) fn extract_assets_file(file: &mut File, destination: &Path) -> Result<()> {
    validate_file(file)?;
    file.seek(SeekFrom::Start(0))?;
    let file = file.try_clone()?;
    let mut archive = Archive::new(GzDecoder::new(file));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = safe_path(&entry.path()?)?;
        if path == Path::new("configs/default.yaml") || path.starts_with("dotfiles/") {
            let output = destination.join(&path);
            let kind = entry.header().entry_type();
            if kind == EntryType::Directory {
                fs::create_dir_all(&output)?;
            } else if kind == EntryType::Regular {
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent)?;
                }
                entry.unpack(&output)?;
            } else {
                bail!(
                    "release member changed during extraction: {}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

pub fn extract_binary(archive_path: &Path, destination: &Path) -> Result<()> {
    let mut file = File::open(archive_path)?;
    extract_binary_file(&mut file, destination)
}

fn extract_binary_file(file: &mut File, destination: &Path) -> Result<()> {
    validate_file(file)?;
    file.seek(SeekFrom::Start(0))?;
    let file = file.try_clone()?;
    let mut archive = Archive::new(GzDecoder::new(file));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if safe_path(&entry.path()?)? == Path::new("cozydot") {
            if entry.header().entry_type() != EntryType::Regular {
                bail!("release binary changed during extraction");
            }
            entry.unpack(destination)?;
            return Ok(());
        }
    }
    bail!("release contains no cozydot binary")
}

fn safe_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe release path: {}", path.display());
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::{fs, io};
    use tar::{Builder, Header};

    fn archive(entries: &[(&str, EntryType)]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        let encoder = GzEncoder::new(file.reopen().unwrap(), Compression::default());
        let mut builder = Builder::new(encoder);
        for (name, kind) in entries {
            let mut header = Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(0o755);
            header.set_size(0);
            header.set_cksum();
            builder.append_data(&mut header, name, io::empty()).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        file
    }

    #[test]
    fn accepts_expected_flat_layout() {
        let file = archive(&[
            ("cozydot", EntryType::Regular),
            ("configs/default.yaml", EntryType::Regular),
            ("dotfiles/pkg/file", EntryType::Regular),
        ]);
        validate(file.path()).unwrap();
    }

    #[test]
    fn rejects_links_duplicates_collisions_and_missing_required_files() {
        let cases = [
            vec![
                ("cozydot", EntryType::Regular),
                ("configs/default.yaml", EntryType::Symlink),
            ],
            vec![
                ("cozydot", EntryType::Regular),
                ("cozydot", EntryType::Regular),
                ("configs/default.yaml", EntryType::Regular),
            ],
            vec![
                ("cozydot", EntryType::Regular),
                ("configs/default.yaml", EntryType::Regular),
                ("dotfiles/clash", EntryType::Regular),
                ("dotfiles/clash/child", EntryType::Regular),
            ],
            vec![("cozydot", EntryType::Regular)],
        ];
        for entries in cases {
            let file = archive(&entries);
            assert!(validate(file.path()).is_err());
        }
    }

    #[test]
    fn extraction_uses_the_opened_inode_after_path_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("release.tar.gz");
        let valid = archive(&[
            ("cozydot", EntryType::Regular),
            ("configs/default.yaml", EntryType::Regular),
        ]);
        fs::copy(valid.path(), &path).unwrap();
        let mut opened = File::open(&path).unwrap();
        let replacement = archive(&[
            ("cozydot", EntryType::Regular),
            ("configs/default.yaml", EntryType::Regular),
            ("unexpected", EntryType::Regular),
        ]);
        fs::rename(replacement.path(), &path).unwrap();

        extract_binary_file(&mut opened, &temp.path().join("cozydot")).unwrap();
        assert!(temp.path().join("cozydot").is_file());
        assert!(validate(&path).is_err());
    }
}
