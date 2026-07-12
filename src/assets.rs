use crate::archive;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use tempfile::{NamedTempFile, TempDir};

struct VerifiedArchive {
    file: NamedTempFile,
}

impl VerifiedArchive {
    fn extract_assets(&mut self, destination: &Path) -> Result<()> {
        archive::extract_assets_file(self.file.as_file_mut(), destination)
    }
}

pub struct Assets {
    root: PathBuf,
    _temporary: Option<TempDir>,
}

impl Assets {
    pub fn discover() -> Result<Self> {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let executable = env::current_exe()?.canonicalize()?;
        if executable.starts_with(source.join("target"))
            && source.join("configs/default.yaml").is_file()
            && source.join("dotfiles").is_dir()
        {
            return Ok(Self {
                root: source,
                _temporary: None,
            });
        }
        let mut archive = cached_release()?;
        let cache = cache_dir()?;
        let temporary = tempfile::Builder::new()
            .prefix(".bundle.")
            .tempdir_in(cache)?;
        archive.extract_assets(temporary.path())?;
        Ok(Self {
            root: temporary.path().to_path_buf(),
            _temporary: Some(temporary),
        })
    }

    pub fn config(&self) -> PathBuf {
        self.root.join("configs/default.yaml")
    }
    pub fn dotfiles(&self) -> PathBuf {
        self.root.join("dotfiles")
    }
}

fn cached_release() -> Result<VerifiedArchive> {
    let arch = match env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => bail!("unsupported architecture: {other}"),
    };
    let version = env!("CARGO_PKG_VERSION");
    let asset = format!("cozydot-{version}-linux-{arch}.tar.gz");
    let cache = cache_dir()?;
    fs::create_dir_all(&cache)?;
    let archive = cache.join(&asset);
    let checksum = cache.join(format!("{asset}.sha256"));
    if let Some(verified) = stage_verified(&archive, &checksum, &cache)? {
        return Ok(verified);
    }
    remove_if_exists(&archive)?;
    remove_if_exists(&checksum)?;

    let base = env::var("COZYDOT_RELEASE_BASE_URL")
        .unwrap_or_else(|_| "https://github.com/adoreblvnk/cozydot/releases".into());
    let url = format!("{base}/download/v{version}");
    let archive_tmp = tempfile::Builder::new()
        .prefix(".archive.")
        .tempfile_in(&cache)?;
    let checksum_tmp = tempfile::Builder::new()
        .prefix(".checksum.")
        .tempfile_in(&cache)?;
    download(&format!("{url}/{asset}"), archive_tmp.path())?;
    download(&format!("{url}/{asset}.sha256"), checksum_tmp.path())?;
    let mut verified = verify_private(archive_tmp, checksum_tmp.path())?
        .context("release checksum verification failed")?;
    publish_archive(&mut verified, &archive)?;
    if env::var_os("COZYDOT_TEST_FAIL_CACHE_AFTER_ARCHIVE").is_some() {
        remove_if_exists(&archive)?;
        bail!("injected cache publication failure");
    }
    if let Err(error) = checksum_tmp.persist(&checksum) {
        remove_if_exists(&archive)?;
        return Err(error.error.into());
    }
    Ok(verified)
}

fn stage_verified(
    archive: &Path,
    checksum: &Path,
    cache: &Path,
) -> Result<Option<VerifiedArchive>> {
    if !archive.is_file() || !checksum.is_file() {
        return Ok(None);
    }
    let mut source = File::open(archive)?;
    let mut private = tempfile::Builder::new()
        .prefix(".verified.")
        .tempfile_in(cache)?;
    io::copy(&mut source, private.as_file_mut())?;
    private.as_file_mut().sync_all()?;
    verify_private(private, checksum)
}

fn verify_private(mut file: NamedTempFile, checksum: &Path) -> Result<Option<VerifiedArchive>> {
    let expected = match read_checksum(checksum)? {
        Some(expected) => expected,
        None => return Ok(None),
    };
    if hash_open_file(file.as_file_mut())? != expected {
        return Ok(None);
    }
    archive::validate_file(file.as_file_mut())?;
    file.as_file_mut().seek(SeekFrom::Start(0))?;
    Ok(Some(VerifiedArchive { file }))
}

