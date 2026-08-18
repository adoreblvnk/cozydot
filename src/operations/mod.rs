//! Execute host operations.

mod appimage;
pub(crate) mod appimaged;
pub(crate) mod apt;
pub(crate) mod binary;
pub(crate) mod desktop;
pub(crate) mod docker;
pub(crate) mod fnm;
pub(crate) mod gnome;
pub(crate) mod go;
mod host;
pub(crate) mod macos;
pub(crate) mod packages;
mod parsers;
pub(crate) mod privileged_file;
pub(crate) mod repo;
pub(crate) mod rustup;
mod shell;
pub(crate) mod snapd;
mod systemd;
pub(crate) mod users;
pub(crate) mod uv;
pub(crate) mod vscode;

pub(crate) use host::{
    Host, TempPath, executable_file, path_program, regular_executable_file, require_regular_executable,
};
pub(super) use parsers::{gnome_shell_version, select_gnome_extension_version};
