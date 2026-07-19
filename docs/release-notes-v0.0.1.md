# Cozydot 0.0.1

This release replaces earlier configuration formats with strict version `1.0.0`. Superseded YAML is not loaded, converted, or interpreted.

Existing configurations must be rewritten using the [configuration reference](configuration.md). To start from the new embedded default while preserving the old file for reference:

```bash
root="${XDG_CONFIG_HOME:-$HOME/.config}/cozydot"
mv "$root/cozydot.yaml" "$root/cozydot.yaml.legacy"
cozydot init
```

The public workflow remains `install -> init -> edit cozydot.yaml -> apply`. Version `1.0.0` uses fixed managers and typed operations; YAML cannot select shell commands, managers, profiles, plugins, interpolation variables, or lock paths.

Docker, VirtualBox, and VS Code integrations configure existing products only. They do not implicitly install those products.

The release target is Debian, Ubuntu, Pop!_OS, and Linux Mint on `x86_64`, `aarch64`, and 32-bit ARMv7 hosts. Configuration uses the canonical selector keys `amd64`, `arm64`, and `arm32`; other architectures are rejected. Managed base APT sources are intentionally limited to pure Ubuntu, pure Debian, and Kali; derivatives preserve distro-owned sources.
