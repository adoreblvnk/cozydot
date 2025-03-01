    # cinnamon settings
    if [[ $DESKTOP_ENVIRONMENT == "cinnamon" ]]; then
      if [[ $(yq ".configure.DE.cinnamon.defaultTerm | tag" "$CONFIG") == "!enabled" ]]; then
        local term=$(yq ".configure.DE.cinnamon.defaultTerm" "$CONFIG")
        _info_msg "Setting default terminal to $term"
        gsettings set org.cinnamon.desktop.default-applications.terminal exec "$term"
        gsettings set org.cinnamon.desktop.default-applications.terminal exec-arg ''
      fi

      # install & autostart plank dock
      if [[ $(yq ".configure.DE.cinnamon.plankDock" "$CONFIG") == true ]]; then
        if ! command -v plank &>/dev/null; then
          _info_msg "Installing plank"
          sudo apt-get update -qq && sudo apt-get install -qq plank
        fi

        _info_msg "Adding plank to startup"
        # sleep to let apps load before adding to plank
        cat <<EOF >"$HOME/.config/autostart/plank.desktop"
[Desktop Entry]
Name=Plank
GenericName=Dock
Comment=Stupidly simple.
Categories=Utility;
Type=Application
Exec=bash -c "sleep 5 && plank"
Icon=plank
Terminal=false
NoDisplay=false
EOF
      fi

      # convert cinnamon panel to MacOS-like menu bar
      if [[ $(yq ".configure.DE.cinnamon.panelToMenuBar" "$CONFIG") == true ]]; then
        _info_msg "Converting panel to menu bar"
        # panel applets in order
        # https://support.apple.com/en-al/guide/mac-help/mchlp1446
        # https://support.apple.com/en-kw/guide/mac-help/mchlad96d366
        gsettings set org.cinnamon enabled-applets "[ \
          'panel1:left:0:menu@cinnamon.org:1', \
          'panel1:right:1:xapp-status@cinnamon.org:2', \
          'panel1:right:2:notifications@cinnamon.org:3', \
          'panel1:right:3:printers@cinnamon.org:4', \
          'panel1:right:4:removable-drives@cinnamon.org:5', \
          'panel1:right:5:keyboard@cinnamon.org:6', \
          'panel1:right:6:power@cinnamon.org:7', \
          'panel1:right:7:sound@cinnamon.org:8', \
          'panel1:right:8:network@cinnamon.org:9', \
          'panel1:right:9:calendar@cinnamon.org:10' \
        ]"
        # panel icon sizes
        gsettings set org.cinnamon panel-zone-symbolic-icon-sizes \
          '[{"left": 25, "center": 25, "right": 20, "panelId": 1}]'
        # shift panel position to top
        gsettings set org.cinnamon panels-enabled "['1:0:top']"
        # smaller panel height
        gsettings set org.cinnamon panels-height "['1:25']"
      fi

      # install gTile tiling assistant
      if [[ $(yq ".configure.DE.cinnamon.gTile | tag" "$CONFIG") == "!enabled" ]]; then
        if [[ ! -d ~/.local/share/cinnamon/extensions/gTile@shuairan ]]; then
          _info_msg "Downloading gTile extension"
          curl -sSL -o /tmp/gTile@shuairan.zip \
            https://cinnamon-spices.linuxmint.com/files/extensions/gTile@shuairan.zip
          unzip /tmp/gTile@shuairan.zip \
            -d ~/.local/share/cinnamon/extensions >/dev/null
        fi
        if ! gsettings get org.cinnamon enabled-extensions | grep -q "gTile"; then
          _warning_msg "Enabling gTile will disable other extensions"
          gsettings set org.cinnamon enabled-extensions "['gTile@shuairan']"
        else
          _info_msg "gTile already enabled"
        fi

        # opinionated tiling layout
        if [[ $(yq ".configure.DE.cinnamon.gTile.tilingLayout" "$CONFIG") == true ]]; then
          _info_msg "Configuring opinionated tiling layout for gTile"
          mkdir -p ~/.local/share/cinnamon/extensions/gTile@shuairan
          cp "$(dirname "$0")/extras/gTile@shuairan.json" \
            ~/.local/share/cinnamon/extensions/gTile@shuairan/gTile@shuairan.json
        fi
      fi
    fi

