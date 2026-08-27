# source: /etc/skel/.bashrc
# ~/.bashrc: executed by bash(1) for non-login shells.
# see /usr/share/doc/bash/examples/startup-files (in the package bash-doc) for examples

case $- in *i*) ;; *) return ;; esac # if not running interactively, don't do anything

# don't put duplicate lines or lines starting with space in the history. see bash(1) for more options
HISTCONTROL=ignoreboth

shopt -s histappend # append to the history file, don't overwrite it

HISTSIZE=1000 HISTFILESIZE=2000 # for setting history length see HISTSIZE and HISTFILESIZE in bash(1)

# check the window size after each command and, if necessary, update the values of LINES and COLUMNS.
shopt -s checkwinsize

# If set, the pattern "**" used in a pathname expansion context will match all files and zero or more directories and subdirectories.
# shopt -s globstar

# make less more friendly for non-text input files, see lesspipe(1)
[ -x /usr/bin/lesspipe ] && eval "$(SHELL=/bin/sh lesspipe)"

# colored GCC warnings and errors
# export GCC_COLORS='error=01;31:warning=01;35:note=01;36:caret=01;32:locus=01:quote=01'

# Add "alert" alias for long running commands. Use like `sleep 10; alert`
alias alert='notify-send --urgency=low -i "$([ $? = 0 ] && echo terminal || echo error)" "$(history|tail -n1|sed -e '\''s/^\s*[0-9]\+\s*//;s/[;&|]\s*alert$//'\'')"'

# Alias definitions.
# You may want to put all your additions into a separate file like
# ~/.bash_aliases, instead of adding them here directly.
# See /usr/share/doc/bash-doc/examples in the bash-doc package.
if [ -f ~/.bash_aliases ]; then source ~/.bash_aliases; fi

# enable programmable completion features (you don't need to enable this, if it's
# already enabled in /etc/bash.bashrc and /etc/profile sources /etc/bash.bashrc).
if ! shopt -oq posix; then
  if [ -f /usr/share/bash-completion/bash_completion ]; then
    source /usr/share/bash-completion/bash_completion
  elif [ -f /etc/bash_completion ]; then
    source /etc/bash_completion
  fi
fi

# force GPG to use pinentry (console) to prompt for passwords instead of a window as per `man gpg-agent`
export GPG_TTY=$(tty)

# WSL: add Win user folder as env var
if [[ -n $WSL_DISTRO_NAME ]]; then export WIN="/mnt/c/Users/$USER"; fi

# Toolchains
FNM_PATH=~/.local/share/fnm
if [ -d "$FNM_PATH" ]; then
  export PATH="$FNM_PATH:$PATH"
  eval "$(fnm env --use-on-cd --shell bash)"
fi

# uv
if command -v uv &>/dev/null; then eval "$(uv generate-shell-completion bash)"; fi
if command -v uvx &>/dev/null; then eval "$(uvx --generate-shell-completion bash)"; fi

# Aliases
alias c=clear
alias pip="python -m pip"

if command -v bat &>/dev/null; then alias cat="bat -pp"; fi
if ! command -v trash &>/dev/null && command -v gio &>/dev/null; then alias trash="gio trash"; fi

# eza aliases
if command -v eza &>/dev/null; then
  alias ls="eza --group-directories-first --icons=auto"
  alias la="eza --group-directories-first --icons=auto -a"
  alias ll="eza --group-directories-first --icons=auto --git -al"
  alias tree="eza --group-directories-first --icons=auto -T"
fi

# Integrations
# tells wezterm the current cwd (for tabs) & command status
# uses OSC 7/133 sequences supported by most terminals & fails silently if wezterm is missing
# source: https://github.com/wezterm/wezterm/blob/main/assets/shell-integration/wezterm.sh
if [[ -f ~/.config/wezterm.sh ]]; then source ~/.config/wezterm.sh; fi

if command -v bat &>/dev/null; then export MANPAGER="bat -plman"; fi

# Set up fzf key bindings and fuzzy completion
if command -v fzf &>/dev/null; then eval "$(fzf --bash)"; fi

if command -v yazi &>/dev/null; then
  # https://yazi-rs.github.io/docs/quick-start#shell-wrapper
  function y() {
  	local tmp cwd; tmp="$(mktemp -t "yazi-cwd.XXXXXX")"
  	command yazi "$@" --cwd-file="$tmp"
  	IFS= read -r -d '' cwd < "$tmp"
  	[ "$cwd" != "$PWD" ] && [ -d "$cwd" ] && builtin cd -- "$cwd" || builtin true
  	command rm -f -- "$tmp"
  }
fi

# https://github.com/ajeetdsouza/zoxide?tab=readme-ov-file#installation
if command -v zoxide &>/dev/null; then eval "$(zoxide init bash)"; fi

# Prompt
if command -v starship >/dev/null; then eval "$(starship init bash)"; fi

# Startup
# display system info
if command -v fastfetch &>/dev/null; then fastfetch; fi
