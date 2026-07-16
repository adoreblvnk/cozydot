use super::Host;
use anyhow::{bail, Context, Result};
use serde::{
    de::{DeserializeOwned, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use std::{
    fmt,
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn parse_strict_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let strict: StrictValue = serde_json::from_slice(bytes)?;
    serde_json::from_value(strict.0).context("deserialize strict JSON value")
}

struct StrictValue(serde_json::Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(StrictValue(value.into()))
    }
    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(StrictValue(value.into()))
    }
    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(StrictValue(value.into()))
    }
    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(|number| StrictValue(number.into()))
            .ok_or_else(|| E::custom("invalid JSON number"))
    }
    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(StrictValue(value.into()))
    }
    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(StrictValue(value.into()))
    }
    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(StrictValue(serde_json::Value::Null))
    }
    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(StrictValue(serde_json::Value::Null))
    }
    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(values.into()))
    }
    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
            if values.insert(key.clone(), value.0).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON key {key:?}"
                )));
            }
        }
        Ok(StrictValue(values.into()))
    }
}

pub(crate) struct ManagedState {
    directory: File,
    record_name: String,
    lock_name: String,
    label: &'static str,
}

impl ManagedState {
    pub(crate) fn open(
        host: &Host<'_>,
        component: &str,
        stem: &str,
        label: &'static str,
    ) -> Result<Self> {
        validate_component(component)?;
        validate_stem(stem)?;
        let state_home = host
            .value("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| host.home().join(".local/state"));
        if !state_home.is_absolute() {
            bail!("{label} state directory must be absolute");
        }
        let root_existed = fs::symlink_metadata(&state_home).is_ok();
        fs::create_dir_all(&state_home).context("create selected managed-state root")?;
        let mut directory = open_directory_path(&state_home, "selected managed-state root")?;
        if !root_existed {
            rustix::fs::fchmod(&directory, rustix::fs::Mode::from_bits_truncate(0o700))
                .context("restrict selected managed-state root")?;
        }
        validate_state_directory(&directory, "selected managed-state root")?;
        directory = open_or_create_state_directory(&directory, "cozydot")?;
        directory = open_or_create_state_directory(&directory, component)?;
        Ok(Self {
            directory,
            record_name: format!("{stem}.json"),
            lock_name: format!("{stem}.lock"),
            label,
        })
    }

    pub(crate) fn acquire_lock(&self) -> Result<File> {
        let flags = rustix::fs::OFlags::RDWR
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC;
        let (lock, created): (File, bool) = match rustix::fs::openat(
            &self.directory,
            self.lock_name.as_str(),
            flags,
            rustix::fs::Mode::from_bits_truncate(0o600),
        ) {
            Ok(lock) => (lock.into(), true),
            Err(rustix::io::Errno::EXIST) => (
                rustix::fs::openat(
                    &self.directory,
                    self.lock_name.as_str(),
                    rustix::fs::OFlags::RDWR
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .with_context(|| {
                    format!(
                        "open existing {} managed-state lock without following links",
                        self.label
                    )
                })?
                .into(),
                false,
            ),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "create {} managed-state lock without following links",
                        self.label
                    )
                })
            }
        };
        if created {
            rustix::fs::fchmod(&lock, rustix::fs::Mode::from_bits_truncate(0o600))?;
        }
        validate_state_file(&lock, &format!("{} managed-state lock", self.label))?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)
            .with_context(|| format!("lock {} managed state", self.label))?;
        self.validate_lock_entry(&lock)?;
        Ok(lock)
    }

    pub(crate) fn validate_lock_entry(&self, lock: &File) -> Result<()> {
        validate_state_file(lock, &format!("{} managed-state lock", self.label))?;
        let entry = rustix::fs::statat(
            &self.directory,
            self.lock_name.as_str(),
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .with_context(|| format!("reinspect {} managed-state lock entry", self.label))?;
        let metadata = lock.metadata()?;
        if entry.st_dev != metadata.dev() || entry.st_ino != metadata.ino() {
            bail!("{} managed-state lock entry was replaced", self.label);
        }
        Ok(())
    }

    pub(crate) fn read(&self) -> Result<Option<Vec<u8>>> {
        let descriptor = match rustix::fs::openat(
            &self.directory,
            self.record_name.as_str(),
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        ) {
            Ok(value) => value,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("open {} managed record without following links", self.label)
                })
            }
        };
        read_validated_file(descriptor.into(), &format!("{} managed record", self.label)).map(Some)
    }

    pub(crate) fn publish(&self, bytes: &[u8]) -> Result<()> {
        let temporary_name = format!(
            ".{}.{}.{}.tmp",
            self.record_name,
            std::process::id(),
            TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        );
        let mut temporary: File = rustix::fs::openat(
            &self.directory,
            temporary_name.as_str(),
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_bits_truncate(0o600),
        )
        .with_context(|| format!("create {} managed-record staging file", self.label))?
        .into();
        let result = (|| {
            rustix::fs::fchmod(&temporary, rustix::fs::Mode::from_bits_truncate(0o600))?;
            validate_state_file(
                &temporary,
                &format!("{} managed-record staging file", self.label),
            )?;
            temporary.write_all(bytes)?;
            temporary.sync_all()?;
            rustix::fs::renameat(
                &self.directory,
                temporary_name.as_str(),
                &self.directory,
                self.record_name.as_str(),
            )
            .with_context(|| format!("publish {} managed record", self.label))?;
            self.directory
                .sync_all()
                .with_context(|| format!("sync {} managed-state directory", self.label))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = rustix::fs::unlinkat(
                &self.directory,
                temporary_name.as_str(),
                rustix::fs::AtFlags::empty(),
            );
        }
        result
    }
}

