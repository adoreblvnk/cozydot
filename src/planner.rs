use crate::{
    config::{field, field_string, untag, Config},
    platform::Platform,
    runner::Step,
};
use anyhow::{bail, Result};
use std::path::Path;

fn sudo(args: Vec<String>) -> Step {
    Step::owned("sudo", args)
}
fn apt(args: &[&str]) -> Step {
    sudo(
        std::iter::once("apt-get".into())
            .chain(args.iter().map(|s| (*s).into()))
            .collect(),
    )
}
fn shell(script: String) -> Step {
    Step::owned(
        "bash",
        vec!["-o".into(), "pipefail".into(), "-c".into(), script],
    )
}
fn add_check(cfg: &Config, p: &Platform, out: &mut Vec<Step>) {
    if cfg.bool("check.distroCfg") {
        match p.distro.as_str() {
            "ubuntu" => {
                out.push(apt(&["purge", "-qy", "snapd", "unattended-upgrades"]));
                out.push(apt(&["install", "-qy", "ubuntu-restricted-extras"]));
            }
            "linuxmint" => out.push(apt(&["install", "-qy", "mint-meta-codecs"])),
            "debian" => out.push(sudo(vec![
                "adduser".into(),
                std::env::var("USER").unwrap_or_else(|_| "user".into()),
                "sudo".into(),
            ])),
            _ => {}
        }
    }
    if cfg.tagged_enabled("check.purge") {
        let mut a = vec!["purge".into(), "-qy".into()];
        a.extend(cfg.strings("check.purge"));
        out.push(apt_owned(a));
    }
    if cfg.tagged_enabled("check.deps") {
        out.push(apt(&["update", "-qq"]));
        let mut a = vec!["install".into(), "-qy".into()];
        a.extend(cfg.strings("check.deps"));
        out.push(apt_owned(a));
    }
    if cfg.bool("check.rustupCheck") {
        out.push(shell("command -v rustup >/dev/null || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y".into()));
    }
    if cfg.bool("check.appimaged") {
        out.push(shell(format!("mkdir -p \"$HOME/Applications\"; systemctl --user -q is-active appimaged || {{ curl -sSL -o \"$HOME/Applications/appimaged.AppImage\" \"$(curl -sSL https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous | yq -r '.assets[].browser_download_url | select(contains(\"appimaged\") and contains(\"{}\"))')\"; chmod +x \"$HOME/Applications/appimaged.AppImage\"; \"$HOME/Applications/appimaged.AppImage\"; }}",p.uname_arch)));
    }
    if cfg.tagged_enabled("check.nerdfont") {
        if let Some(font) = cfg.string("check.nerdfont") {
            out.push(shell(format!("fc-list :family='{font} NF' | grep -q . || {{ sudo mkdir -p '/usr/share/fonts/{font}'; curl -sSL 'https://github.com/ryanoasis/nerd-fonts/releases/latest/download/{font}.tar.xz' | sudo tar -xJ -C '/usr/share/fonts/{font}'; fc-cache -f; }}")));
        }
    }
}
fn apt_owned(args: Vec<String>) -> Step {
    sudo(std::iter::once("apt-get".into()).chain(args).collect())
}

