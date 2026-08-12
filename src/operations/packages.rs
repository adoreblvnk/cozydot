pub(crate) mod cargo {
    use anyhow::{Context, Result, bail};

    use std::path::Path;

    use super::super::{Host, executable_file, path_program};

    pub(crate) fn ensure(host: &Host, packages: &[String]) -> Result<()> {
        let cargo_home = host.home().join(".cargo");
        let cargo = path_program(&cargo_home.join("bin/cargo"), "managed Cargo executable path")?;
        let output = host.require("Cargo installed package query", &cargo, ["install", "--list"])?;
        let installed = installed_crates(&output.stdout)?;
        let missing =
            packages.iter().filter(|package| !installed.contains(crate_identity(package))).cloned().collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(());
        }
        let binstall = resolve_binstall(host, &cargo_home)?
            .context("Cargo package operation: managed cargo-binstall is unavailable after bootstrap")?;
        let mut args = vec!["--no-confirm".to_owned(), "--".into()];
        args.extend(missing);
        host.require("Cargo package mutation", &binstall, args)?;
        Ok(())
    }

    pub(crate) fn update_all(host: &Host) -> Result<()> {
        let program = host.home().join(".cargo/bin/cargo-install-update");
        if !executable_file(&program) {
            return Ok(());
        }
        let program = path_program(&program, "managed cargo-install-update executable path")?;
        host.require("Cargo package update", &program, ["-a"])?;
        Ok(())
    }

    fn installed_crates(output: &[u8]) -> Result<std::collections::BTreeSet<String>> {
        let output = std::str::from_utf8(output).context("cargo install --list returned non-UTF-8 state")?;
        let mut installed = std::collections::BTreeSet::new();
        for line in output.lines().filter(|line| !line.is_empty()) {
            if line.starts_with(char::is_whitespace) {
                continue;
            }
            let header = line.strip_suffix(':').context("cargo install --list returned malformed state")?;
            let mut fields = header.splitn(3, char::is_whitespace).filter(|field| !field.is_empty());
            let name = fields.next().context("cargo install --list returned malformed state")?;
            let version = fields.next().context("cargo install --list returned malformed state")?;
            if !version.starts_with('v') || name.chars().any(char::is_control) {
                bail!("cargo install --list returned malformed state");
            }
            match fields.next() {
                None => {
                    installed.insert(name.to_owned());
                }
                Some(source)
                    if source.starts_with('(') && source.ends_with(')') && !source.chars().any(char::is_control) => {}
                Some(_) => bail!("cargo install --list returned malformed state"),
            }
        }
        Ok(installed)
    }

    fn crate_identity(package: &str) -> &str {
        package.split_once('@').map_or(package, |(name, _)| name)
    }

    fn resolve_binstall(host: &Host, cargo_home: &Path) -> Result<Option<String>> {
        if cfg!(target_os = "macos") {
            return super::super::macos::formula_program(host, "cargo-binstall", "cargo-binstall").map(Some);
        }
        let managed = cargo_home.join("bin/cargo-binstall");
        if executable_file(&managed) {
            return path_program(&managed, "cargo-binstall executable path").map(Some);
        }
        Ok(None)
    }
}

pub(crate) mod npm {
    use anyhow::{Context, Result, bail};

    use super::super::{Host, executable_file};

    pub(crate) fn ensure(host: &Host, packages: &[String]) -> Result<()> {
        let Some(fnm) = resolve_fnm(host)? else {
            bail!("npm package operation: managed fnm is unavailable after bootstrap");
        };
        let mut missing = Vec::new();
        for package in packages {
            let identity = package_identity(package);
            let output = host
                .run(&fnm, ["exec", "--using=default", "--", "npm", "list", "--global", "--depth=0", "--", identity])?;
            if !output.status.success() {
                missing.push(package.clone());
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        let mut npm_args = vec!["install".to_owned(), "--global".into(), "--".into()];
        npm_args.extend(missing);
        run_npm_required(host, &fnm, "npm package mutation", npm_args)?;
        Ok(())
    }

    pub(crate) fn update_all(host: &Host) -> Result<()> {
        let Some(fnm) = resolve_fnm(host)? else { return Ok(()) };
        run_npm_required(host, &fnm, "npm package update", ["update", "--global"])?;
        Ok(())
    }

    fn package_identity(package: &str) -> &str {
        if package.starts_with('@') {
            let slash = package.find('/').unwrap_or(package.len());
            let version = package[slash..].find('@').map(|index| slash + index);
            return version.map_or(package, |index| &package[..index]);
        }
        package.split_once('@').map_or(package, |(name, _)| name)
    }

    fn resolve_fnm(host: &Host) -> Result<Option<String>> {
        if cfg!(target_os = "macos") {
            return super::super::macos::formula_program(host, "fnm", "fnm").map(Some);
        }
        let data_home = host.home().join(".local/share");
        let managed = data_home.join("fnm/fnm");
        if executable_file(&managed) {
            return managed.to_str().map(str::to_owned).map(Some).context("managed fnm executable path is not UTF-8");
        }
        Ok(None)
    }

    fn run_npm_required<I, S>(host: &Host, fnm: &str, operation: &str, npm_args: I) -> Result<std::process::Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut args = vec!["exec".to_owned(), "--using=default".into(), "--".into(), "npm".into()];
        args.extend(npm_args.into_iter().map(|arg| arg.as_ref().to_owned()));
        host.require(operation, fnm, args)
    }
}

pub(crate) mod flatpak {
    use super::super::Host;
    use anyhow::Result;