fn read_validated_file(mut file: File, label: &str) -> Result<Vec<u8>> {
    validate_state_file(&file, label)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("read {label} descriptor"))?;
    Ok(bytes)
}

fn validate_component(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("invalid managed-state component");
    }
    Ok(())
}
fn validate_stem(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        bail!("invalid managed-state stem");
    }
    Ok(())
}
fn open_directory_path(path: &std::path::Path, label: &str) -> Result<File> {
    Ok(rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .with_context(|| format!("open {label} without following links"))?
    .into())
}
fn open_or_create_state_directory(parent: &File, name: &str) -> Result<File> {
    let created =
        match rustix::fs::mkdirat(parent, name, rustix::fs::Mode::from_bits_truncate(0o700)) {
            Ok(()) => true,
            Err(rustix::io::Errno::EXIST) => false,
            Err(error) => {
                return Err(error).with_context(|| format!("create managed-state {name}"))
            }
        };
    let directory: File = rustix::fs::openat(
        parent,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .with_context(|| format!("open managed-state {name} without following links"))?
    .into();
    if created {
        rustix::fs::fchmod(&directory, rustix::fs::Mode::from_bits_truncate(0o700))?;
    }
    validate_state_directory(&directory, &format!("managed-state {name}"))?;
    Ok(directory)
}
fn validate_state_directory(directory: &File, label: &str) -> Result<()> {
    let metadata = directory
        .metadata()
        .with_context(|| format!("inspect {label}"))?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        bail!("{label} has unsafe type, owner, or permissions");
    }
    Ok(())
}
fn validate_state_file(file: &File, label: &str) -> Result<()> {
    let metadata = file
        .metadata()
        .with_context(|| format!("inspect {label}"))?;
    if !metadata.file_type().is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        bail!("{label} has unsafe type, owner, permissions, or link count");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn strict_json_rejects_duplicate_keys_recursively() {
        for value in [
            br#"{"version":1,"version":1}"#.as_slice(),
            br#"{"outer":{"name":"a","name":"b"}}"#.as_slice(),
        ] {
            assert!(parse_strict_json::<serde_json::Value>(value).is_err());
        }
    }

    #[test]
    fn record_reader_uses_held_nofollow_descriptor_after_entry_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("record.json");
        fs::write(&path, b"managed bytes").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let descriptor: File = rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap()
        .into();
        fs::rename(&path, directory.path().join("held-record.json")).unwrap();
        fs::write(directory.path().join("foreign"), b"foreign bytes").unwrap();
        symlink("foreign", &path).unwrap();

        assert_eq!(
            read_validated_file(descriptor, "test record").unwrap(),
            b"managed bytes"
        );
    }

    #[test]
    fn lock_entry_replacement_is_detected_against_held_descriptor() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let state = ManagedState {
            directory: open_directory_path(directory.path(), "test state").unwrap(),
            record_name: "sample.json".into(),
            lock_name: "sample.lock".into(),
            label: "test",
        };
        let lock = state.acquire_lock().unwrap();
        fs::remove_file(directory.path().join("sample.lock")).unwrap();
        fs::write(directory.path().join("sample.lock"), b"").unwrap();
        fs::set_permissions(
            directory.path().join("sample.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        assert!(state.validate_lock_entry(&lock).is_err());
    }
}
