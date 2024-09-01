# bash aliases
alias c=clear
alias py=python
alias pip="python -m pip"

alias cat="bat -pp"

# eza aliases
alias ls="eza --group-directories-first --icons=auto"
alias la="eza --group-directories-first --icons=auto -a"
alias ll="eza --group-directories-first --icons=auto -al"
alias tree="eza --group-directories-first --icons=auto -T"

# Create alias if flatpak exists. Disable if .bashrc takes too long to load.
# Arguments:
#   flatpak ID
#   alias name
flatpak_alias() {
  if flatpak info "$1" &>/dev/null; then alias "$2"="flatpak run $1"; fi
}
flatpak_alias com.discordapp.Discord discord
flatpak_alias com.obsproject.Studio obs
flatpak_alias org.qbittorrent.qBittorrent qbittorrent
flatpak_alias org.shotcut.Shotcut shotcut
flatpak_alias org.telegram.desktop tlgrm

# Create AppImage aliases. Looks for AppImages in ~/Applications directory &
# excludes the appimaged daemon. Uses fd if available for speed.
appimage_alias() {
  local appimages
  mapfile -t appimages < <(
    if command -v fd &>/dev/null; then
      fd -E '*appimaged*' -t f -e AppImage '.' ~/Applications/
    else
      find ~/Applications/ -name '*.AppImage' -type f ! -name '*appimaged*'
    fi
  )
  for app in "${appimages[@]}"; do
    local app_with_ext=$(basename "$app")
    # remove file extension & convert to lowercase
    local app_name=$(echo "${app_with_ext::-9}" | awk '{print tolower($0)}')
    alias "$app_name"="$app"
  done
}
appimage_alias
