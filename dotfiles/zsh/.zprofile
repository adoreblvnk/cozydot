# ~/.zprofile: executed by zsh for login shells.
# macOS loads /etc/zprofile before this file.

# load env vars from ~/.config/cozydot/.env if exists
if [[ -r ~/.config/cozydot/.env ]]; then
  set -a
  source ~/.config/cozydot/.env
  set +a
fi

# https://docs.brew.sh/Installation#post-installation-steps
if [[ -x /opt/homebrew/bin/brew ]]; then
  eval "$(/opt/homebrew/bin/brew shellenv)"
elif [[ -x /usr/local/bin/brew ]]; then
  eval "$(/usr/local/bin/brew shellenv)"
fi

# set PATH so it includes user's private bin if it exists
if [[ -d ~/bin ]]; then export PATH="$HOME/bin:$PATH"; fi

# set PATH so it includes user's local bin if it exists
if [[ -d ~/.local/bin ]]; then export PATH="$HOME/.local/bin:$PATH"; fi

# Toolchains
if [[ -f ~/.cargo/env ]]; then source ~/.cargo/env; fi

# uv
if [[ -f ~/.local/bin/env ]]; then source ~/.local/bin/env; fi

if [[ -d ~/.bun/bin ]]; then
  export BUN_INSTALL="$HOME/.bun"
  export PATH="$BUN_INSTALL/bin:$PATH"
fi

if [[ -d /usr/local/go ]]; then export PATH="$PATH:/usr/local/go/bin"; fi
