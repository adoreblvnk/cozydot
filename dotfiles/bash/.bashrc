# ~/.bashrc: executed by bash(1) for non-login shells.
# see /usr/share/doc/bash/examples/startup-files (in the package bash-doc) for examples

# if not running interactively, don't do anything
case $- in *i*) ;; *) return ;; esac

# don't put duplicate lines or lines starting with space in the history.
# see bash(1) for more options
HISTCONTROL=ignoreboth

shopt -s histappend # append to the history file, don't overwrite it

# for setting history length see HISTSIZE and HISTFILESIZE in bash(1)
HISTSIZE=1000
HISTFILESIZE=2000

# check the window size after each command and, if necessary, update the values
# of LINES and COLUMNS.
shopt -s checkwinsize

# If set, the pattern "**" used in a pathname expansion context will match all
# files and zero or more directories and subdirectories.
# shopt -s globstar

# make less more friendly for non-text input files, see lesspipe(1)
[ -x /usr/bin/lesspipe ] && eval "$(SHELL=/bin/sh lesspipe)"

# colored GCC warnings and errors
# export GCC_COLORS='error=01;31:warning=01;35:note=01;36:caret=01;32:locus=01:quote=01'

# Add "alert" alias for long running commands. Use like `sleep 10; alert``
alias alert='notify-send --urgency=low -i "$([ $? = 0 ] && echo terminal || echo error)" "$(history|tail -n1|sed -e '\''s/^\s*[0-9]\+\s*//;s/[;&|]\s*alert$//'\'')"'

# best practice to add alias definitions into separate file, eg ~/.bash_aliases
if [ -f ~/.bash_aliases ]; then . ~/.bash_aliases; fi

# enable programmable completion features (you don't need to enable this, if it's
# already enabled in /etc/bash.bashrc and /etc/profile sources /etc/bash.bashrc).
if ! shopt -oq posix; then
  if [ -f /usr/share/bash-completion/bash_completion ]; then
    . /usr/share/bash-completion/bash_completion
  elif [ -f /etc/bash_completion ]; then . /etc/bash_completion; fi
fi

# force GPG to use pinentry (console) to prompt for passwords instead of a
# window as per `man gpg-agent`
export GPG_TTY=$(tty)

# ----- Load Languages -----
if [ -f ~/.cargo/env ]; then . "$HOME/.cargo/env"; fi

if [ -d /usr/local/go ]; then export PATH=$PATH:/usr/local/go/bin; fi

if [ -d ~/.pyenv ]; then
  # https://github.com/pyenv/pyenv-installer?tab=readme-ov-file#uninstall
  export PATH="$HOME/.pyenv/bin:$PATH"
  eval "$(pyenv init -)"
  eval "$(pyenv virtualenv-init -)"
fi

if [ -s ~/.nvm/nvm.sh ]; then
  export NVM_DIR="$HOME/.nvm"
  . "$NVM_DIR/nvm.sh" && . "$NVM_DIR/bash_completion"
fi

# ----- Apps -----
if [ -f ~/.bash_completion/alacritty ]; then . ~/.bash_completion/alacritty; fi

if command -v starship >/dev/null; then eval "$(starship init bash)"; fi

if command -v yazi &>/dev/null; then
  function yy() { # https://yazi-rs.github.io/docs/quick-start#shell-wrapper
    local tmp="$(mktemp -t "yazi-cwd.XXXXXX")"
    yazi "$@" --cwd-file="$tmp"
    if cwd="$(cat -- "$tmp")" && [ -n "$cwd" ] && [ "$cwd" != "$PWD" ]; then
      builtin cd -- "$cwd"
    fi
    rm -f -- "$tmp"
  }
fi

if command -v zellij &>/dev/null; then
  export ZELLIJ_AUTO_EXIT=true # https://zellij.dev/documentation/integration#bash
  eval "$(zellij setup --generate-auto-start bash)"
  # https://zellij.dev/documentation/controlling-zellij-through-cli#completions
  eval "$(zellij setup --generate-completion bash)"
fi

# https://github.com/ajeetdsouza/zoxide?tab=readme-ov-file#installation
if command -v zoxide &>/dev/null; then eval "$(zoxide init bash)"; fi

# ----- Extras -----
setxkbmap -option caps:swapescape # swap caps lock & escape key

if command -v macchina &>/dev/null; then
  macchina -t Minimal -o host -o kernel -o distribution -o uptime -o processor-load -o memory
fi
