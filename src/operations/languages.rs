use super::{Host, TempDir, TempPath};
use crate::json_helpers;
use anyhow::{bail, Context, Result};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub fn go(host: &Host<'_>, requested: &str, arch: &str) -> Result<()> {
    if requested != "latest" && host.command_exists("go") {
        let output = host.require("go install", "go", ["version"])?;
        if String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .nth(2)
            == Some(&format!("go{requested}"))
        {
            return Ok(());
        }
    }
    let metadata = host.require(
        "go install",
        "curl",
        ["-fsSL", "https://go.dev/dl/?mode=json&include=all"],
    )?;
    let (version, filename, checksum) =
        json_helpers::latest_go(&String::from_utf8(metadata.stdout)?, requested, arch)?;
    if host.command_exists("go") {
        let output = host.require("go install", "go", ["version"])?;
        if String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .nth(2)
            == Some(&format!("go{version}"))
        {
            return Ok(());
        }
    }
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("go install: invalid archive checksum");
    }
    let archive = TempPath::new(host, "go.tar.gz")?;
    let stage = TempDir::new(host, "go-stage")?;
    let url = format!("https://go.dev/dl/{filename}");
    host.require(
        "go install",
        "curl",
        ["-fL", "-o", &archive.path().to_string_lossy(), &url],
    )?;
    let checksum_input = format!("{checksum}  {}\n", archive.path().display());
    host.require_input(
        "go install checksum",
        "sha256sum",
        ["-c", "-"],
        checksum_input.as_bytes(),
    )?;
    host.require(
        "go install",
        "tar",
        ["-tzf", &archive.path().to_string_lossy()],
    )?;
    host.require(
        "go install",
        "tar",
        [
            "-C",
            &stage.path().to_string_lossy(),
            "-xzf",
            &archive.path().to_string_lossy(),
        ],
    )?;
    let staged_go = stage.path().join("go");
    let go_binary = staged_go.join("bin/go");
    if !go_binary.is_file() || std::fs::metadata(&go_binary)?.permissions().mode() & 0o111 == 0 {
        bail!("go install: extracted archive has no executable go binary");
    }
    host.require("go install", "sudo", ["rm", "-rf", "/usr/local/go"])?;
    host.require(
        "go install",
        "sudo",
        ["mv", &staged_go.to_string_lossy(), "/usr/local/go"],
    )?;
    Ok(())
}

pub fn node(host: &Host<'_>, version: &str, npm: &[String], update: bool) -> Result<()> {
    let fnm = ensure_fnm(host)?;
    let mut args = vec![
        "-euo".to_owned(),
        "pipefail".to_owned(),
        "-c".to_owned(),
        r#"eval "$("$1" env --shell bash)"
fnm=$1
requested=$2
update=$3
shift 3
current=$("$fnm" current 2>/dev/null || true)
default=$("$fnm" default 2>/dev/null || true)
candidate=
if [ "$update" != true ]; then
  if [ "$requested" = latest ]; then
    if [ -n "$current" ] && [ "$current" != none ]; then candidate=$current
    elif [ -n "$default" ] && [ "$default" != none ]; then candidate=$default
    fi
  else
    for installed in "$current" "$default"; do
      case "${installed#v}" in
        "$requested"|"$requested".*) candidate=$installed; break ;;
      esac
    done
  fi
fi
if [ -n "$candidate" ]; then
  "$fnm" use "$candidate"
elif [ "$requested" = latest ]; then
  "$fnm" install --lts --use
else
  "$fnm" install "$requested" --use
fi
current=$("$fnm" current)
if [ "$default" != "$current" ]; then "$fnm" default "$current"; fi
if [ "$#" -gt 0 ]; then
  for package in "$@"; do
    if ! npm list --global --depth=0 "$package" >/dev/null 2>&1; then
      npm install --global "$package"
    fi
  done
fi"#
        .to_owned(),
        "--".to_owned(),
        fnm,
        version.to_owned(),
        update.to_string(),
    ];
    args.extend(npm.iter().cloned());
    host.require("node install", "bash", args)?;
    Ok(())
}

pub fn fnm_bootstrap(host: &Host<'_>) -> Result<()> {
    ensure_fnm(host).map(|_| ())
}

