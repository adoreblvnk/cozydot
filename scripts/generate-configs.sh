#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
base="$root/configs/cozydot.yaml"
mode="${1:-write}"

if (( $# > 1 )) || [[ "$mode" != "write" && "$mode" != "--check" ]]; then
  printf 'usage: %s [--check]\n' "${0##*/}" >&2
  exit 2
fi

temporary="$(mktemp -d)"
trap 'rm -rf -- "$temporary"' EXIT

generate_cli() {
  yq '
    del(
      .linux.system.allowed_platforms.desktops,
      .linux.system.ubuntu.restricted_extras,
      .linux.packages.flatpak,
      .linux.desktop,
      .linux.updates.flatpak
    ) |
    .linux.integrations = {} |
    .linux.packages.apt.install -= ["ffmpeg", "imagemagick", "vlc"] |
    .linux.packages.apt.repos |= map(select(.name == "github-cli")) |
    .shared.packages.npm = ["opencode-ai"] |
    .shared.integrations.vscode.extensions = [] |
    .linux.packages.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .shared.dotfiles.packages -= ["opencode", "vscode", "wezterm"]
  ' "$base"
}

generate_vm() {
  yq '
    .linux.system.allowed_platforms = {
      "distros": ["ubuntu", "debian"],
      "desktops": ["gnome"]
    } |
    del(
      .linux.integrations.docker,
      .linux.integrations.virtualbox
    ) |
    .linux.packages.apt.repos |= map(select(.name == "vscode" or .name == "wezterm")) |
    .linux.packages.flatpak = ["com.bitwarden.desktop"] |
    .shared.packages.cargo = ["bat", "fd-find", "starship", "tealdeer"] |
    .shared.packages.npm = ["opencode-ai"] |
    .linux.packages.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .shared.tools.node = "latest" |
    .shared.dotfiles.packages -= ["opencode"] |
    .shared.integrations.vscode.extensions = ["catppuccin.catppuccin-vsc"] |
    .linux.updates.apt = "full-upgrade"
  ' "$base"
}

generate_cli >"$temporary/cli.yaml"
generate_vm >"$temporary/vm.yaml"

status=0
for preset in cli vm; do
  generated="$temporary/$preset.yaml"
  committed="$root/configs/$preset.yaml"
  if [[ "$mode" == "--check" ]]; then
    if ! cmp -s -- "$generated" "$committed"; then
      printf 'configs/%s.yaml is stale; run scripts/generate-configs.sh\n' "$preset" >&2
      status=1
    fi
  else
    install -m 0644 "$generated" "$committed"
  fi
done
exit "$status"
