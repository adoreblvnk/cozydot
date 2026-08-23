use std::{
    collections::BTreeMap,
    env,
    ffi::OsStr,
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

fn main() {
    if let Err(error) = generate() {
        panic!("failed to generate embedded bundle: {error}");
    }
}

fn generate() -> io::Result<()> {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let mut records = BTreeMap::new();
    walk(&root.join("dotfiles"), Path::new("dotfiles"), &mut records)?;

    let mut presets = Vec::new();
    for (constant, name) in [("COZYDOT_PRESET", "cozydot"), ("CLI_PRESET", "cli"), ("VM_PRESET", "vm")] {
        let source = root.join("configs").join(format!("{name}.yaml"));
        println!("cargo:rerun-if-changed={}", source.display());
        let metadata = fs::symlink_metadata(&source)?;
        if !metadata.file_type().is_file() {
            return Err(invalid(&source, "preset is not a regular file"));
        }
        presets.push((constant, fs::read(source)?));
    }

    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bundle.rs");
    let mut file = fs::File::create(output)?;
    writeln!(file, "pub static EMBEDDED_FILES: &[EmbeddedFile] = &[")?;
    for (path, (source, mode)) in records {
        let bytes = fs::read(source)?;
        writeln!(file, "    EmbeddedFile {{ path: {path:?}, bytes: &{bytes:?}, mode: {mode:#o} }},")?;
    }
    writeln!(file, "];")?;
    for (constant, bytes) in presets {
        writeln!(file, "pub const {constant}: &[u8] = &{bytes:?};")?;
    }
    Ok(())
}

fn walk(source: &Path, destination: &Path, records: &mut BTreeMap<String, (PathBuf, u32)>) -> io::Result<()> {
    println!("cargo:rerun-if-changed={}", source.display());
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.is_dir() {
        return Err(invalid(source, "asset root is not a directory"));
    }
    let mut entries = fs::read_dir(source)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let file_name = entry.file_name();
        let name = validate_name(&file_name, &entry.path())?;
        let child_destination = destination.join(name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(&entry.path(), &child_destination, records)?;
        } else if kind.is_file() {
            add_file(&entry.path(), &child_destination, records)?;
        } else {
            return Err(invalid(&entry.path(), "asset is a symlink or special file"));
        }
    }
    Ok(())
}

fn validate_name<'a>(name: &'a OsStr, source: &Path) -> io::Result<&'a str> {
    let name = name.to_str().ok_or_else(|| invalid(source, "asset path is not UTF-8"))?;
    if name.is_empty() || name.contains(['\t', '\n', '\r']) {
        return Err(invalid(source, "asset path contains an unsafe character"));
    }
    Ok(name)
}

fn add_file(source: &Path, destination: &Path, records: &mut BTreeMap<String, (PathBuf, u32)>) -> io::Result<()> {
    println!("cargo:rerun-if-changed={}", source.display());
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.file_type().is_file() {
        return Err(invalid(source, "asset is not a regular file"));
    }
    let destination = validate_destination(destination, source)?;
    // normalize bundled modes for reproducible installs; only shebang files are executable
    let mode = if fs::read(source)?.starts_with(b"#!") { 0o755 } else { 0o644 };
    if records.insert(destination.clone(), (source.to_path_buf(), mode)).is_some() {
        return Err(invalid(source, &format!("duplicate destination {destination}")));
    }
    Ok(())
}

fn validate_destination(destination: &Path, source: &Path) -> io::Result<String> {
    let mut components = destination.components();
    if !matches!(components.next(), Some(Component::Normal(root)) if root == "dotfiles")
        || !matches!(components.next(), Some(Component::Normal(_)))
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid(source, "asset destination must have shape dotfiles/<package>/<content>"));
    }
    destination
        .to_str()
        .filter(|path| !path.contains(['\t', '\n', '\r']))
        .map(str::to_owned)
        .ok_or_else(|| invalid(source, "asset destination is invalid UTF-8 or contains controls"))
}

fn invalid(path: &Path, message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{message}: {}", path.display()))
}
