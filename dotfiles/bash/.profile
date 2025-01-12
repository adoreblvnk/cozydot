# ~/.profile: executed by the command interpreter for login shells.
# This file is not read by bash(1), if ~/.bash_profile or ~/.bash_login exists.
# see /usr/share/doc/bash/examples/startup-files for examples.
# the files are located in the bash-doc package.

# the default umask is set in /etc/profile; for setting the umask for ssh
# logins, install and configure the libpam-umask package.
# umask 022

# if running bash
if [ -n "$BASH_VERSION" ]; then
  # include .bashrc if it exists
  if [ -f ~/.bashrc ]; then source ~/.bashrc; fi
fi

# set PATH so it includes user's private bin if it exists
if [ -d ~/bin ]; then PATH="$HOME/bin:$PATH"; fi

# set PATH so it includes user's private bin if it exists
if [ -d ~/.local/bin ]; then PATH="$HOME/.local/bin:$PATH"; fi

if [ -d ~/.pyenv ]; then
  # https://github.com/pyenv/pyenv-installer?tab=readme-ov-file#uninstall
  export PATH="$HOME/.pyenv/bin:$PATH"
  eval "$(pyenv init -)"
  eval "$(pyenv virtualenv-init -)"
fi

if [ -f ~/.cargo/env ]; then source ~/.cargo/env; fi
