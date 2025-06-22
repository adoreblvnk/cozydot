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

# Add "alert" alias for long running commands. Use like `sleep 10; alert``
alias alert='notify-send --urgency=low -i "$([ $? = 0 ] && echo terminal || echo error)" "$(history|tail -n1|sed -e '\''s/^\s*[0-9]\+\s*//;s/[;&|]\s*alert$//'\'')"'

# best practice to add alias definitions into separate file, eg ~/.bash_aliases
if [ -f ~/.bash_aliases ]; then source ~/.bash_aliases; fi

# bash aliases if you don't use a bash_aliases file
alias c=clear
alias py=python
alias pip="python -m pip"

if command -v bat &>/dev/null; then alias cat="bat -pp"; fi

# eza aliases
if command -v eza &>/dev/null; then
  alias ls="eza --group-directories-first --icons=auto"
  alias la="eza --group-directories-first --icons=auto -a"
  alias ll="eza --group-directories-first --icons=auto -al"
  alias tree="eza --group-directories-first --icons=auto -T"
fi

if command -v cozydot &>/dev/null; then alias czy=cozydot; fi

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

# ----- Load Languages -----
if command -v uv &>/dev/null; then eval "$(uv generate-shell-completion bash)"; fi
if command -v uvx &>/dev/null; then eval "$(uvx --generate-shell-completion bash)"; fi

if [ -f ~/.cargo/env ]; then source ~/.cargo/env; fi

if [ -d /usr/local/go ]; then export PATH=$PATH:/usr/local/go/bin; fi

if [ -s ~/.nvm/nvm.sh ]; then
  export NVM_DIR="$HOME/.nvm"
  source "$NVM_DIR/nvm.sh"
  source "$NVM_DIR/bash_completion"
fi

# ----- Apps -----
if [ -f ~/.bash_completion/alacritty ]; then source ~/.bash_completion/alacritty; fi

if command -v starship >/dev/null; then eval "$(starship init bash)"; fi

if command -v yazi &>/dev/null; then
  # https://yazi-rs.github.io/docs/quick-start#shell-wrapper
  function y() {
    local tmp="$(mktemp -t "yazi-cwd.XXXXXX")" cwd
    yazi "$@" --cwd-file="$tmp"
    IFS= read -r -d '' cwd <"$tmp"
    [ -n "$cwd" ] && [ "$cwd" != "$PWD" ] && builtin cd -- "$cwd"
    rm -f -- "$tmp"
  }
fi

if command -v zellij &>/dev/null; then
  eval "$(zellij setup --generate-auto-start bash)"
  # https://zellij.dev/documentation/controlling-zellij-through-cli#completions
  eval "$(zellij setup --generate-completion bash)"
fi

# https://github.com/ajeetdsouza/zoxide?tab=readme-ov-file#installation
if command -v zoxide &>/dev/null; then eval "$(zoxide init bash)"; fi

# display system info
if command -v macchina &>/dev/null; then
  macchina -o host -o kernel -o distribution -o uptime -o processor-load -o memory
fi