    const FLATHUB_NAME: &str = "flathub";
    const FLATHUB_DESCRIPTOR_URL: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
    const FLATHUB_URL: &str = "https://dl.flathub.org/repo/";

    pub fn ensure_flathub(host: &Host) -> Result<()> {
        host.require(
            "Flathub remote ensure",
            "flatpak",
            ["--user", "remote-add", "--if-not-exists", FLATHUB_NAME, FLATHUB_DESCRIPTOR_URL],
        )?;
        let url_arg = format!("--url={FLATHUB_URL}");
        host.require(
            "Flathub remote security canonicalization",
            "flatpak",
            [
                "--user",
                "remote-modify",
                &url_arg,
                "--gpg-verify",
                "--enumerate",
                "--use-for-deps",
                "--enable",
                "--no-filter",
                FLATHUB_NAME,
            ],
        )?;
        Ok(())
    }

    pub fn ensure_apps(host: &Host, refs: &[String]) -> Result<()> {
        let mut missing = Vec::new();
        for app_id in refs {
            let output = host.run("flatpak", ["--user", "info", "--show-ref", "--", app_id])?;
            if !output.status.success() {
                missing.push(app_id.clone());
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        let mut args = vec![
            "--user".to_owned(),
            "install".into(),
            "--app".into(),
            "--noninteractive".into(),
            "-y".into(),
            "flathub".into(),
            "--".into(),
        ];
        args.extend(missing);
        host.require("Flatpak application installation", "flatpak", args)?;
        Ok(())
    }

    pub fn update_apps(host: &Host) -> Result<()> {
        host.require("Flatpak application update", "flatpak", ["--user", "update", "--app", "--noninteractive", "-y"])?;
        Ok(())
    }
}

pub(crate) mod fonts {
    use anyhow::{Context, Result, bail};
    use std::{ffi::OsStr, fs, path::Path};
    use url::Url;

    use super::super::{Host, TempPath};

    const FONT_ROOT: &str = "/usr/share/fonts";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum NerdFontsMode {
        EnsurePresent,
        Update,
    }

    pub(crate) fn execute(host: &Host, families: &[String], mode: NerdFontsMode) -> Result<()> {
        execute_at(host, families, mode, Path::new(FONT_ROOT), true)
    }

    pub(crate) fn execute_user(host: &Host, families: &[String], mode: NerdFontsMode) -> Result<()> {
        let parent = host.home().join("Library/Fonts");
        fs::create_dir_all(&parent).context("create user font directory")?;
        execute_at(host, families, mode, &parent, false)
    }

    fn execute_at(
        host: &Host,
        families: &[String],
        mode: NerdFontsMode,
        parent: &Path,
        privileged: bool,
    ) -> Result<()> {
        let mut changed = false;
        for family in families {
            let destination = parent.join(family);
            let is_present = match fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.is_dir() => true,
                Ok(_) => bail!("Nerd Font destination conflict at {}", destination.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    return Err(error).context(format!("inspect Nerd Font destination {}", destination.display()));
                }
            };
            if mode == NerdFontsMode::Update || !is_present {
                install_family(host, family, &destination, privileged)?;
                changed = true;
            }
        }
        if changed && privileged {
            host.require(
                "Nerd Font cache refresh",
                "sudo",
                [OsStr::new("fc-cache"), OsStr::new("--force"), parent.as_os_str()],
            )?;
        }
        Ok(())
    }

    fn install_family(host: &Host, family: &str, destination: &Path, privileged: bool) -> Result<()> {
        let archive = TempPath::new_with_suffix(host, "nerd-font", ".tar.xz")?;
        let mut url =
            Url::parse("https://github.com/ryanoasis/nerd-fonts/releases/latest/download/placeholder.tar.xz")?;
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
        let path = destination.to_str().context("font path is not UTF-8")?;
        let archive_path = archive.path().to_str().context("font archive path is not UTF-8")?;
        if privileged {
            host.require("Nerd Font destination replacement", "sudo", ["rm", "--recursive", "--force", "--", path])?;
            host.require("Nerd Font destination creation", "sudo", ["mkdir", "--parents", "--", path])?;
            host.require(
                "Nerd Font archive extraction",
                "sudo",
                ["tar", "--extract", "--xz", "--directory", path, "--file", archive_path],
            )?;
        } else {
            host.require("Nerd Font destination replacement", "rm", ["-rf", path])?;
            host.require("Nerd Font destination creation", "mkdir", ["-p", path])?;
            host.require("Nerd Font archive extraction", "tar", ["-xJf", archive_path, "-C", path])?;
        }
        Ok(())
    }
}

pub(crate) mod dotfiles {
    use anyhow::{Context, Result, bail};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::super::Host;

