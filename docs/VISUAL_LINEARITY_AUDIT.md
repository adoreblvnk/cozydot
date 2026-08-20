# Visual linearity audit

Audited all 37 Rust files under `src/` against this rule:

> Prefer explicit, visually linear code. Multiline data is fine; multiline expression chains are not. Use temporary locals to name stages, keep assignments anchored, and avoid iterator adapters when a direct list, loop, or conditional is clearer.

The proposals preserve behavior and were checked against rustfmt. Concise one-line semantic chains were excluded when a loop or extra locals would be less clear.

## Root modules

### `src/config.rs:577-579`

Current:

```rust
u32::try_from(duration.as_secs())
    .map(Self)
    .map_err(|_| de::Error::custom("duration exceeds the supported uint32 seconds range"))
```

Proposed:

```rust
let seconds = u32::try_from(duration.as_secs());
let seconds = seconds.map_err(|_| de::Error::custom("duration exceeds the supported uint32 seconds range"))?;
Ok(Self(seconds))
```

The conversion and error mapping become separate named stages.

### `src/init.rs:104-111`

Current:

```rust
let managed = self
    .managed
    .iter()
    .filter(|(relative, _)| {
        relative.strip_prefix(package).is_ok_and(|suffix| suffix.components().next().is_some())
    })
    .collect::<Vec<_>>();
```

Proposed:

```rust
let mut managed = Vec::new();
for (relative, hash) in &self.managed {
    let Ok(suffix) = relative.strip_prefix(package) else {
        continue;
    };
    if suffix.components().next().is_some() {
        managed.push((relative, hash));
    }
}
```

The loop exposes prefix validation and conditional collection in reading order.

### `src/init.rs:128-130`

Current:

```rust
Ok(managed
    .iter()
    .all(|(relative, hash)| hash_file(&self.root.join(relative)).is_ok_and(|current| &current == *hash)))
```

Proposed:

```rust
for (relative, hash) in managed {
    let path = self.root.join(relative);
    let Ok(current) = hash_file(&path) else {
        return Ok(false);
    };
    if &current != hash {
        return Ok(false);
    }
}
Ok(true)
```

Hash lookup, failure handling, and comparison become explicit stages.

### `src/main.rs:49-51`

Current:

```rust
let config = config::Config::load(&root.join("cozydot.yaml"))
    .with_context(|| "active configuration is missing or invalid; run 'cozydot init' first")?;
```

Proposed:

```rust
let path = root.join("cozydot.yaml");
let config = config::Config::load(&path);
let context = "active configuration is missing or invalid; run 'cozydot init' first";
let config = config.with_context(|| context)?;
```

### `src/main.rs:67-69`

Current:

```rust
config::Config::load(&path)
    .with_context(|| "active configuration is missing or invalid; run 'cozydot init' first")?;
```

Proposed:

```rust
let context = "active configuration is missing or invalid; run 'cozydot init' first";
config::Config::load(&path).with_context(|| context)?;
```

Both proposals keep configuration loading and context attachment on visually separate stages.

### `src/platform.rs:44-49`

Current:

```rust
let base_codename = match family {
    Family::Ubuntu => os.get_value("UBUNTU_CODENAME"),
    Family::Debian => os.get_value("DEBIAN_CODENAME"),
}
.unwrap_or(&distro_codename)
.to_owned();
```

Proposed:

```rust
let base_codename = match family {
    Family::Ubuntu => os.get_value("UBUNTU_CODENAME"),
    Family::Debian => os.get_value("DEBIAN_CODENAME"),
};
let base_codename = base_codename.unwrap_or(&distro_codename).to_owned();
```

The platform lookup is separated from fallback and ownership conversion.

### `src/workflow.rs:31-32`

Current:

```rust
const APT_PREREQS: [&str; 8] =
    ["ca-certificates", "curl", "fontconfig", "gnupg", "stow", "unzip", "xdg-terminal-exec", "xz-utils"];
```

Proposed:

```rust
const APT_PREREQS: [&str; 8] = [
    "ca-certificates",
    "curl",
    "fontconfig",
    "gnupg",
    "stow",
    "unzip",
    "xdg-terminal-exec",
    "xz-utils",
];
```

