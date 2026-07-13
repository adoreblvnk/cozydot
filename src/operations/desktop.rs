use super::{Host, TempPath};
use crate::json_helpers;
use anyhow::Result;

pub fn gnome_extension(host: &Host<'_>, extension: &str) -> Result<()> {
    if !host.command_exists("gnome-extensions") {
        return Ok(());
    }
    let output = host.require("gnome extension", "gnome-extensions", ["list"])?;
    if String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|installed| installed == extension)
    {
        host.require("gnome extension", "gnome-extensions", ["enable", extension])?;
        return Ok(());
    }
    let endpoint = format!("https://extensions.gnome.org/extension-info/?uuid={extension}");
    let metadata = host.require("gnome extension", "curl", ["-fsSL", &endpoint])?;
    let shell = host.require("gnome extension", "gnome-shell", ["--version"])?;
    let shell_version = json_helpers::gnome_shell_version(&String::from_utf8(shell.stdout)?)?;
    let version =
        json_helpers::gnome_version(&String::from_utf8(metadata.stdout)?, &shell_version)?;
    let archive = TempPath::new(host, "gnome-extension.zip")?;
    let name = extension.replace('@', "");
    let url = format!(
        "https://extensions.gnome.org/extension-data/{name}.v{version}.shell-extension.zip"
    );
    host.require(
        "gnome extension",
        "curl",
        ["-fL", "-o", &archive.path().to_string_lossy(), &url],
    )?;
    host.require(
        "gnome extension",
        "gnome-extensions",
        ["install", "--force", &archive.path().to_string_lossy()],
    )?;
    host.require("gnome extension", "gnome-extensions", ["enable", extension])?;
    Ok(())
}