    pub(crate) fn execute(host: &Host, root: &Path, packages: &[String], replace: bool) -> Result<()> {
        let root = fs::canonicalize(root)
            .with_context(|| format!("dotfiles operation: canonicalize root {}", root.display()))?;
        if !fs::symlink_metadata(&root)?.file_type().is_dir() {
            bail!("dotfiles root is not a directory: {}", root.display());
        }

        let mut sources = Vec::with_capacity(packages.len());
        for package in packages {
            let source = root.join(package);
            let metadata = fs::symlink_metadata(&source)
                .with_context(|| format!("dotfiles package {package:?} does not exist"))?;
            if !metadata.file_type().is_dir() {
                bail!("dotfiles package {package:?} is not a real directory");
            }
            sources.push((package, source));
        }

        let mut conflicts = Vec::new();
        for (package, source) in &sources {
            collect_conflicts(source, host.home(), package, &mut conflicts)?;
        }
        conflicts.sort_by(|left, right| left.1.cmp(&right.1));
        conflicts.dedup_by(|left, right| left.1 == right.1);
        if !conflicts.is_empty() && !replace {
            let paths =
                conflicts.iter().map(|(_, path)| format!("  {}", path.display())).collect::<Vec<_>>().join("\n");
            bail!(
                "unmanaged dotfile conflicts:\n{paths}\nno dotfiles were changed; rerun with `cozydot dotfiles --replace`"
            );
        }
        host.require("Stow preflight", "stow", ["--version"]).context("dotfiles require GNU Stow")?;
        if replace {
            backup_conflicts(host, &conflicts)?;
        }

        for (package, source) in sources {
            apply_package(host, &root, package, &source)?;
        }
        Ok(())
    }

    fn apply_package(host: &Host, root: &Path, package: &str, source: &Path) -> Result<()> {
        prepare_gnupg_home(source, &host.home())?;
        host.require(
            "dotfiles Stow mutation",
            "stow",
            [
                "--dir".as_ref(),
                root.as_os_str(),
                "--target".as_ref(),
                host.home().as_os_str(),
                "--stow".as_ref(),
                "--".as_ref(),
                package.as_ref(),
            ],
        )?;
        Ok(())
    }

    fn prepare_gnupg_home(source: &Path, home: &Path) -> Result<()> {
        let source = source.join(".gnupg");
        if !source.is_dir() {
            return Ok(());
        }

        let target = home.join(".gnupg");
        if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            fs::remove_file(&target).context("replace folded GnuPG dotfiles directory")?;
        }
        fs::create_dir_all(&target).context("create GnuPG home")?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).context("secure GnuPG home")?;
        Ok(())
    }

    fn collect_conflicts(
        source: &Path,
        target: PathBuf,
        package: &str,
        conflicts: &mut Vec<(String, PathBuf)>,
    ) -> Result<()> {
        let source_metadata =
            fs::symlink_metadata(source).with_context(|| format!("inspect dotfiles source {}", source.display()))?;
        if source_metadata.file_type().is_dir() {
            match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_dir() => {
                    let mut entries = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
                    entries.sort_by_key(std::fs::DirEntry::file_name);
                    for entry in entries {
                        collect_conflicts(&entry.path(), target.join(entry.file_name()), package, conflicts)?;
                    }
                }
                Ok(_) if !resolves_to(&target, source) => conflicts.push((package.to_owned(), target)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("inspect dotfile destination"),
            }
        } else if source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
            match fs::symlink_metadata(&target) {
                Ok(_) if !resolves_to(&target, source) => conflicts.push((package.to_owned(), target)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("inspect dotfile destination"),
            }
        } else {
            bail!("unsupported dotfiles source type at {}", source.display());
        }
        Ok(())
    }

    fn backup_conflicts(host: &Host, conflicts: &[(String, PathBuf)]) -> Result<()> {
        if conflicts.is_empty() {
            return Ok(());
        }
        let state_home =
            host.value("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/state"));
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("dotfiles backup timestamp is before the Unix epoch")?
            .as_nanos();
        let backup_root =
            state_home.join("cozydot/dotfile-backups").join(format!("{timestamp}-{}", std::process::id()));
        for (package, conflict) in conflicts {
            let relative = conflict
                .strip_prefix(host.home())
                .with_context(|| format!("dotfiles conflict escaped the home directory: {}", conflict.display()))?;
            let backup = backup_root.join(package).join(relative);
            let parent = backup.parent().context("dotfiles backup has no parent")?;
            fs::create_dir_all(parent).context("create dotfiles backup directory")?;
            fs::rename(conflict, &backup)
                .with_context(|| format!("move dotfiles conflict {} to {}", conflict.display(), backup.display()))?;
            if fs::symlink_metadata(conflict).is_ok() || fs::symlink_metadata(&backup).is_err() {
                bail!("dotfiles conflict backup did not move {} to {}", conflict.display(), backup.display());
            }
        }
        Ok(())
    }

    fn resolves_to(target: &Path, source: &Path) -> bool {
        fs::canonicalize(target)
            .and_then(|target| fs::canonicalize(source).map(|source| target == source))
            .unwrap_or(false)
    }
}
