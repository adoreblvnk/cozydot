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
      .system.ubuntu.restricted_extras,
      .packages.linux.flatpak,
      .desktop,
      .updates.packages.linux.flatpak
    ) |
    .integrations.linux = {} |
    .packages.linux.apt.install -= ["ffmpeg", "imagemagick", "vlc"] |
    .packages.linux.apt.repos |= map(select(.name == "github-cli")) |
    .tools.npm = ["opencode-ai"] |
    .integrations.vscode.extensions = [] |
    .packages.linux.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .dotfiles.packages.all -= ["opencode", "vscode", "wezterm"] |
    .packages.macos.homebrew.casks = ["git-credential-manager"]
  ' "$base"
}

generate_vm() {
  yq '
    .integrations.linux = {} |
    .packages.linux.apt.repos |= map(select(.name == "vscode" or .name == "wezterm")) |
    .packages.linux.flatpak = ["com.bitwarden.desktop"] |
    .tools.cargo = ["bat", "fd-find", "starship", "tealdeer"] |
    .tools.npm = ["opencode-ai"] |
    .packages.linux.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .tools.node = "latest" |
    .dotfiles.packages.all -= ["bottom", "opencode", "yazi"] |
    .integrations.vscode.extensions = ["catppuccin.catppuccin-vsc"] |
    .packages.macos.homebrew.casks = [
      "bitwarden",
      "git-credential-manager",
      "visual-studio-code",
      "wezterm"
    ]
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
      printf 'error: configs/%s.yaml is stale; run scripts/generate-configs.sh\n' "$preset" >&2
      status=1
    fi
  else
    install -m 0644 "$generated" "$committed"
  fi
done
exit "$status"
