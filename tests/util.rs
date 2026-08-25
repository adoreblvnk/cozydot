//! Sandbox shared by the integration tests, modeled on fd's testenv.

use std::{env, fs, os::unix::fs::PermissionsExt, path::PathBuf};

use assert_cmd::Command;

pub const MINIMAL_CONFIG: &str = "\
version: 1
system:
  macos:
    xcode: {}
packages:
  linux: {}
  macos:
    homebrew:
      formulae: []
      casks: []
tools: {}
fonts: {}
dotfiles:
  packages:
    all: []
    linux: []
    macos: []
integrations:
  vscode:
    extensions: []
  linux: {}
updates:
  packages:
    linux: {}
    macos:
      homebrew: {}
  tools: {}
  fonts: false
";

pub struct TestEnv {
    temp: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TestEnv {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let bin = temp.path().join("bin");
        fs::create_dir_all(&home).unwrap();
        Self { temp, home, bin }
    }

    pub fn root(&self) -> &std::path::Path {
        self.temp.path()
    }

    pub fn home(&self) -> &std::path::Path {
        &self.home
    }

    pub fn state_home(&self) -> PathBuf {
        self.temp.path().join("state")
    }

    /// Cozydot command isolated from the host: HOME & XDG dirs inside the sandbox
    pub fn cozydot(&self) -> Command {
        let mut command = Command::cargo_bin("cozydot").unwrap();
        command
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.temp.path())
            .env("XDG_STATE_HOME", self.state_home())
            .env("XDG_CURRENT_DESKTOP", "");
        if self.bin.exists() {
            command.env("PATH", self.mocked_path());
        }
        command
    }

    /// PATH with the sandbox mocks ahead of the host executables
    pub fn mocked_path(&self) -> std::ffi::OsString {
        let mut path = std::ffi::OsString::from(&self.bin);
        path.push(":");
        path.push(env::var_os("PATH").unwrap_or_default());
        path
    }

    pub fn mock(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }

    pub fn write_config(&self, config: &str) {
        fs::create_dir_all(self.temp.path().join("cozydot")).unwrap();
        fs::write(self.temp.path().join("cozydot/cozydot.yaml"), config).unwrap();
    }
}