fn publish_archive(verified: &mut VerifiedArchive, destination: &Path) -> Result<()> {
    verified.file.as_file_mut().seek(SeekFrom::Start(0))?;
    let mut output = tempfile::Builder::new().prefix(".publish.").tempfile_in(
        destination
            .parent()
            .context("cache archive has no parent")?,
    )?;
    io::copy(verified.file.as_file_mut(), output.as_file_mut())?;
    output.as_file_mut().sync_all()?;
    output.persist(destination).map_err(|error| error.error)?;
    verified.file.as_file_mut().seek(SeekFrom::Start(0))?;
    Ok(())
}

fn cache_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(path).join("cozydot"));
    }
    Ok(PathBuf::from(env::var_os("HOME").context("HOME is not set")?).join(".cache/cozydot"))
}

fn download(url: &str, path: &Path) -> Result<()> {
    if let Some(source) = url.strip_prefix("file://") {
        fs::copy(source, path).with_context(|| format!("copy {url}"))?;
        return Ok(());
    }
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("download {url}"))?;
    let mut output = File::create(path)?;
    const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
    let copied = io::copy(
        &mut response.body_mut().as_reader().take(MAX_DOWNLOAD + 1),
        &mut output,
    )?;
    if copied > MAX_DOWNLOAD {
        bail!("release download exceeds {MAX_DOWNLOAD} bytes");
    }
    output.sync_all()?;
    Ok(())
}

fn read_checksum(path: &Path) -> Result<Option<String>> {
    const MAX_CHECKSUM_TEXT: u64 = 4096;
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_CHECKSUM_TEXT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CHECKSUM_TEXT {
        return Ok(None);
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(_) => return Ok(None),
    };
    let mut fields = text.split_whitespace();
    let expected = fields.next().unwrap_or("");
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(None);
    }
    Ok(Some(expected.to_ascii_lowercase()))
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    hash_open_file(&mut file)
}

fn hash_open_file(file: &mut File) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    io::copy(file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    use tar::{Builder, Header};

    fn release(config: &[u8], dotfile: &[u8]) -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let encoder = GzEncoder::new(file.reopen().unwrap(), Compression::default());
        let mut builder = Builder::new(encoder);
        for (path, contents) in [
            ("cozydot", b"binary".as_slice()),
            ("configs/default.yaml", config),
            ("dotfiles/example", dotfile),
        ] {
            let mut header = Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(contents.len() as u64);
            header.set_cksum();
            builder.append_data(&mut header, path, contents).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        file
    }

    #[test]
    fn verified_archive_survives_checksum_invalid_cache_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let cache_archive = temp.path().join("release.tar.gz");
        let checksum = temp.path().join("release.tar.gz.sha256");
        let original = release(b"verified: true\n", b"verified\n");
        fs::copy(original.path(), &cache_archive).unwrap();
        writeln!(
            File::create(&checksum).unwrap(),
            "{}  release.tar.gz",
            hash_file(&cache_archive).unwrap()
        )
        .unwrap();

        let mut verified = stage_verified(&cache_archive, &checksum, temp.path())
            .unwrap()
            .unwrap();
        let replacement = release(b"attacker: true\n", b"attacker\n");
        fs::rename(replacement.path(), &cache_archive).unwrap();
        assert_ne!(
            hash_file(&cache_archive).unwrap(),
            read_checksum(&checksum).unwrap().unwrap()
        );

        let destination = temp.path().join("assets");
        verified.extract_assets(&destination).unwrap();
        assert_eq!(
            fs::read(destination.join("configs/default.yaml")).unwrap(),
            b"verified: true\n"
        );
        assert_eq!(
            fs::read(destination.join("dotfiles/example")).unwrap(),
            b"verified\n"
        );
    }

    #[test]
    fn rejects_oversized_and_malformed_checksum_text() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), vec![b'a'; 4097]).unwrap();
        assert!(read_checksum(file.path()).unwrap().is_none());
        fs::write(file.path(), b"not-a-sha256\n").unwrap();
        assert!(read_checksum(file.path()).unwrap().is_none());
    }
}
