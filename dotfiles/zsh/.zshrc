# ~/.zshrc: executed by zsh for interactive shells.
# macOS loads /etc/zshrc before this file.

# do not save consecutive duplicates or commands prefixed with a space
setopt HIST_IGNORE_DUPS HIST_IGNORE_SPACE

# https://zsh.sourceforge.io/Doc/Release/Parameters.html
WORDCHARS=${WORDCHARS//\//} # treat / as word separator
# https://zsh.sourceforge.io/Doc/Release/Options.html
unsetopt LIST_AMBIGUOUS # show all matches when ambiguous instead of completing the common prefix
setopt INTERACTIVE_COMMENTS # treat # as start of comment
# https://zsh.sourceforge.io/Doc/Release/Zsh-Line-Editor.html
bindkey '^[[A' history-beginning-search-backward
bindkey '^[[B' history-beginning-search-forward
# https://zsh.sourceforge.io/Doc/Release/Completion-System.html
# enable completion system
autoload -U compinit
compinit
_comp_options+=(globdots) # include hidden files
zstyle ':completion:*' matcher-list 'm:{a-zA-Z}={A-Za-z}' # case insensitive completion
zstyle ':completion:*' list-colors ''

# Toolchains
FNM_PATH=~/.local/share/fnm
if [[ -d "$FNM_PATH" ]]; then
  export PATH="$FNM_PATH:$PATH"
  eval "$(fnm env --use-on-cd --shell zsh)"
fi

# uv
if command -v uv &>/dev/null; then eval "$(uv generate-shell-completion zsh)"; fi
if command -v uvx &>/dev/null; then eval "$(uvx --generate-shell-completion zsh)"; fi

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
if command -v fzf &>/dev/null; then eval "$(fzf --zsh)"; fi

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
if command -v zoxide &>/dev/null; then eval "$(zoxide init zsh)"; fi

# Prompt
if command -v starship &>/dev/null; then eval "$(starship init zsh)"; fi

# Startup
# display system info
if command -v fastfetch &>/dev/null; then fastfetch; fi

# Plugins
if command -v brew &>/dev/null; then
  # must be sourced after completion & other plugins
  if [[ -f "$HOMEBREW_PREFIX/share/zsh-autosuggestions/zsh-autosuggestions.zsh" ]]; then
    source "$HOMEBREW_PREFIX/share/zsh-autosuggestions/zsh-autosuggestions.zsh"
  fi
  if [[ -f "$HOMEBREW_PREFIX/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh" ]]; then
    source "$HOMEBREW_PREFIX/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh"
  fi
fi
