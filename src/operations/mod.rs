//! Execute host operations.

mod appimage;
pub(crate) mod appimaged;
pub(crate) mod apt;
pub(crate) mod binary;
pub(crate) mod desktop;
pub(crate) mod docker;
pub(crate) mod fnm;
mod github;
pub(crate) mod gnome;
pub(crate) mod go;
pub(crate) mod host;
pub(crate) mod macos;
pub(crate) mod packages;
pub(crate) mod privileged_file;
pub(crate) mod repo;
pub(crate) mod rustup;
mod shell;
pub(crate) mod snapd;
mod systemd;
pub(crate) mod users;
pub(crate) mod uv;
pub(crate) mod vscode;