pub fn plan(command: &str, cfg: &Config, p: &Platform, root: &Path) -> Result<Vec<Step>> {
    let mut out = vec![];
    match command {
        "check" => add_check(cfg, p, &mut out),
        "install" => {
            if cfg.bool("install.check") {
                add_check(cfg, p, &mut out)
            }
            install(cfg, p, &mut out)
        }
        "update" => {
            if cfg.bool("update.check") {
                add_check(cfg, p, &mut out)
            }
            update(cfg, p, &mut out)
        }
        "configure" => {
            if cfg.bool("configure.check") {
                add_check(cfg, p, &mut out)
            }
            configure(cfg, p, root, &mut out)
        }
        _ => bail!("unknown command {command}"),
    }
    Ok(out)
}
fn install(cfg: &Config, p: &Platform, out: &mut Vec<Step>) {
    if cfg.tagged_enabled("install.apt") {
        out.push(apt(&["update", "-qq"]));
        let mut a = vec!["install".into(), "-qy".into()];
        a.extend(cfg.strings("install.apt"));
        out.push(apt_owned(a));
    }
    if cfg.tagged_enabled("install.addRepos") {
        for repo in cfg.sequence("install.addRepos") {
            let name = field_string(repo, "sourceName").unwrap();
            let key = field_string(repo, "remoteKey").unwrap_or_default();
            let keypath = field_string(repo, "keyPath").unwrap_or_default();
            let entry = p.expand(&field_string(repo, "repo").unwrap());
            if keypath.ends_with(".gpg") {
                out.push(shell(format!(
                    "curl -sSL '{}' | sudo gpg --dearmor --yes -o '{}'",
                    p.expand(&key),
                    keypath
                )));
            } else {
                out.push(sudo(vec![
                    "curl".into(),
                    "-sSL".into(),
                    "-o".into(),
                    keypath.clone(),
                    p.expand(&key),
                ]));
            }
            out.push(
                sudo(vec![
                    "tee".into(),
                    format!("/etc/apt/sources.list.d/{name}.list"),
                ])
                .input(entry),
            );
            if let Some(pin) = field(repo, "pinning") {
                if !matches!(untag(pin), serde_yaml::Value::Bool(false)) {
                    out.push(
                        sudo(vec!["tee".into(), format!("/etc/apt/preferences.d/{name}")])
                            .input(serde_yaml::to_string(untag(pin)).unwrap()),
                    );
                }
            }
        }
        out.push(apt(&["update", "-qq"]));
        let pkgs: Vec<String> = cfg
            .sequence("install.addRepos")
            .into_iter()
            .flat_map(|r| {
                field(r, "packages")
                    .and_then(|v| untag(v).as_sequence())
                    .into_iter()
                    .flatten()
            })
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        out.push(apt_owned(
            std::iter::once("install".into())
                .chain(std::iter::once("-qy".into()))
                .chain(pkgs)
                .collect(),
        ));
    }
    if cfg.tagged_enabled("install.flatpak") {
        out.push(Step::new(
            "flatpak",
            &[
                "remote-add",
                "--if-not-exists",
                "flathub",
                "https://dl.flathub.org/repo/flathub.flatpakrepo",
            ],
        ));
        let mut a = vec!["install".into(), "-y".into(), "flathub".into()];
        a.extend(cfg.strings("install.flatpak"));
        out.push(Step::owned("flatpak", a));
    }
    if cfg.tagged_enabled("install.cargo") {
        out.push(shell(
            "command -v cargo-binstall >/dev/null || cargo install cargo-binstall --locked".into(),
        ));
        for pkg in cfg.strings("install.cargo") {
            let mut a = vec!["binstall".into(), "--no-confirm".into()];
            a.extend(pkg.split_whitespace().map(str::to_owned));
            out.push(Step::owned("cargo", a));
        }
    }
    if cfg.tagged_enabled("install.binaries") {
        for b in cfg.sequence("install.binaries") {
            let name = field_string(b, "name").unwrap();
            let url = p.expand(&field_string(b, "url").unwrap());
            let dest = format!("$HOME/Applications/{name}");
            out.push(shell(format!(
                "mkdir -p \"$HOME/Applications\"; curl -sSL -o \"{dest}\" \"{url}\""
            )));
            if name.ends_with(".deb") {
                out.push(shell(format!(
                    "sudo apt-get install -qy \"{dest}\" && rm -f \"{dest}\""
                )))
            } else {
                out.push(shell(format!("chmod +x \"{dest}\"")))
            }
        }
    }
    if cfg.tagged_enabled("install.languages.goVersion") {
        let v = cfg.string("install.languages.goVersion").unwrap();
        out.push(shell(format!("v='{v}'; [ \"$v\" != latest ] || v=$(curl -sSL 'https://go.dev/dl/?mode=json' | yq -r '.[0].version' | cut -c3-); curl -sSL -o /tmp/go.tar.gz \"https://go.dev/dl/go${{v}}.linux-{}.tar.gz\"; sudo rm -rf /usr/local/go; sudo tar -C /usr/local -xzf /tmp/go.tar.gz",p.go_arch)));
    }
    if cfg.tagged_enabled("install.languages.nodeVersion") {
        let v = cfg.string("install.languages.nodeVersion").unwrap();
        out.push(shell(format!("command -v fnm >/dev/null || curl -fsSL https://fnm.vercel.app/install | bash -s -- --skip-shell; eval \"$(fnm env --shell bash)\"; fnm install {} --use; fnm default \"$(fnm current)\"",if v=="latest"{"--lts"}else{&v})));
    }
    if cfg.tagged_enabled("install.npm") {
        let mut a = vec!["install".into(), "--global".into()];
        a.extend(cfg.strings("install.npm"));
        out.push(Step::owned("npm", a));
    }
    if cfg.tagged_enabled("install.languages.pyenv") {
        out.push(shell(
            "command -v pyenv >/dev/null || curl https://pyenv.run | bash".into(),
        ));
        let v = cfg.string("install.languages.pyenv.version").unwrap();
        out.push(shell(format!(
            "v=$(pyenv latest -k '{v}'); pyenv install -s \"$v\"; pyenv global \"$v\""
        )));
    }
    if cfg.tagged_enabled("install.languages.uv") {
        out.push(shell(
            "command -v uv >/dev/null || curl -LsSf https://astral.sh/uv/install.sh | sh".into(),
        ));
        if cfg.tagged_enabled("install.languages.uv.version") {
            out.push(Step::owned(
                "uv",
                vec![
                    "python".into(),
                    "install".into(),
                    cfg.string("install.languages.uv.version").unwrap(),
                ],
            ))
        }
    }
}