The assignment stays anchored while the array remains readable multiline data.

### `src/workflow.rs:188-189`

Current:

```rust
let homebrew_packages =
    dotfiles.is_some() || !config.macos.homebrew.formulae.is_empty() || !config.macos.homebrew.casks.is_empty();
```

Proposed:

```rust
let has_formulae = !config.macos.homebrew.formulae.is_empty();
let has_casks = !config.macos.homebrew.casks.is_empty();
let homebrew_packages = dotfiles.is_some() || has_formulae || has_casks;
```

## Host operations

### `src/operations/host/mod.rs:32`

Current:

```rust
let args = args.into_iter().map(|arg| arg.as_ref().to_os_string()).collect::<Vec<_>>();
```

Proposed:

```rust
let mut owned_args = Vec::new();
for arg in args {
    owned_args.push(arg.as_ref().to_os_string());
}
```

The collection and ownership conversion are explicit.

### `src/operations/host/mod.rs:50-52`

Current:

```rust
self.run(&format!("{label} CLI version check"), program, ["--version"])
    .with_context(|| format!("{label} integration requires an existing usable {program} CLI"))?;
```

Proposed:

```rust
let version_check = self.run(&format!("{label} CLI version check"), program, ["--version"]);
version_check.with_context(|| format!("{label} integration requires an existing usable {program} CLI"))?;
```

### `src/operations/host/mod.rs:61-65`

Current:

```rust
let mut curl_args = ["--location", "--fail", "--silent", "--show-error", "--retry", "3", "--retry-all-errors"]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
curl_args.extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
```

Proposed:

```rust
let options = [
    "--location",
    "--fail",
    "--silent",
    "--show-error",
    "--retry",
    "3",
    "--retry-all-errors",
];
let mut curl_args = options.map(OsString::from).to_vec();
for arg in args {
    curl_args.push(arg.as_ref().to_os_string());
}
```

The options remain multiline data, while both argument-building stages become explicit.

### `src/operations/host/mod.rs:75-78`

Current:

```rust
std::env::var_os("PATH")
    .is_some_and(|path| std::env::split_paths(&path).any(|directory| executable_file(&directory.join(name))))
```

Proposed:

```rust
let Some(path) = std::env::var_os("PATH") else {
    return false;
};
for directory in std::env::split_paths(&path) {
    if executable_file(&directory.join(name)) {
        return true;
    }
}
false
```

### `src/operations/host/mod.rs:88-104`

Current:

```rust
tempfile::Builder::new()
    .prefix(stem)
    .suffix(suffix)
    .tempfile()
    .map(|file| Self(file.into_temp_path()))
    .context("create temporary file")
```

Proposed:

```rust
let mut builder = tempfile::Builder::new();
builder.prefix(stem);
builder.suffix(suffix);
let file = builder.tempfile().context("create temporary file")?;
Ok(Self(file.into_temp_path()))
```

Apply the same stages to `new_in_with_suffix`, replacing `tempfile()` with `tempfile_in(parent)`.

### `src/operations/host/mod.rs:115-118`

Current:

```rust
fs::symlink_metadata(path)
    .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0)
```

Proposed:

```rust
let Ok(metadata) = fs::symlink_metadata(path) else {
    return false;
};
metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0
```

### `src/operations/host/mod.rs:141-147`

Current:

```rust
std::iter::once(OsStr::new(program))
    .chain(args.iter().map(OsString::as_os_str))
    .map(|part| part.to_string_lossy())
    .collect::<Vec<_>>()
    .join(" ")
```

Proposed:

```rust
let mut parts = Vec::with_capacity(args.len() + 1);
parts.push(OsStr::new(program).to_string_lossy());
for arg in args {
    parts.push(arg.to_string_lossy());
}
parts.join(" ")
```

## Desktop and integration operations

### `src/operations/desktop/mod.rs:23-24`

Current:

```rust
const GNOME_TERMINAL_SHORTCUT: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/cozydot-terminal/";
```

Proposed:

```rust
const GNOME_TERMINAL_SHORTCUT: &str = concat!(
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/",
    "cozydot-terminal/",
);
```

