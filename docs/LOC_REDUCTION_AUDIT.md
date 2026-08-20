# Remaining LOC reductions

These are the proposals not selected from the full `src/` audit. Approved reductions have been applied and removed from this report.

Every remaining proposal preserves behavior, survives rustfmt, uses no more physical lines, and keeps extracted locals on 1 line.

## `src/operations/host/privileged_file.rs:22-55`

Current: 32 LOC. Proposed: 10 LOC. Reduction: 22 LOC.

```rust
let install = OsStr::new("install");
let owner = OsStr::new("-o");
let group = OsStr::new("-g");
let root = OsStr::new("root");
let mode = OsStr::new("-m");
let separator = OsStr::new("--");
let args = [install, "-d".as_ref(), owner, root, group, root, mode, "0755".as_ref(), separator, parent_arg];
host.run(label, "sudo", args)?;

let args = [install, owner, root, group, root, mode, "0644".as_ref(), separator, local_arg, staged_arg];
host.run(label, "sudo", args)?;
```

## `src/operations/toolchains/rustup.rs:14-35`

Current: 22 LOC. Proposed: 6 LOC. Reduction: 16 LOC, plus removal of the unused `OsStr` import.

```rust
let path = installer.path().as_os_str();
let curl_args = ["--proto".as_ref(), "=https".as_ref(), "--tlsv1.2".as_ref(), "--output".as_ref(), path];
host.curl("rustup installer download", "https://sh.rustup.rs", curl_args)?;
let default_arg = "--default-toolchain";
let install_args = [path, "-y".as_ref(), "--no-modify-path".as_ref(), default_arg.as_ref(), selector.as_ref()];
host.run("rustup install", "sh", install_args)?;
```

## `src/operations/packages/binary/mod.rs:77-90`

Current: 13 LOC. Proposed: 5 LOC. Reduction: 8 LOC.

```rust
let frontend = OsStr::new("DEBIAN_FRONTEND=noninteractive");
let apt_get = OsStr::new("apt-get");
let install = OsStr::new("install");
let args = [frontend, apt_get, install, "-y".as_ref(), "-qq".as_ref(), "--".as_ref(), temp.path().as_os_str()];
host.run("Deb package install", "sudo", args)?;
```

`OsStr` can be added to the existing 1-line `std` import without increasing import LOC.

## `src/operations/packages/snapd.rs:27-39`

Current: 13 LOC. Proposed: 4 LOC. Reduction: 9 LOC.

```rust
let home_snap = home_snap.as_os_str();
let snapd = OsStr::new("/var/lib/snapd");
let args = ["rm".as_ref(), "-rf".as_ref(), "--".as_ref(), home_snap, "/snap".as_ref(), "/var/snap".as_ref(), snapd];
host.run("snap data removal", "sudo", args)?;
```

`OsStr` can be added to the existing 1-line `std` import without increasing import LOC.

## Toolchain executable calls

### `src/operations/toolchains/uv.rs:24-36`

Current: 11 LOC. Proposed: 5 LOC. Reduction: 6 LOC.

```rust
let error = "uv python install: uv is unavailable after install";
let uv = require_regular_executable(&host.home().join(".local/bin/uv"), "managed tool executable path", error)?;
let args = ["python", "install", "--no-config", "--managed-python", "--no-progress", "--default", "--", selector];
host.run("uv python install", &uv, args)?;
Ok(())
```

### `src/operations/toolchains/uv.rs:38-47`

Current: 8 LOC. Proposed: 5 LOC. Reduction: 3 LOC.

```rust
let error = "Python version upgrade: uv is unavailable after install";
let uv = require_regular_executable(&host.home().join(".local/bin/uv"), "managed tool executable path", error)?;
host.run("uv self update", &uv, ["self", "update"])?;
host.run("Python version upgrade", &uv, ["python", "upgrade", "--no-config", "--managed-python", "--no-progress"])?;
Ok(())
```

### `src/operations/toolchains/rustup.rs:47-55`

Current: 7 LOC. Proposed: 5 LOC. Reduction: 2 LOC.

```rust
let path = host.home().join(".cargo/bin/rustup");
let error = "Rust toolchain update: rustup is unavailable after install";
let rustup = require_regular_executable(&path, "managed tool executable path", error)?;
host.run("Rust toolchain update", &rustup, ["update"])?;
Ok(())
```

## Binary path contexts

### `src/operations/packages/binary/appimaged.rs:29-33`

Current: 5 LOC. Proposed: 3 LOC. Reduction: 2 LOC.

```rust
let context = || format!("appimaged path is not UTF-8: {}", destination.display());
let program = destination.to_str().with_context(context)?;
host.run("launch appimaged", program, std::iter::empty::<&str>())?;
```

### `src/operations/packages/binary/appimage.rs:7-9`

Current: 3 LOC. Proposed: 2 LOC. Reduction: 1 LOC.

```rust
let context = || format!("AppImage destination has no parent: {}", destination.display());
let parent = destination.parent().with_context(context)?;
```

## `src/operations/host/mod.rs`

### Curl options at lines 61-64

Current: 4 LOC. Proposed: 2 LOC. Reduction: 2 LOC.

Iterator use is necessary because the literals must become an owned mutable `Vec<OsString>` before caller arguments and the URL are appended.

```rust
let options = ["--location", "--fail", "--silent", "--show-error", "--retry", "3", "--retry-all-errors"];
let mut curl_args = options.into_iter().map(OsString::from).collect::<Vec<_>>();
```

### Command display at lines 141-146

Current: 5 LOC. Proposed: 3 LOC. Reduction: 2 LOC.

Iterator use is necessary because the program and argument slice are combined and converted to display text without another intermediate collection.

```rust
let parts = std::iter::once(OsStr::new(program)).chain(args.iter().map(OsString::as_os_str));
let parts = parts.map(|part| part.to_string_lossy());
parts.collect::<Vec<_>>().join(" ")
```

## `src/operations/packages/npm.rs:6-8`

Current: 3 LOC. Proposed: 1 LOC. Reduction: 2 LOC.

```rust
let Some(fnm) = fnm::find_executable(host)? else { bail!("npm install: managed fnm is unavailable after install") };
```

## Workflow closure reductions

The following 13 sites each reduce 3 LOC to 2 LOC by naming the closure on 1 line. Combined reduction: 13 LOC.

- `src/workflow.rs:125-127`
- `src/workflow.rs:137-139`
- `src/workflow.rs:195-197`
- `src/workflow.rs:206-208`
- `src/workflow.rs:266-268`
- `src/workflow.rs:287-289`
- `src/workflow.rs:303-305`
- `src/workflow.rs:309-311`
- `src/workflow.rs:318-320`
- `src/workflow.rs:367-369`
- `src/workflow.rs:373-375`
- `src/workflow.rs:382-384`
- `src/workflow.rs:430-432`

Representative proposal:

```rust
let operation = || apt::set_unattended_upgrades(host, state == EnabledDisabled::Enabled);
run("Apply", "unattended-upgrades set", operation)?;
```

These pass the LOC rule but introduce repeated generic `operation` locals.

## Excluded after rustfmt

`src/operations/packages/binary/appimaged.rs:19-23` was selected, but its proposed `args` local formats across 2 lines. The source remains unchanged under the 1-line-local rule.