fn update(cfg: &Config, p: &Platform, out: &mut Vec<Step>) {
    if cfg.tagged_enabled("update.apt") {
        out.push(apt(&["update", "-qq"]));
        out.push(apt(&["upgrade", "-qy"]));
        if cfg.bool("update.apt.aptFull") {
            out.push(apt(&["dist-upgrade", "-qy"]));
            out.push(apt(&["autoremove", "-qy", "--purge"]));
        }
    }
    if cfg.bool("update.flatpak") {
        out.push(Step::new("flatpak", &["update", "-y"]));
    }
    if cfg.bool("update.cargo") {
        out.push(Step::new("rustup", &["update"]));
        out.push(shell(
            "command -v cargo-binstall >/dev/null || cargo install cargo-binstall --locked".into(),
        ));
        for pkg in cfg.strings("install.cargo") {
            let mut a = vec!["binstall".into(), "--no-confirm".into(), "--force".into()];
            a.extend(pkg.split_whitespace().map(str::to_owned));
            out.push(Step::owned("cargo", a));
        }
    }
    if cfg.bool("update.other.yq") {
        out.push(sudo(vec![
            "curl".into(),
            "-sSL".into(),
            "-o".into(),
            "/usr/bin/yq".into(),
            format!(
                "https://github.com/mikefarah/yq/releases/latest/download/yq_linux_{}",
                p.go_arch
            ),
        ]));
        out.push(sudo(vec![
            "chmod".into(),
            "+x".into(),
            "/usr/bin/yq".into(),
        ]));
    }
    if cfg.bool("update.other.go") {
        out.push(shell(format!("v=$(curl -sSL 'https://go.dev/dl/?mode=json' | yq -r '.[0].version' | cut -c3-); curl -sSL -o /tmp/go.tar.gz \"https://go.dev/dl/go${{v}}.linux-{}.tar.gz\"; sudo rm -rf /usr/local/go; sudo tar -C /usr/local -xzf /tmp/go.tar.gz",p.go_arch)));
    }
    if cfg.bool("update.other.node") {
        out.push(shell("eval \"$(fnm env --shell bash)\"; fnm install --lts --use; fnm default \"$(fnm current)\"".into()));
    }
}