The assignment stays anchored and the path remains multiline data.

### `src/operations/desktop/mod.rs:58-60`

Current:

```rust
let config_home = env::var_os("XDG_CONFIG_HOME");
let fallback = || host.home().join(".config");
let config_home = config_home.filter(|path| !path.is_empty()).map(PathBuf::from).unwrap_or_else(fallback);
```

Proposed:

```rust
let config_home = match env::var_os("XDG_CONFIG_HOME") {
    Some(path) if !path.is_empty() => PathBuf::from(path),
    _ => host.home().join(".config"),
};
```

The resolved path has 1 name and 1 representation.

### `src/operations/desktop/gnome.rs:15-16`

Current:

```rust
const ROUNDED_CORNERS_SETTINGS: &str =
    "/org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings";
```

Proposed:

```rust
const ROUNDED_CORNERS_SETTINGS: &str = concat!(
    "/org/gnome/shell/extensions/rounded-window-corners-reborn/",
    "global-rounded-corner-settings",
);
```

### `src/operations/desktop/gnome.rs:69-75`

Current:

```rust
let valid = value.split_once('@').is_some_and(|(left, right)| valid_uuid_part(left) && valid_uuid_part(right));
if !valid {
    bail!("invalid GNOME extension UUID {value:?}");
}
```

Proposed:

```rust
let Some((left, right)) = value.split_once('@') else {
    bail!("invalid GNOME extension UUID {value:?}");
};
if !valid_uuid_part(left) || !valid_uuid_part(right) {
    bail!("invalid GNOME extension UUID {value:?}");
}
```

UUID structure and part validation become explicit stages.

### `src/operations/desktop/gnome.rs:83-88`

Current:

```rust
let metadata = host.curl("GNOME extension metadata", &endpoint, std::iter::empty::<&str>())?;
let shell = host.run("GNOME extension shell version", "gnome-shell", ["--version"])?;
let shell_version = shell_version(std::str::from_utf8(&shell.stdout).context("GNOME Shell version is not UTF-8")?)?;
let metadata = std::str::from_utf8(&metadata.stdout).context("GNOME extension metadata is not UTF-8")?;
```

Proposed:

```rust
let metadata_output = host.curl("GNOME extension metadata", &endpoint, std::iter::empty::<&str>())?;
let shell_output = host.run("GNOME extension shell version", "gnome-shell", ["--version"])?;
let shell_text = std::str::from_utf8(&shell_output.stdout).context("GNOME Shell version is not UTF-8")?;
let shell_version = shell_version(shell_text)?;
let metadata = std::str::from_utf8(&metadata_output.stdout).context("GNOME extension metadata is not UTF-8")?;
```

The names distinguish command output, decoded text, and parsed version.

### `src/operations/desktop/gnome.rs:97-106`

Current:

```rust
for part in input.split_whitespace() {
    let part = part.trim_matches(|character: char| !character.is_ascii_digit() && character != '.');
    if !part.is_empty()
        && part
            .split('.')
            .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Ok(part);
    }
}
```

Proposed:

```rust
for token in input.split_whitespace() {
    let candidate = token.trim_matches(|character: char| !character.is_ascii_digit() && character != '.');
    let mut is_version = !candidate.is_empty();
    for component in candidate.split('.') {
        if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
            is_version = false;
            break;
        }
    }
    if is_version {
        return Ok(candidate);
    }
}
```

### `src/operations/integrations/docker.rs:14-15`

Current:

```rust
let log_options = daemon_config.entry("log-opts").or_insert_with(|| Value::Object(Map::new()));
let log_options = log_options.as_object_mut().context("Docker daemon config log-opts must be a JSON object")?;
```

Proposed:

```rust
let log_options_value = daemon_config.entry("log-opts").or_insert_with(|| Value::Object(Map::new()));
let log_options_context = "Docker daemon config log-opts must be a JSON object";
let log_options = log_options_value.as_object_mut().context(log_options_context)?;
```

### `src/operations/integrations/docker.rs:31-32`

Current:

