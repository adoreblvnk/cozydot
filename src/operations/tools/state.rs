use super::*;

pub(super) fn inspect_rust(
    host: &Host<'_>,
    rustup: &str,
    toolchain: &str,
) -> Result<Option<RustState>> {
    let output = host.run(
        rustup,
        ["run", toolchain, "rustc", "--version", "--verbose"],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    parse_rust_state(&output.stdout).map(Some)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RustState {
    pub(super) release: String,
    pub(super) host: String,
}

pub(super) fn parse_rust_state(output: &[u8]) -> Result<RustState> {
    let output = std::str::from_utf8(output).context("rustc returned non-UTF-8 state")?;
    let mut release = None;
    let mut host = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("release: ") {
            if release.replace(value.to_owned()).is_some() || !valid_rust_release(value) {
                bail!("rustc returned malformed release state");
            }
        } else if let Some(value) = line.strip_prefix("host: ") {
            if host.replace(value.to_owned()).is_some() || !valid_rust_host(value) {
                bail!("rustc returned malformed host state");
            }
        }
    }
    Ok(RustState {
        release: release.context("rustc state is missing release")?,
        host: host.context("rustc state is missing host")?,
    })
}

pub(super) fn rust_default(host: &Host<'_>, rustup: &str) -> Result<Option<String>> {
    let output = host.run(rustup, ["default"])?;
    if !output.status.success() {
        return Ok(None);
    }
    let output = single_line(&output.stdout, "rustup default")?;
    if output == "no default toolchain configured" {
        return Ok(None);
    }
    let Some(toolchain) = output.strip_suffix(" (default)") else {
        bail!("rustup returned malformed default toolchain state");
    };
    if toolchain.is_empty() || toolchain.chars().any(char::is_whitespace) {
        bail!("rustup returned malformed default toolchain state");
    }
    Ok(Some(toolchain.to_owned()))
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct GoState {
    pub(super) version: String,
    pub(super) architecture: String,
}

pub(super) fn inspect_go(host: &Host<'_>, program: &str) -> Result<Option<GoState>> {
    if program.starts_with('/') && !executable_file(Path::new(program)) {
        return Ok(None);
    }
    let output = match host.run(program, ["version"]) {
        Ok(output) => output,
        Err(error)
            if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::PermissionDenied
            }) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    if !output.status.success() {
        return Ok(None);
    }
    parse_go_state(&output.stdout).map(Some)
}

pub(super) fn parse_go_state(output: &[u8]) -> Result<GoState> {
    let output = single_line(output, "go version")?;
    let fields = output.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 4
        || fields[0] != "go"
        || fields[1] != "version"
        || fields[3] != "linux/amd64"
            && fields[3] != "linux/arm64"
            && fields[3] != "linux/arm"
            && fields[3] != "linux/riscv64"
    {
        bail!("go returned malformed version state");
    }
    let version = fields[2]
        .strip_prefix("go")
        .filter(|version| numeric_version(version, 2, 3))
        .context("go returned malformed version state")?;
    Ok(GoState {
        version: version.to_owned(),
        architecture: fields[3].trim_start_matches("linux/").to_owned(),
    })
}

pub(super) fn validate_go_archive_listing(output: &[u8]) -> Result<()> {
    let output = std::str::from_utf8(output).context("Go archive listing is not UTF-8")?;
    let mut saw_binary = false;
    for entry in output.lines() {
        if entry.is_empty()
            || !entry.starts_with("go/")
            || entry.split('/').any(|component| component == "..")
            || entry.chars().any(char::is_control)
        {
            bail!("Go archive contains an unsafe path");
        }
        saw_binary |= entry == "go/bin/go";
    }
    if !saw_binary {
        bail!("Go archive listing does not contain go/bin/go");
    }
    Ok(())
}

pub(super) fn resolve_fnm(host: &Host<'_>) -> Result<String> {
    let data_home = host
        .value("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(".local/share"));
    if !data_home.is_absolute() {
        bail!("managed FNM data directory must be absolute");
    }
    let managed = data_home.join("fnm/fnm");
    if executable_file(&managed) {
        return path_program(&managed, "managed fnm executable");
    }
    bail!("Node toolchain operation: fnm is unavailable after bootstrap")
}

pub(super) fn inspect_node(host: &Host<'_>, fnm: &str, selector: &str) -> Result<Option<String>> {
    let output = host.run(
        fnm,
        ["exec", "--using", selector, "--", "node", "--version"],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    parse_node_version(&output.stdout).map(Some)
}

pub(super) fn resolve_node_version(
    host: &Host<'_>,
    fnm: &str,
    selector: &NodeToolchainSelector,
) -> Result<String> {
    if let NodeToolchainSelector::Version(version) = selector {
        if numeric_version(version, 3, 3) {
            return Ok(format!("v{version}"));
        }
    }
    let mut args = vec!["list-remote", "--latest"];
    match selector {
        NodeToolchainSelector::Lts => args.push("--lts"),
        NodeToolchainSelector::Latest => {}
        NodeToolchainSelector::Version(version) => {
            args.extend(["--filter", version]);
        }
    }
    let output = host.require("Node release resolution", fnm, args)?;
    parse_remote_node_version(&output.stdout)
}

pub(super) fn parse_remote_node_version(output: &[u8]) -> Result<String> {
    let output = single_line(output, "fnm list-remote")?;
    let version = output
        .split_whitespace()
        .next()
        .context("fnm list-remote returned empty state")?;
    if !output
        .chars()
        .all(|character| !character.is_control() || character == '\t')
    {
        bail!("fnm list-remote returned malformed state");
    }
    parse_node_version(format!("{version}\n").as_bytes())
}

pub(super) fn parse_node_version(output: &[u8]) -> Result<String> {
    let version = single_line(output, "node --version")?;
    let numeric = version
        .strip_prefix('v')
        .filter(|version| numeric_version(version, 3, 3))
        .context("node returned malformed version state")?;
    Ok(format!("v{numeric}"))
}

pub(super) fn fnm_default(host: &Host<'_>, fnm: &str) -> Result<Option<String>> {
    let output = host.run(fnm, ["default"])?;
    if !output.status.success() {
        return Ok(None);
    }
    if output.stdout.is_empty() || output.stdout == b"none\n" || output.stdout == b"none" {
        return Ok(None);
    }
    parse_node_version(&output.stdout).map(Some)
}

pub(super) fn inspect_python(host: &Host<'_>, uv: &str, request: &str) -> Result<Option<String>> {
    let output = host.run(
        uv,
        [
            "python",
            "find",
            "--no-project",
            "--managed-python",
            "--show-version",
            request,
        ],
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    let version = single_line(&output.stdout, "uv python find")?;
    if !numeric_version(version, 3, 3) {
        bail!("uv returned malformed managed Python version state");
    }
    Ok(Some(version.to_owned()))
}

pub(super) fn resolve_managed(
    host: &Host<'_>,
    directory_variable: &str,
    default_directory: &str,
    relative_program: &str,
) -> Result<Option<String>> {
    let base = host
        .value(directory_variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| host.home().join(default_directory));
    if !base.is_absolute() {
        bail!("managed tool directory must be absolute");
    }
    let managed = base.join(relative_program);
    if executable_file(&managed) {
        return path_program(&managed, "managed tool executable").map(Some);
    }
    Ok(None)
}

fn executable_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0
    })
}

pub(super) fn path_program(path: &Path, description: &str) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .with_context(|| format!("{description} path is not UTF-8: {}", path.display()))
}