fn configure(cfg: &Config, p: &Platform, root: &Path, out: &mut Vec<Step>) {
    if cfg.tagged_enabled("configure.dotfiles") {
        for pkg in cfg.strings("configure.dotfiles.packages") {
            if cfg.string("configure.dotfiles.stowMode").as_deref() == Some("override") {
                out.push(Step::owned(
                    "cp",
                    vec![
                        "-rT".into(),
                        "--remove-destination".into(),
                        root.join("dotfiles").join(&pkg).display().to_string(),
                        std::env::var("HOME").unwrap_or_else(|_| "~".into()),
                    ],
                ));
            }
            out.push(Step::owned(
                "stow",
                vec![
                    "--no-folding".into(),
                    "--adopt".into(),
                    "-d".into(),
                    root.join("dotfiles").display().to_string(),
                    "-t".into(),
                    std::env::var("HOME").unwrap_or_else(|_| "~".into()),
                    pkg,
                ],
            ));
        }
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    if cfg.bool("configure.apps.docker") {
        out.push(sudo(vec!["groupadd".into(), "-f".into(), "docker".into()]));
        out.push(sudo(vec![
            "usermod".into(),
            "-aG".into(),
            "docker".into(),
            user.clone(),
        ]));
        out.push(
            sudo(vec!["tee".into(), "/etc/docker/daemon.json".into()])
                .input("{\"log-driver\":\"local\",\"log-opts\":{\"max-size\":\"10m\"}}\n".into()),
        );
    }
    if cfg.bool("configure.apps.virtualbox") {
        out.push(sudo(vec![
            "groupadd".into(),
            "-f".into(),
            "vboxusers".into(),
        ]));
        out.push(sudo(vec![
            "usermod".into(),
            "-aG".into(),
            "vboxusers".into(),
            user,
        ]));
    }
    if cfg.tagged_enabled("configure.apps.vscodeExtensions") {
        for ext in cfg.strings("configure.apps.vscodeExtensions") {
            out.push(Step::owned("code", vec!["--install-extension".into(), ext]));
        }
    }
    if cfg.tagged_enabled("configure.desktopEnvironment")
        && cfg.tagged_enabled("configure.desktopEnvironment.common.defaultTerm")
    {
        let term = cfg
            .string("configure.desktopEnvironment.common.defaultTerm")
            .unwrap();
        let schema = if p.desktop == "cinnamon" {
            "org.cinnamon.desktop.default-applications.terminal"
        } else {
            "org.gnome.desktop.default-applications.terminal"
        };
        out.push(Step::owned(
            "gsettings",
            vec!["set".into(), schema.into(), "exec".into(), term],
        ));
    }
    if p.desktop == "gnome" && cfg.tagged_enabled("configure.desktopEnvironment.gnome") {
        if cfg.bool("configure.desktopEnvironment.gnome.settings") {
            out.push(Step::new(
                "gsettings",
                &["set", "org.gnome.desktop.session", "idle-delay", "900"],
            ));
            out.push(Step::new(
                "gsettings",
                &[
                    "set",
                    "org.gnome.desktop.interface",
                    "color-scheme",
                    "prefer-dark",
                ],
            ));
        }
        for ext in cfg.strings("configure.desktopEnvironment.gnome.extensions") {
            out.push(shell(format!("gnome-extensions list | grep -Fq '{ext}' && gnome-extensions enable '{ext}' || true")));
        }
        if cfg.bool("configure.desktopEnvironment.gnome.MacOSDock") {
            for (k, v) in [
                ("dock-position", "'BOTTOM'"),
                ("dash-max-icon-size", "32"),
                ("dock-fixed", "false"),
                ("autohide", "true"),
                ("extend-height", "false"),
            ] {
                out.push(Step::owned(
                    "dconf",
                    vec![
                        "write".into(),
                        format!("/org/gnome/shell/extensions/dash-to-dock/{k}"),
                        v.into(),
                    ],
                ));
            }
        }
    }
}
