# source: /etc/skel/.profile
# ~/.profile: executed by the command interpreter for login shells.
# This file is not read by bash(1), if ~/.bash_profile or ~/.bash_login exists.
# see /usr/share/doc/bash/examples/startup-files for examples.
# the files are located in the bash-doc package.

# the default umask is set in /etc/profile; for setting the umask for ssh
# logins, install and configure the libpam-umask package.
# umask 022

# load env vars from ~/.config/cozydot/.env if exists
if [ -r ~/.config/cozydot/.env ]; then
  set -a
  . ~/.config/cozydot/.env
  set +a
fi

# set PATH so it includes user's private bin if it exists
if [ -d ~/bin ]; then PATH="$HOME/bin:$PATH"; fi

# set PATH so it includes user's private bin if it exists
if [ -d ~/.local/bin ]; then PATH="$HOME/.local/bin:$PATH"; fi

# Git Credential Manager has no default credential store on Linux.
export GCM_CREDENTIAL_STORE=gpg

# Toolchains
if [ -f ~/.cargo/env ]; then . ~/.cargo/env; fi

# uv
if [ -f ~/.local/bin/env ]; then . ~/.local/bin/env; fi

if [ -d ~/.bun/bin ]; then
  export BUN_INSTALL="$HOME/.bun"
  export PATH="$BUN_INSTALL/bin:$PATH"
fi

if [ -d /usr/local/go ]; then export PATH=$PATH:/usr/local/go/bin; fi

# if running bash
if [ -n "$BASH_VERSION" ]; then
  # include .bashrc if it exists
  if [ -f ~/.bashrc ]; then . ~/.bashrc; fi
fi
