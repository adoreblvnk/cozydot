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
  yq '
    .packages.apt.install = (.packages.apt.install + [
      "ttf-mscorefonts-installer"
    ] | sort) |
    .packages.apt.repositories = (.packages.apt.repositories + [
      {
        "name": "helium",
        "key": "https://raw.githubusercontent.com/imputnet/helium-linux/main/pubkey.asc",
        "urls": {"default": "https://pkg.helium.computer/deb"},
        "suite": "stable",
        "components": ["main"],
        "packages": ["helium-bin"]
      },
      {
        "name": "onlyoffice",
        "key": "https://download.onlyoffice.com/GPG-KEY-ONLYOFFICE",
        "urls": {"default": "https://download.onlyoffice.com/repo/debian"},
        "suite": "squeeze",
        "components": ["main"],
        "packages": ["onlyoffice-desktopeditors"]
      },
      {
        "name": "virtualbox",
        "key": "https://www.virtualbox.org/download/oracle_vbox_2016.asc",
        "urls": {"default": "https://download.virtualbox.org/virtualbox/debian"},
        "suite": "system",
        "components": ["contrib"],
        "packages": ["virtualbox-7.1"]
      }
    ] | sort_by(.name)) |
    .packages.flatpak = (.packages.flatpak + [
      "org.shotcut.Shotcut"
    ] | sort) |
    .packages.npm |= sort |
    .packages.binaries = [
      {
        "name": "drawio",
        "format": "deb",
        "commands": ["drawio"],
        "source": {
          "provider": "github",
          "repository": "jgraph/drawio-desktop",
          "assets": {
            "amd64": "^drawio-amd64-.*\\.deb$",
            "arm64": "^drawio-arm64-.*\\.deb$"
          }
        }
      },
      {
        "name": "fastfetch",
        "format": "deb",
        "commands": ["fastfetch"],
        "source": {
          "provider": "github",
          "repository": "fastfetch-cli/fastfetch",
          "assets": {
            "amd64": "^fastfetch-linux-amd64\\.deb$",
            "arm64": "^fastfetch-linux-aarch64\\.deb$",
            "arm32": "^fastfetch-linux-armv7l.*\\.deb$"
          }
        }
      },
      {
        "name": "git-credential-manager",
        "format": "deb",
        "commands": ["git-credential-manager"],
        "source": {
          "provider": "github",
          "repository": "git-ecosystem/git-credential-manager",
          "assets": {
            "amd64": "^gcm-linux-x64-.*\\.deb$",
            "arm64": "^gcm-linux-arm64-.*\\.deb$"
          }
        }
      },
      {
        "name": "obsidian",
        "format": "appimage",
        "commands": ["obsidian"],
        "source": {
          "provider": "github",
          "repository": "obsidianmd/obsidian-releases",
          "assets": {
            "amd64": "^Obsidian-[0-9]+(?:\\.[0-9]+)+\\.AppImage$",
            "arm64": "^Obsidian-[0-9]+(?:\\.[0-9]+)+-arm64\\.AppImage$"
          }
        }
      },
      {
        "name": "zen-browser",
        "format": "appimage",
        "commands": ["zen"],
        "source": {
          "provider": "github",
          "repository": "zen-browser/desktop",
          "assets": {
            "amd64": "^zen-x86_6[0-9]\\.AppImage$",
            "arm64": "^zen-aarch6[0-9]\\.AppImage$"
          }
        }
      }
    ] |
    .integrations.docker = {
      "add_user_to_group": true,
      "logging": {"driver": "local", "max_size": "10m"}
    } |
    .integrations.virtualbox = {"add_user_to_group": true} |
    .integrations.vscode.extensions = (.integrations.vscode.extensions + [
      "christian-kohler.path-intellisense",
      "ecmel.vscode-html-css",
      "foxundermoon.shell-format",
      "golang.go",
      "llvm-vs-code-extensions.vscode-clangd",
      "ms-python.debugpy",
      "ms-python.vscode-pylance",
      "prettier.prettier-vscode",
      "rust-lang.rust-analyzer",
      "streetsidesoftware.code-spell-checker",
      "timonwong.shellcheck",
      "wayou.vscode-todo-highlight"
    ] | sort) |
    .desktop.terminal = "wezterm" |
    .updates.apt = "full" |
    .updates.packages.binaries = true |
    .updates.fonts = true |
    .packages.apt as $apt |
    .packages.apt = {
      "remove": $apt.remove,
      "install": $apt.install,
      "repositories": $apt.repositories
    }
  ' "$base"
}

generate_cli() {
  yq '
    del(
      .system.ubuntu.codecs,
      .packages.apt.remove,
      .packages.flatpak,
      .integrations,
      .desktop,
      .updates.flatpak
    ) |
    .packages.apt.install -= ["ffmpeg", "imagemagick", "vlc"] |
    .packages.apt.repositories |= map(select(.name == "github-cli")) |
    .packages.npm = ["opencode-ai"] |
    .packages.binaries |= map(select(
      .name == "fastfetch" or .name == "git-credential-manager"
    )) |
    .dotfiles.packages -= ["opencode", "vscode", "wezterm"]
  ' "$base"
}

generate_vm() {
  yq '
    .system.require = {
      "distros": ["ubuntu", "debian"],
      "desktops": ["gnome"]
    } |
    .system.apt.sources = {
      "mode": "managed",
      "components": {
        "ubuntu": ["main", "restricted", "universe", "multiverse"],
        "debian": ["main", "contrib", "non-free", "non-free-firmware"]
      }
    } |
    .packages.apt.install = (.packages.apt.install + [
      "ttf-mscorefonts-installer"
    ] | sort) |
    del(
      .packages.apt.remove,
      .integrations.docker,
      .integrations.virtualbox
    ) |
    .packages.apt.repositories = ((.packages.apt.repositories | map(select(.name == "vscode"))) + [
      {
        "name": "wezterm",
        "key": "https://apt.fury.io/wez/gpg.key",
        "urls": {"default": "https://apt.fury.io/wez/"},
        "suite": "*",
        "components": ["*"],
        "packages": ["wezterm-nightly"]
      }
    ] | sort_by(.name)) |
    .packages.flatpak = ["com.bitwarden.desktop"] |
    .packages.cargo = ["bat", "fd-find", "starship", "tealdeer"] |
    .packages.npm = ["opencode-ai"] |
    .packages.binaries = [
      {
        "name": "fastfetch",
        "format": "deb",
        "commands": ["fastfetch"],
        "source": {
          "provider": "github",
          "repository": "fastfetch-cli/fastfetch",
          "assets": {
            "amd64": "^fastfetch-linux-amd64\\.deb$",
            "arm64": "^fastfetch-linux-aarch64\\.deb$",
            "arm32": "^fastfetch-linux-armv7l.*\\.deb$"
          }
        }
      },
      {
        "name": "git-credential-manager",
        "format": "deb",
        "commands": ["git-credential-manager"],
        "source": {
          "provider": "github",
          "repository": "git-ecosystem/git-credential-manager",
          "assets": {
            "amd64": "^gcm-linux-x64-.*\\.deb$",
            "arm64": "^gcm-linux-arm64-.*\\.deb$"
          }
        }
      }
    ] |
    .tools.node = "latest" |
    .dotfiles.packages -= ["opencode"] |
    .integrations.vscode.extensions = ["catppuccin.catppuccin-vsc"] |
    .desktop.terminal = "wezterm" |
    .updates.apt = "full" |
    .updates.packages.binaries = true |
    .updates.fonts = true
  ' "$base"
}

generate_full >"$temporary/full.yaml"
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