```rust
let mode = stdout_line(&stat_output.stdout, "sudo stat")?;
let mode = u32::from_str_radix(mode, 16).context("sudo stat returned malformed mode output")?;
```

Proposed:

```rust
let mode_hex = stdout_line(&stat_output.stdout, "sudo stat")?;
let mode = u32::from_str_radix(mode_hex, 16).context("sudo stat returned malformed mode output")?;
```

## Dotfile operations

### `src/operations/dotfiles.rs:21-22`

Current:

```rust
let metadata =
    fs::symlink_metadata(&source).with_context(|| format!("dotfiles package {package:?} does not exist"))?;
```

Proposed:

```rust
let metadata_context = || format!("dotfiles package {package:?} does not exist");
let metadata = fs::symlink_metadata(&source).with_context(metadata_context)?;
```

### `src/operations/dotfiles.rs:71-72`

Current:

```rust
let source_metadata =
    fs::symlink_metadata(source).with_context(|| format!("read dotfiles source metadata {}", source.display()))?;
```

Proposed:

```rust
let metadata_context = || format!("read dotfiles source metadata {}", source.display());
let source_metadata = fs::symlink_metadata(source).with_context(metadata_context)?;
```

### `src/operations/dotfiles.rs:100-102`

Current:

```rust
fs::canonicalize(target).and_then(|target| fs::canonicalize(source).map(|source| target == source)).unwrap_or(false)
```

Proposed:

```rust
let Ok(target) = fs::canonicalize(target) else {
    return false;
};
let Ok(source) = fs::canonicalize(source) else {
    return false;
};
target == source
```

### `src/operations/dotfiles.rs:108-113`

Current:

```rust
let state_home =
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| host.home().join(".local/state"));
let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .context("dotfiles backup timestamp is before the Unix epoch")?
    .as_nanos();
```

Proposed:

```rust
let state_home = match std::env::var_os("XDG_STATE_HOME") {
    Some(path) => PathBuf::from(path),
    None => host.home().join(".local/state"),
};
let timestamp_context = "dotfiles backup timestamp is before the Unix epoch";
let now = SystemTime::now();
let elapsed = now.duration_since(UNIX_EPOCH).context(timestamp_context)?;
let timestamp = elapsed.as_nanos();
```

### `src/operations/dotfiles.rs:122-123`

Current:

```rust
fs::rename(conflict, &backup)
    .with_context(|| format!("move dotfiles conflict {} to {}", conflict.display(), backup.display()))?;
```

Proposed:

```rust
let move_context = || format!("move dotfiles conflict {} to {}", conflict.display(), backup.display());
fs::rename(conflict, &backup).with_context(move_context)?;
```

## Package operations

### `src/operations/packages/apt/repo.rs:149-163`

Current:

```rust
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
```

Proposed:

```rust
let mut replacement = String::with_capacity(text.len());
for line in text.split_inclusive('\n') {
    let body = line.strip_suffix('\n').unwrap_or(line);
    let Some((name, values)) = body.split_once(':') else {
        replacement.push_str(line);
        continue;
    };
    if !name.eq_ignore_ascii_case("Components") {
        replacement.push_str(line);
        continue;
    }
    let values = values.split_ascii_whitespace().collect::<Vec<_>>();
    if !values.contains(&"main") {
        replacement.push_str(line);
        continue;
    }
    let line = append_missing(body, body.len(), &values, line.ends_with('\n'));
    replacement.push_str(&line);
}
replacement
```

### `src/operations/packages/apt/repo.rs:166-183`

Current:

```rust
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
```

Proposed:

```rust
let mut replacement = String::with_capacity(text.len());
for line in text.split_inclusive('\n') {
    let body = line.strip_suffix('\n').unwrap_or(line);
    let comment = body.find('#').unwrap_or(body.len());
    let active = body[..comment].trim();
    let fields = active.split_ascii_whitespace().collect::<Vec<_>>();
    if fields.first() != Some(&"deb") {
        replacement.push_str(line);
        continue;
    }
    let uri = fields.iter().position(|field| field.starts_with("http://") || field.starts_with("https://"));
    let Some(uri) = uri else {
        replacement.push_str(line);
        continue;
    };
    if !debian_uri(fields[uri]) || fields.len() <= uri + 2 || !fields[uri + 2..].contains(&"main") {
        replacement.push_str(line);
        continue;
    }
    let line = append_missing(body, comment, &fields[uri + 2..], line.ends_with('\n'));
    replacement.push_str(&line);
}
replacement
```

