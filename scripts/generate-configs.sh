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

# install mikefarah/yq when absent; existing installs & CI runners are left alone
ensure_yq() {
  if command -v yq >/dev/null 2>&1; then
    return
  fi
  status "Installing yq"

  case "$(uname -m)" in
    x86_64) YQ_ARCH=amd64 ;;
    aarch64 | arm64) YQ_ARCH=arm64 ;;
    *) error "unsupported architecture for yq: $(uname -m)" ;;
  esac

  curl -fsSL "https://github.com/mikefarah/yq/releases/latest/download/yq_linux_$YQ_ARCH.tar.gz" -o "$TEMP/yq.tar.gz"
  mkdir -p "${HOME}/.local/bin"
  # archive holds ./yq_linux_<arch> beside its man page; extract all & install the binary
  tar -xzf "$TEMP/yq.tar.gz" -C "$TEMP"
  install -m 0755 "$TEMP/yq_linux_$YQ_ARCH" "${HOME}/.local/bin/yq"
}

generate_cli() {
  yq '
    .packages.linux.apt.install -= ["vlc"] |
    .packages.linux.apt.repos |= map(select(.name == "github-cli")) |
    .packages.linux.flatpak = [] |
    .packages.linux.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .packages.macos.homebrew.casks = ["git-credential-manager"] |
    .dotfiles.packages.all -= ["wezterm"] |
    .dotfiles.packages.linux -= ["vscode-linux"] |
    .dotfiles.packages.macos -= ["vscode-macos"] |
    .integrations.vscode.extensions = [] |
    .integrations.linux = {} |
    del(.desktop) |
    .updates.packages.linux.flatpak = false
  ' "$BASE"
}

generate_vm() {
  yq '
    .packages.linux.apt.repos |= map(select(.name == "vscode" or .name == "wezterm")) |
    .packages.linux.flatpak = ["com.bitwarden.desktop"] |
    .packages.linux.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .packages.macos.homebrew.casks = [
      "bitwarden",
      "git-credential-manager",
      "visual-studio-code",
      "wezterm"
    ] |
    .tools.cargo = ["bat", "fd-find", "starship", "tealdeer"] |
    .tools.npm = ["opencode-ai"] |
    .dotfiles.packages.all -= ["bottom", "opencode", "yazi"] |
    .dotfiles.packages.macos -= ["vscode-macos"] |
    .integrations.skills = [] |
    .integrations.vscode.extensions = ["catppuccin.catppuccin-vsc"] |
    .integrations.linux = {}
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
  ensure_yq
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
