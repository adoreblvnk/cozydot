use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn run(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn asset_name() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => panic!("unsupported test architecture: {other}"),
    };
    format!("cozydot-0.0.1-linux-{arch}.tar.gz")
}

fn mirror_archive(root: &Path) -> PathBuf {
    let mirror = root.join("mirror/download/v0.0.1");
    fs::create_dir_all(&mirror).unwrap();
    mirror.join(asset_name())
}

fn write_checksum(archive: &Path) {
    let checksum = run(Command::new("sha256sum").arg(archive));
    fs::write(archive.with_extension("gz.sha256"), checksum.stdout).unwrap();
}

fn installer(root: &Path) -> Command {
    let mut command = Command::new("bash");
    command
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh"))
        .env("HOME", root.join("home"))
        .env("XDG_BIN_HOME", root.join("bin"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("TMPDIR", root.join("tmp"))
        .env(
            "COZYDOT_RELEASE_BASE_URL",
            format!("file://{}", root.join("mirror").display()),
        );
    command
}

fn assert_install_rejected(root: &Path) {
    fs::create_dir(root.join("tmp")).unwrap();
    let output = installer(root).output().unwrap();
    assert!(
        !output.status.success(),
        "malformed archive was accepted\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.join("bin/cozydot").exists(), "binary was published");
    assert!(!root.join("config").exists(), "config was published");
    assert!(!root.join("cache").exists(), "cache was published");
    assert_eq!(fs::read_dir(root.join("tmp")).unwrap().count(), 0);
}

#[test]
fn installer_only_installs_binary_and_installed_init_is_offline() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let stage = root.join("stage");
    let archive = mirror_archive(root);
    let config = root.join("config");
    let cache = root.join("cache");
    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&stage).unwrap();
    fs::create_dir(root.join("tmp")).unwrap();
    fs::create_dir(&elsewhere).unwrap();
    fs::copy(
        assert_cmd::cargo::cargo_bin!("cozydot"),
        stage.join("cozydot"),
    )
    .unwrap();
    run(Command::new("tar").args([
        "--sort=name",
        "--mtime=@0",
        "--owner=0",
        "--group=0",
        "--numeric-owner",
        "-C",
        stage.to_str().unwrap(),
        "-czf",
        archive.to_str().unwrap(),
        "cozydot",
    ]));
    write_checksum(&archive);

    run(&mut installer(root));
    assert!(!config.exists(), "installer provisioned config state");
    assert!(!cache.exists(), "installer created a cache");
    assert_eq!(fs::read_dir(root.join("tmp")).unwrap().count(), 0);

    fs::remove_dir_all(root.join("mirror")).unwrap();
    run(Command::new(root.join("bin/cozydot"))
        .arg("init")
        .current_dir(&elsewhere)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_CACHE_HOME", &cache)
        .env("COZYDOT_RELEASE_BASE_URL", "file:///does/not/exist"));

    let installed = config.join("cozydot");
    assert!(installed.join("cozydot.yaml").is_file());
    assert!(installed.join("dotfiles/bash/.bashrc").is_file());
    assert_eq!(
        fs::metadata(installed.join("cozydot.yaml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert_ne!(
        fs::metadata(installed.join("dotfiles/bin/.local/bin/round"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert!(!cache.exists(), "init created or consumed a cache");
}

#[test]
fn installer_rejects_malformed_archives_without_publication() {
    for case in [
        "extra",
        "duplicate",
        "symlink",
        "special",
        "traversal",
        "absolute",
        "checksum",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let stage = root.join("stage");
        let archive = mirror_archive(root);
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("payload"), b"binary").unwrap();

        let mut tar = Command::new("tar");
        tar.arg("-C").arg(&stage).arg("-czf").arg(&archive);
        match case {
            "extra" => {
                fs::copy(stage.join("payload"), stage.join("cozydot")).unwrap();
                fs::write(stage.join("extra"), b"extra").unwrap();
                tar.args(["cozydot", "extra"]);
            }
            "duplicate" => {
                fs::copy(stage.join("payload"), stage.join("cozydot")).unwrap();
                tar.args(["cozydot", "cozydot"]);
            }
            "symlink" => {
                symlink("payload", stage.join("cozydot")).unwrap();
                tar.arg("cozydot");
            }
            "special" => {
                run(Command::new("mkfifo").arg(stage.join("cozydot")));
                tar.arg("cozydot");
            }
            "traversal" => {
                tar.arg("--transform=s|^payload$|../cozydot|")
                    .arg("payload");
            }
            "absolute" => {
                tar.arg("--transform=s|^payload$|/cozydot|").arg("payload");
            }
            "checksum" => {
                fs::copy(stage.join("payload"), stage.join("cozydot")).unwrap();
                tar.arg("cozydot");
            }
            _ => unreachable!(),
        }
        run(&mut tar);
        write_checksum(&archive);
        if case == "checksum" {
            fs::write(
                archive.with_extension("gz.sha256"),
                format!("{}  {}\n", "0".repeat(64), asset_name()),
            )
            .unwrap();
        }
        assert_install_rejected(root);
    }
}
