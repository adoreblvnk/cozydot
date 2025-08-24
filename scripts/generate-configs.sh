#!/usr/bin/env bash
set -euo pipefail

# execute in the directory of cozydot
DEFAULT="$PWD/configs/default.yaml"
FULL="$PWD/configs/full.yaml"
CLI="$PWD/configs/cli.yaml"
VM="$PWD/configs/vm.yaml"
declare -a CONFIGS=("$FULL" "$CLI" "$VM")

for file in "${CONFIGS[@]}"; do
  if [[ -f $file ]]; then
    touch "$file"
  fi
done

yq ".metadata.description = \"All features enabled & apps installed.\" \
  | with(.check; \
    .pyenv.update = true \
    | .uv tag = \"!enabled\") \
  | .install.binaries += [ \
    {\"name\": \"zen.AppImage\", \
    \"url\": \"\$(curl -sSL https://api.github.com/repos/zen-browser/desktop/releases/latest | yq '.assets[].browser_download_url | select(. == \\\"*x86_64.AppImage\\\")')\"} \
  ] \
  | with(.update; \
    .apt.aptFull = true \
    | .cargo = true) \
  | .configure.apps.vscodeExtensions += [ \
    \"foxundermoon.shell-format\", \
    \"golang.go\", \
    \"rust-lang.rust-analyzer\", \
    \"timonwong.shellcheck\"]" "$DEFAULT" > "$FULL"

yq ".metadata.description = \"CLI utilities only. For use with WSL2 too.\" \
  | with(.check; .appimaged = false) \
  | with(.install; \
    .addRepos |= filter(.sourceName == \"github-cli\") \
    | .flatpak tag = \"!disabled\" | .flatpak |= [] \
    | .cargo |= filter(. != \"alacritty\") \
    | .binaries |= filter(.name == \"git-credential-manager.deb\")) \
  | with(.configure; \
    .dotfiles.packages |= filter(. != \"alacritty\" and . != \"vscode\") \
    | .apps.alacritty = false \
    | .apps.virtualbox = false \
    | .apps.vscodeExtensions tag = \"!disabled\" | .apps.vscodeExtensions |= [] \
    | .desktopEnvironment tag = \"!disabled\" \
    | .desktopEnvironment.common tag = \"!disabled\" \
    | .desktopEnvironment.gnome.tag = \"!disabled\" \
    | .desktopEnvironment.cinnamon.tag = \"!disabled\")" "$DEFAULT" >"$CLI"

yq ".metadata.description = \"Lightweight config with minimal utilities / apps installed for virtual machines.\" \
  | with(.install; \
    .addRepos |= filter( \
      .sourceName == \"mozilla\" \
      or .sourceName == \"vscode\") \
    | .flatpak tag = \"!disabled\" | .flatpak |= [] \
    | .cargo |= filter(
      . != \"du-dust\" \
      and . != \"fd-find\" \
      and . != \"tealdeer\" \
      and . != \"yazi-cli --locked\" \
      and . != \"yazi-fm --locked\") \
    | .binaries |= filter(.name == \"git-credential-manager.deb\")) \
  | with(.configure; \
    .dotfiles.packages |= filter(. != \"yazi\") \
    | .apps.docker = false \
    | .apps.virtualbox = false \
    | .apps.vscodeExtensions |= filter( \
      . == \"catppuccin.catppuccin-vsc\" \
      or . == \"visualstudioexptteam.intellicode-api-usage-examples\" \
      or . == \"visualstudioexptteam.vscodeintellicode\"))" "$DEFAULT" >"$VM"
