# Cozydot 0.0.1

This release replaces the tagged legacy configuration with strict schema v1. Legacy YAML is not loaded, converted, or interpreted.

Existing configurations must be rewritten using [`config-schema-v1.md`](config-schema-v1.md). To start from the new embedded default while preserving the old file for reference:

```bash
root="${XDG_CONFIG_HOME:-$HOME/.config}/cozydot"
mv "$root/cozydot.yaml" "$root/cozydot.yaml.legacy"
cozydot init
```

The public workflow remains `install -> init -> edit cozydot.yaml -> apply`. Schema v1 uses fixed managers and typed operations; YAML cannot select shell commands, managers, profiles, plugins, interpolation variables, or lock paths.

Docker, VirtualBox, and VS Code integrations configure existing products only. They do not implicitly install those products.

The release target is Debian, Ubuntu, Pop!_OS, and Linux Mint on amd64 and arm64. Managed base APT sources are intentionally limited to pure Ubuntu, pure Debian, and Kali; derivatives preserve distro-owned sources.