fn ensure_fnm(host: &Host<'_>) -> Result<String> {
    let fnm_dir = host
        .value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"))
        .join("fnm");
    let installed_fnm = fnm_dir.join("fnm");
    Ok(if host.command_exists("fnm") {
        "fnm".to_owned()
    } else if executable_file(&installed_fnm) {
        installed_fnm.to_string_lossy().into_owned()
    } else {
        let installer = TempPath::new(host, "fnm-install")?;
        host.require(
            "node install",
            "curl",
            [
                "-fsSL",
                "-o",
                &installer.path().to_string_lossy(),
                "https://fnm.vercel.app/install",
            ],
        )?;
        host.require(
            "node install",
            "bash",
            [&installer.path().to_string_lossy(), "--skip-shell"],
        )?;
        if !executable_file(&installed_fnm) {
            bail!(
                "node install: fnm installer did not create {}",
                installed_fnm.display()
            );
        }
        installed_fnm.to_string_lossy().into_owned()
    })
}

pub fn npm_packages(host: &Host<'_>, packages: &[String]) -> Result<()> {
    for package in packages {
        let installed = host.run("npm", ["list", "--global", "--depth=0", package])?;
        if !installed.status.success() {
            host.require(
                "npm package installation",
                "npm",
                ["install", "--global", package],
            )?;
        }
    }
    Ok(())
}

pub fn pyenv(host: &Host<'_>, update: bool, version: &str, pip: bool) -> Result<()> {
    let root = host.home().join(".pyenv");
    let pyenv = if host.command_exists("pyenv") {
        "pyenv".to_owned()
    } else {
        if root.is_dir() {
            bail!("pyenv install: pyenv directory exists but pyenv is not in PATH");
        }
        let installer = TempPath::new(host, "pyenv-install")?;
        host.require(
            "pyenv install",
            "curl",
            [
                "-fL",
                "-o",
                &installer.path().to_string_lossy(),
                "https://pyenv.run",
            ],
        )?;
        host.require("pyenv install", "bash", [installer.path()])?;
        root.join("bin/pyenv").to_string_lossy().into_owned()
    };
    if update && host.command_exists("pyenv") {
        host.require("pyenv install", &pyenv, ["update"])?;
    }
    let latest = host.require("pyenv install", &pyenv, ["latest", "-k", version])?;
    let latest = String::from_utf8(latest.stdout)?.trim().to_owned();
    let active = host.require("pyenv install", &pyenv, ["version-name"])?;
    if String::from_utf8(active.stdout)?.trim() != latest {
        let versions = host.require("pyenv install", &pyenv, ["versions", "--bare"])?;
        if !String::from_utf8(versions.stdout)?
            .lines()
            .any(|installed| installed == latest)
        {
            host.require("pyenv install", &pyenv, ["install", &latest])?;
        }
        host.require("pyenv install", &pyenv, ["global", &latest])?;
    }
    if pip {
        let python = root.join("shims").join(format!("python{version}"));
        let program = python.to_str().context("pyenv Python path is not UTF-8")?;
        host.require(
            "pyenv pip upgrade",
            program,
            ["-m", "pip", "install", "-q", "--upgrade", "pip"],
        )?;
    }
    Ok(())
}

pub fn uv(host: &Host<'_>, version_enabled: bool, version: &str) -> Result<()> {
    let uv = ensure_uv(host)?;
    if version_enabled {
        let installed = host.run(&uv, ["python", "find", version])?;
        if !installed.status.success() {
            host.require("uv install", &uv, ["python", "install", version])?;
        }
    }
    Ok(())
}

pub fn uv_bootstrap(host: &Host<'_>) -> Result<()> {
    ensure_uv(host).map(|_| ())
}

fn ensure_uv(host: &Host<'_>) -> Result<String> {
    let install_dir = host.home().join(".local/bin");
    let installed_uv = install_dir.join("uv");
    Ok(if host.command_exists("uv") {
        "uv".to_owned()
    } else if executable_file(&installed_uv) {
        installed_uv.to_string_lossy().into_owned()
    } else {
        let installer = TempPath::new(host, "uv-install")?;
        host.require(
            "uv install",
            "curl",
            [
                "-LsSf",
                "-o",
                &installer.path().to_string_lossy(),
                "https://astral.sh/uv/install.sh",
            ],
        )?;
        std::fs::create_dir_all(&install_dir).context("uv install: create install directory")?;
        host.require(
            "uv install",
            "env",
            vec![
                format!("UV_UNMANAGED_INSTALL={}", install_dir.display()),
                "sh".into(),
                installer.path().to_string_lossy().into_owned(),
            ],
        )?;
        if !executable_file(&installed_uv) {
            bail!(
                "uv install: installer did not create executable {}",
                installed_uv.display()
            );
        }
        installed_uv.to_string_lossy().into_owned()
    })
}

fn executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
