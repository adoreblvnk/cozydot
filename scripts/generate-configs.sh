#!/bin/sh
set -eu

# ----- GLOBAL VARIABLES -----

ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
readonly ROOT
readonly BASE="$ROOT/configs/cozydot.yaml"
CHECK=false

# ----- PRINT FUNCTIONS -----

status() { printf '%s\n' "$1" >&2; }
warning() { printf 'warning: %s\n' "$1" >&2; }
error() { printf 'error: %s\n' "$1" >&2; exit 1; }

# ----- HELPERS -----

cleanup() { [ -z "$TEMP" ] || rm -rf "$TEMP"; }

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
  ' "$BASE"
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
  ' "$BASE"
}

# ----- MAIN -----

main() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --check) CHECK=true; shift ;;
      *) error "unexpected argument: $1" ;;
    esac
  done

  TEMP=$(mktemp -d)
  trap cleanup 0 # remove temp dir on exit
  generate_cli >"$TEMP/cli.yaml"
  generate_vm >"$TEMP/vm.yaml"

  for PRESET in cli vm; do
    GENERATED="$TEMP/$PRESET.yaml"
    COMMITTED="$ROOT/configs/$PRESET.yaml"
    if [ "$CHECK" = false ]; then
      # copy preset with 0644 permissions
      install -m 0644 "$GENERATED" "$COMMITTED"
    elif ! cmp -s "$GENERATED" "$COMMITTED"; then
      error "configs/$PRESET.yaml is stale; run scripts/generate-configs.sh"
    fi
  done
}

main "$@"
