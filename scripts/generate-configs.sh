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

generate_full() {
  cp "$base" "$temporary/full.yaml"
}

generate_cli() {
  yq '
    del(
      .os.linux.system.require.desktops,
      .os.linux.system.ubuntu.codecs,
      .os.linux.packages.flatpak,
      .os.linux.desktop,
      .os.linux.updates.flatpak
    ) |
    .os.linux.integrations = {} |
    .os.linux.packages.apt.install -= ["ffmpeg", "imagemagick", "vlc"] |
    .os.linux.packages.apt.repositories |= map(select(.name == "github-cli")) |
    .shared.packages.npm = ["opencode-ai"] |
    .os.linux.packages.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .shared.dotfiles.packages -= ["opencode", "vscode", "wezterm"]
  ' "$base"
}

generate_vm() {
  yq '
    .os.linux.system.require = {
      "distros": ["ubuntu", "debian"],
      "desktops": ["gnome"]
    } |
    del(
      .os.linux.integrations.docker,
      .os.linux.integrations.virtualbox
    ) |
    .os.linux.packages.apt.repositories |= map(select(.name == "vscode" or .name == "wezterm")) |
    .os.linux.packages.flatpak = ["com.bitwarden.desktop"] |
    .shared.packages.cargo = ["bat", "fd-find", "starship", "tealdeer"] |
    .shared.packages.npm = ["opencode-ai"] |
    .os.linux.packages.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .shared.tools.node = "latest" |
    .shared.dotfiles.packages -= ["opencode"] |
    .shared.integrations.vscode.extensions = ["catppuccin.catppuccin-vsc"] |
    .os.linux.updates.apt = "full"
  ' "$base"
}

generate_full
generate_cli >"$temporary/cli.yaml"
generate_vm >"$temporary/vm.yaml"

status=0
for preset in full cli vm; do
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
