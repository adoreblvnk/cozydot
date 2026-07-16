use super::*;

fn data_artifact(host: &Host<'_>, operation: &BinaryPackageOperation) -> PathBuf {
    host.value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"))
        .join("cozydot/binaries")
        .join(format!("{}.AppImage", operation.name))
}
fn command_links(host: &Host<'_>, operation: &BinaryPackageOperation) -> Vec<PathBuf> {
    let root = host
        .value("XDG_BIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/bin"));
    operation
        .commands
        .iter()
        .map(|name| root.join(name))
        .collect()
}
pub(super) fn preflight_appimage(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    record: Option<&Record>,
) -> Result<()> {
    let artifact = data_artifact(host, operation);
    ensure_secure_data_parent(host, &artifact)?;
    ensure_secure_command_root(host)?;
    match fs::symlink_metadata(&artifact) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) if record.is_some_and(|record| record_owns_artifact(&artifact, record)) => {}
        Ok(_) => bail!(
            "binary AppImage artifact conflict at {}",
            artifact.display()
        ),
        Err(error) => return Err(error.into()),
    }
    for (command, link) in operation
        .commands
        .iter()
        .zip(command_links(host, operation))
    {
        let retry_may_own_link = record.is_some_and(|record| match record.status {
            Status::PendingInitial | Status::PendingUpdate => {
                record.declaration.commands.contains(command)
            }
            Status::Completed => record.declaration.commands.contains(command),
        });
        preflight_link(&link, &artifact, !retry_may_own_link)?;
    }
    if let Some(previous) = record.and_then(|r| {
        if r.status == Status::Completed {
            Some(Previous {
                declaration: r.declaration.clone(),
                resolved: r.resolved.clone(),
            })
        } else {
            r.previous.clone()
        }
    }) {
        for command in previous
            .declaration
            .commands
            .iter()
            .filter(|name| !operation.commands.contains(name))
        {
            let link = command_links_for(host, std::slice::from_ref(command))
                .pop()
                .unwrap();
            preflight_stale_link(
                &link,
                &artifact,
                record.is_some_and(|record| record.status == Status::PendingUpdate),
            )?;
        }
    }
    Ok(())
}
pub(super) fn install_appimage(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    resolved: &Resolved,
    downloaded: Option<&Downloaded>,
    previous: Option<&Previous>,
    retrying_update: bool,
) -> Result<()> {
    let artifact = data_artifact(host, operation);
    ensure_secure_data_parent(host, &artifact)?;
    if !artifact_matches(host, operation, resolved)? {
        let source = downloaded
            .context("AppImage publication requires staged bytes")?
            .temporary
            .path();
        require_elf(source, &operation.name)?;
        let destination_absent = match fs::symlink_metadata(&artifact) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Ok(_)
                if previous.is_some_and(|previous| {
                    previous.declaration.format == BinaryPackageFormat::AppImage
                        && valid_artifact(&artifact, &previous.resolved.actual_sha256)
                            .unwrap_or(false)
                }) =>
            {
                false
            }
            Ok(_) | Err(_) => bail!(
                "binary AppImage artifact ownership changed before publication at {}",
                artifact.display()
            ),
        };
        publish_artifact(source, &artifact, previous.is_none() || destination_absent)?;
    }
    let links = command_links(host, operation);
    for link in &links {
        publish_link(link, &artifact)?;
    }
    verify_appimage(&artifact, &links, resolved)?;
    if let Some(previous) = previous {
        for command in previous
            .declaration
            .commands
            .iter()
            .filter(|name| !operation.commands.contains(name))
        {
            let link = command_links_for(host, std::slice::from_ref(command))
                .pop()
                .unwrap();
            match fs::symlink_metadata(&link) {
                Ok(_) if managed_link(&link, &artifact) => {
                    fs::remove_file(&link).with_context(|| {
                        format!("remove stale owned command {}", link.display())
                    })?;
                    if fs::symlink_metadata(&link).is_ok() {
                        bail!("stale binary command removal failed");
                    }
                }
                Err(error) if retrying_update && error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) | Err(_) => bail!(
                    "stale owned binary command at {} was changed or is missing",
                    link.display()
                ),
            }
        }
    }
    Ok(())
}
fn command_links_for(host: &Host<'_>, commands: &[String]) -> Vec<PathBuf> {
    let root = host
        .value("XDG_BIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/bin"));
    commands.iter().map(|name| root.join(name)).collect()
}
fn ensure_secure_data_parent(host: &Host<'_>, artifact: &Path) -> Result<()> {
    let data = host
        .value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"));
    if !data.is_absolute() {
        bail!("binary data directory must be absolute");
    }
    let existed = fs::symlink_metadata(&data).is_ok();
    fs::create_dir_all(&data)?;
    if !existed {
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700))?;
    }
    validate_owned_directory(&data)?;
    let cozy = data.join("cozydot");
    create_owned_directory(&cozy)?;
    let binaries = cozy.join("binaries");
    create_owned_directory(&binaries)?;
    if artifact.parent() != Some(binaries.as_path()) {
        bail!("binary artifact path escaped managed directory");
    }
    Ok(())
}
fn ensure_secure_command_root(host: &Host<'_>) -> Result<()> {
    let root = host
        .value("XDG_BIN_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/bin"));
    if !root.is_absolute() {
        bail!("binary command directory must be absolute");
    }
    let existed = fs::symlink_metadata(&root).is_ok();
    fs::create_dir_all(&root).context("create binary command directory")?;
    if !existed {
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    validate_owned_directory(&root)
        .context("binary command directory has unsafe type, owner, or permissions")
}
fn create_owned_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    validate_owned_directory(path)
}
fn validate_owned_directory(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_dir()
        || m.uid() != rustix::process::geteuid().as_raw()
        || m.permissions().mode() & 0o022 != 0
    {
        bail!("binary managed data directory has unsafe type, owner, or permissions");
    }
    Ok(())
}
fn publish_artifact(source: &Path, destination: &Path, no_replace: bool) -> Result<()> {
    let parent = destination.parent().unwrap();
    let mut source_file = fs::File::open(source)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    std::io::copy(&mut source_file, staged.as_file_mut())?;
    staged
        .as_file_mut()
        .set_permissions(fs::Permissions::from_mode(0o755))?;
    staged.as_file_mut().sync_all()?;
    if no_replace {
        let path = staged.into_temp_path();
        fs::hard_link(&path, destination)
            .context("publish initial binary artifact without replacement")?;
        fs::remove_file(&path)?;
    } else {
        staged
            .into_temp_path()
            .persist(destination)
            .map_err(|e| e.error)?;
    }
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn preflight_link(link: &Path, artifact: &Path, require_absent: bool) -> Result<()> {
    match fs::symlink_metadata(link) {
        Ok(_) if require_absent => {
            bail!("binary AppImage command conflict at {}", link.display())
        }
        Ok(m) if m.file_type().is_symlink() && managed_link(link, artifact) => Ok(()),
        Ok(_) => bail!("binary AppImage command conflict at {}", link.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn preflight_stale_link(link: &Path, artifact: &Path, allow_absent: bool) -> Result<()> {
    match fs::symlink_metadata(link) {
        Ok(_) if managed_link(link, artifact) => Ok(()),
        Err(error) if allow_absent && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) | Err(_) => bail!(
            "stale owned binary command at {} was changed or is missing",
            link.display()
        ),
    }
}
fn publish_link(link: &Path, artifact: &Path) -> Result<()> {
    if managed_link(link, artifact) {
        return Ok(());
    }
    fs::create_dir_all(link.parent().unwrap())?;
    match symlink(artifact, link) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && managed_link(link, artifact) => {
            Ok(())
        }
        Err(e) => Err(e).context("publish binary AppImage command link"),
    }
}
fn managed_link(link: &Path, artifact: &Path) -> bool {
    fs::symlink_metadata(link).is_ok_and(|m| m.file_type().is_symlink())
        && fs::read_link(link).is_ok_and(|target| target == artifact)
}
fn verify_appimage(artifact: &Path, links: &[PathBuf], resolved: &Resolved) -> Result<()> {
    if !valid_artifact(artifact, &resolved.actual_sha256)?
        || links.iter().any(|link| !managed_link(link, artifact))
    {
        bail!("binary AppImage verification failed");
    }
    Ok(())
}
pub(super) fn artifact_matches(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    resolved: &Resolved,
) -> Result<bool> {
    let _ = host;
    valid_artifact(&data_artifact(host, operation), &resolved.actual_sha256)
}
fn record_owns_artifact(artifact: &Path, record: &Record) -> bool {
    record.declaration.format == BinaryPackageFormat::AppImage
        && valid_artifact(artifact, &record.resolved.actual_sha256).unwrap_or(false)
        || record.previous.as_ref().is_some_and(|previous| {
            previous.declaration.format == BinaryPackageFormat::AppImage
                && valid_artifact(artifact, &previous.resolved.actual_sha256).unwrap_or(false)
        })
}
fn valid_artifact(path: &Path, digest: &str) -> Result<bool> {
    let m = match fs::symlink_metadata(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    Ok(m.file_type().is_file()
        && m.uid() == rustix::process::geteuid().as_raw()
        && m.nlink() == 1
        && m.len() > 0
        && m.permissions().mode() & 0o7777 == 0o755
        && has_elf_magic(path)
        && sha256_file(path)? == BinarySha256::parse(digest)?.0)
}
pub(super) fn postconditions(
    host: &Host<'_>,
    operation: &BinaryPackageOperation,
    resolved: &Resolved,
) -> Result<bool> {
    match operation.format {
        BinaryPackageFormat::Deb => Ok(operation
            .commands
            .iter()
            .all(|name| executable_on_path(host, name))),
        BinaryPackageFormat::AppImage => {
            let artifact = data_artifact(host, operation);
            Ok(valid_artifact(&artifact, &resolved.actual_sha256)?
                && command_links(host, operation)
                    .iter()
                    .all(|link| managed_link(link, &artifact)))
        }
    }
}
pub(super) fn verify_commands(host: &Host<'_>, operation: &BinaryPackageOperation) -> Result<()> {
    let missing = operation
        .commands
        .iter()
        .filter(|name| !executable_on_path(host, name))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!("binary package installed but commands remain unavailable: {missing:?}");
    }
    Ok(())
}
fn executable_on_path(host: &Host<'_>, name: &str) -> bool {
    host.value("PATH")
        .and_then(|path| {
            std::env::split_paths(&path).find(|dir| {
                fs::metadata(dir.join(name))
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
        })
        .is_some()
}

pub(super) fn require_elf(path: &Path, name: &str) -> Result<()> {
    if !has_elf_magic(path) {
        bail!("binary package {name:?} AppImage does not have ELF magic");
    }
    Ok(())
}
fn has_elf_magic(path: &Path) -> bool {
    let mut magic = [0; 4];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == *b"\x7fELF"
}