The URI `position` iterator remains because first-match search is clearer than a manual indexed loop.

### `src/operations/packages/apt/repo.rs:186-195`

Current:

```rust
[
    "http://deb.debian.org/debian",
    "https://deb.debian.org/debian",
    "http://deb.debian.org/debian-security",
    "https://deb.debian.org/debian-security",
    "http://security.debian.org/debian-security",
    "https://security.debian.org/debian-security",
]
.contains(&uri.trim_end_matches('/'))
```

Proposed:

```rust
let supported = [
    "http://deb.debian.org/debian",
    "https://deb.debian.org/debian",
    "http://deb.debian.org/debian-security",
    "https://deb.debian.org/debian-security",
    "http://security.debian.org/debian-security",
    "https://security.debian.org/debian-security",
];
supported.contains(&uri.trim_end_matches('/'))
```

The multiline array is acceptable data; only the attached method continuation is removed.

### `src/operations/packages/binary/mod.rs:62`

Current:

```rust
let matches = release.assets.iter().filter(|asset| pattern.is_match(&asset.name)).collect::<Vec<_>>();
```

Proposed:

```rust
let mut matches = Vec::new();
for asset in &release.assets {
    if pattern.is_match(&asset.name) {
        matches.push(asset);
    }
}
```

### `src/operations/packages/binary/appimage.rs:7-9`

Current:

```rust
let parent = destination
    .parent()
    .with_context(|| format!("AppImage destination has no parent: {}", destination.display()))?;
```

Proposed:

```rust
let parent = destination.parent();
let missing_parent = || format!("AppImage destination has no parent: {}", destination.display());
let parent = parent.with_context(missing_parent)?;
```

### `src/operations/packages/binary/appimaged.rs:56-57`

Current:

```rust
let package =
    if host.output("apt-cache", ["show", "libfuse2t64"])?.status.success() { "libfuse2t64" } else { "libfuse2" };
```

Proposed:

```rust
let output = host.output("apt-cache", ["show", "libfuse2t64"])?;
let package = if output.status.success() { "libfuse2t64" } else { "libfuse2" };
```

## Toolchain operations

### `src/operations/toolchains/go.rs:16-20`

Current:

```rust
if !regular_executable_file(program.as_ref())
    || !host.output(program, ["version"]).is_ok_and(|output| {
        output.status.success() && std::str::from_utf8(&output.stdout).is_ok_and(|stdout| stdout.trim() == expected)
    })
{
```

Proposed:

```rust
let installed = regular_executable_file(program.as_ref());
let mut version_matches = false;
if installed {
    if let Ok(output) = host.output(program, ["version"]) {
        if output.status.success() {
            if let Ok(stdout) = std::str::from_utf8(&output.stdout) {
                version_matches = stdout.trim() == expected;
            }
        }
    }
}
if !version_matches {
```

Executable detection, command execution, decoding, and comparison become visible stages.

### `src/operations/toolchains/fnm.rs:45`

Current:

```rust
host.run("fnm default", &fnm, ["default", "--", if selector == "lts" { "lts-latest" } else { selector }])?;
```

Proposed:

```rust
let default_selector = if selector == "lts" { "lts-latest" } else { selector };
host.run("fnm default", &fnm, ["default", "--", default_selector])?;
```

## Deliberate non-findings

The following remain unchanged because they are already concise and semantically clear:

- `src/config.rs:339`: ordered `find_map` lookup
- `src/init.rs:125`: ordered iterator equality
- `src/operations/packages/cargo.rs:57-63`: one `filter_map` with direct parse-and-discard semantics
- `src/operations/packages/apt/repo.rs:199`: one-line missing-component filter
- `src/workflow.rs:357`: one-line package-list merge
- multiline arrays, argument lists, struct literals, match data, and raw strings throughout `src/`
