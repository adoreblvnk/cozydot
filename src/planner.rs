use crate::{
    config::{field, field_string, untag, Config},
    operations::Operation,
    platform::Platform,
    runner::Step,
};
use anyhow::{bail, Result};
use std::path::Path;

const RUN_IF: &str = r#"
kind=$1; shift
case "$kind" in
  command) command -v "$1" >/dev/null || exit 0; shift; exec "$@" ;;
  no-command) ! command -v "$1" >/dev/null || exit 0; shift; exec "$@" ;;
  package-missing) ! dpkg-query -W "$1" >/dev/null 2>&1 || exit 0; shift; exec "$@" ;;
  package-present) dpkg-query -W "$1" >/dev/null 2>&1 || exit 0; shift; exec "$@" ;;
  service-active) systemctl -q is-active "$1" || exit 0; shift; exec "$@" ;;
  user-service-inactive) ! systemctl --user -q is-active "$1" || exit 0; shift; exec "$@" ;;
  group-missing-user) ! getent group "$1" | grep -Fq "$2" || exit 0; shift 2; exec "$@" ;;
  file-absent) [ ! -e "$1" ] || exit 0; shift; exec "$@" ;;
  file-present) [ -e "$1" ] || exit 0; shift; exec "$@" ;;
  *) printf 'unsupported condition: %s\n' "$kind" >&2; exit 2 ;;
esac
"#;

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

fn apt_owned(args: Vec<String>) -> Step {
    sudo(std::iter::once("apt-get".into()).chain(args).collect())
}

trait CheckedOperands {
    fn append_to(self, args: &mut Vec<String>);
}

impl CheckedOperands for &str {
    fn append_to(self, args: &mut Vec<String>) {
        args.push(self.into());
    }
}

impl CheckedOperands for String {
    fn append_to(self, args: &mut Vec<String>) {
        args.push(self);
    }
}

impl<const N: usize> CheckedOperands for [String; N] {
    fn append_to(self, args: &mut Vec<String>) {
        args.extend(self);
    }
}

fn run_if(kind: &str, checked: impl CheckedOperands, command: Step) -> Step {
    let mut args = vec![kind.into()];
    checked.append_to(&mut args);
    args.push(command.program);
    args.extend(command.args);
    Step::bash(RUN_IF, args)
}

fn add_check(cfg: &Config, p: &Platform, root: &Path, out: &mut Vec<Step>) {
    out.push(Step::bash(
        "[ -L \"$2\" ] || cp \"$1\" \"$2\"",
        vec![
            root.join("dotfiles/bash/.bashrc").display().to_string(),
            home_path(".bashrc"),
        ],
    ));
    if cfg.bool("check.distroCfg") {
        match p.distro.as_str() {
            "ubuntu" => {
                out.push(Step::operation(Operation::SnapCleanup));
                out.push(
                    sudo(vec![
                        "tee".into(),
                        "/etc/apt/preferences.d/nosnap.pref".into(),
                    ])
                    .input("Package: snapd\nPin: release a=*\nPin-Priority: -10\n".into()),
                );
                out.push(run_if(
                    "package-missing",
                    "ubuntu-restricted-extras",
                    Step::bash(
                        "sudo apt-get update -qq; sudo apt-get install -qq ubuntu-restricted-extras",
                        vec![],
                    ),
                ));
                out.push(run_if(
                    "command",
                    "unattended-upgrades",
                    apt(&["purge", "-qq", "unattended-upgrades"]),
                ));
                out.push(
                    sudo(vec![
                        "tee".into(),
                        "/etc/apt/apt.conf.d/20auto-upgrades".into(),
                    ])
                    .input(
                        "APT::Periodic::Update-Package-Lists \"0\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n"
                            .into(),
                    ),
                );
            }
            "linuxmint" => out.push(run_if(
                "package-missing",
                "mint-meta-codecs",
                Step::bash(
                    "sudo apt-get update -qq; sudo apt-get install -qq mint-meta-codecs",
                    vec![],
                ),
            )),
            "debian" => {
                let user = user();
                out.push(run_if(
                    "group-missing-user",
                    ["sudo".to_owned(), user.clone()],
                    Step::owned("adduser", vec![user, "sudo".into()]),
                ));
                out.push(
                    sudo(vec!["tee".into(), "/etc/apt/sources.list".into()])
                        .input(debian_sources(&p.codename)),
                );
            }
            _ => {}
        }
    }
    if cfg.tagged_enabled("check.purge") {
        for pkg in cfg.strings("check.purge") {
            out.push(run_if(
                "package-present",
                pkg.clone(),
                apt_owned(vec!["purge".into(), "-qq".into(), pkg]),
            ));
        }
    }
    if cfg.tagged_enabled("check.deps") {
        out.push(apt(&["update", "-qq"]));
        for pkg in cfg.strings("check.deps") {
            out.push(run_if(
                "package-missing",
                pkg.clone(),
                apt_owned(vec!["install".into(), "-qq".into(), pkg]),
            ));
        }
    }
    if cfg.bool("check.rustupCheck") {
        out.push(run_if(
            "no-command",
            "rustup",
            Step::bash(
                "tmp=$(mktemp \"${TMPDIR:-/tmp}/rustup.XXXXXX\"); trap 'rm -f \"$tmp\"' EXIT; curl --proto '=https' --tlsv1.2 -sSf -o \"$tmp\" https://sh.rustup.rs; sh \"$tmp\" -y",
                vec![],
            ),
        ));
    }
    if cfg.bool("check.appimaged") {
        out.push(Step::operation(Operation::Appimaged {
            arch: p.uname_arch.clone(),
        }));
    }
    if cfg.tagged_enabled("check.nerdfont") {
        if let Some(font) = cfg.string("check.nerdfont") {
            out.push(Step::operation(Operation::NerdFont { font }));
        }
    }
}

pub fn plan(command: &str, cfg: &Config, p: &Platform, root: &Path) -> Result<Vec<Step>> {
    let mut out = vec![];
    match command {
        "check" => add_check(cfg, p, root, &mut out),
        "install" => {
            if cfg.bool("install.check") {
                add_check(cfg, p, root, &mut out)
            }
            install(cfg, p, &mut out)
        }
        "update" => {
            if cfg.bool("update.check") {
                add_check(cfg, p, root, &mut out)
            }
            update(cfg, p, &mut out)
        }
        "configure" => {
            if cfg.bool("configure.check") {
                add_check(cfg, p, root, &mut out)
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
        for pkg in cfg.strings("install.apt") {
            out.push(run_if(
                "package-missing",
                pkg.clone(),
                apt_owned(vec!["install".into(), "-qq".into(), pkg]),
            ));
        }
    }
    if cfg.tagged_enabled("install.addRepos") {
        for repo in cfg.sequence("install.addRepos") {
            let name = field_string(repo, "sourceName").expect("validated sourceName");
            let key = p.expand(&field_string(repo, "remoteKey").expect("validated remoteKey"));
            let keypath = field_string(repo, "keyPath").expect("validated keyPath");
            let entry = p.expand_shell_arch(&field_string(repo, "repo").expect("validated repo"));
            if keypath.ends_with(".gpg") {
                out.push(Step::bash(
                    "curl -sSL \"$1\" | sudo gpg --dearmor --yes | sudo tee \"$2\" >/dev/null",
                    vec![key, keypath.clone()],
                ));
            } else {
                out.push(sudo(vec![
                    "curl".into(),
                    "-sSL".into(),
                    "-o".into(),
                    keypath,
                    key,
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
                            .input(pinning_text(pin)),
                    );
                }
            }
        }
        out.push(apt(&["update", "-qq"]));
        for pkg in repo_packages(cfg) {
            out.push(run_if(
                "package-missing",
                pkg.clone(),
                apt_owned(vec!["install".into(), "-qq".into(), pkg]),
            ));
        }
    }
    if cfg.tagged_enabled("install.flatpak") {
        out.push(run_if(
            "command",
            "flatpak",
            Step::new(
                "flatpak",
                &[
                    "remote-add",
                    "--if-not-exists",
                    "flathub",
                    "https://dl.flathub.org/repo/flathub.flatpakrepo",
                ],
            ),
        ));
        let mut a = vec!["install".into(), "-y".into(), "flathub".into()];
        a.extend(cfg.strings("install.flatpak"));
        out.push(run_if("command", "flatpak", Step::owned("flatpak", a)));
    }
    if cfg.tagged_enabled("install.cargo") {
        out.push(Step::bash(
            "export PATH=\"${CARGO_HOME:-$HOME/.cargo}/bin:$PATH\"; command -v cargo >/dev/null || exit 0; command -v cargo-binstall >/dev/null || cargo install cargo-binstall --locked; for pkg in \"$@\"; do read -r -a parts <<<\"$pkg\"; cargo binstall --no-confirm \"${parts[@]}\"; done",
            cfg.strings("install.cargo"),
        ));
    }
    if cfg.tagged_enabled("install.binaries") {
        for b in cfg.sequence("install.binaries") {
            let name = field_string(b, "name").expect("validated name");
            let url_value = field(b, "url").expect("validated url");
            let (url, repo, pattern) = if let Some(url) = untag(url_value).as_str() {
                (p.expand(url), String::new(), String::new())
            } else {
                let repo = field_string(url_value, "repo").expect("validated repo");
                let pattern = p.expand(&field_string(url_value, "asset").expect("validated asset"));
                (String::new(), repo, pattern)
            };
            out.push(Step::operation(Operation::DownloadBinary {
                name,
                url,
                repo,
                pattern,
            }));
        }
    }
    if cfg.tagged_enabled("install.languages.goVersion") {
        out.push(Step::operation(Operation::GoInstall {
            version: cfg
                .string("install.languages.goVersion")
                .expect("validated goVersion"),
            arch: p.go_arch.clone(),
        }));
    }
    if cfg.tagged_enabled("install.languages.nodeVersion") {
        out.push(Step::operation(Operation::NodeInstall {
            version: cfg
                .string("install.languages.nodeVersion")
                .expect("validated nodeVersion"),
            npm: if cfg.tagged_enabled("install.npm") {
                cfg.strings("install.npm")
            } else {
                vec![]
            },
        }));
    }
    if cfg.tagged_enabled("install.npm") && !cfg.tagged_enabled("install.languages.nodeVersion") {
        let mut a = vec!["install".into(), "--global".into()];
        a.extend(cfg.strings("install.npm"));
        out.push(run_if("command", "npm", Step::owned("npm", a)));
    }
    if cfg.tagged_enabled("install.languages.pyenv") {
        out.push(Step::operation(Operation::PyenvInstall {
            update: cfg.bool("install.languages.pyenv.update"),
            version: cfg
                .string("install.languages.pyenv.version")
                .expect("validated pyenv version"),
            pip: cfg.bool("install.languages.pyenv.pip"),
        }));
    }
    if cfg.tagged_enabled("install.languages.uv") {
        out.push(Step::operation(Operation::UvInstall {
            version_enabled: cfg.tagged_enabled("install.languages.uv.version"),
            version: cfg
                .string("install.languages.uv.version")
                .expect("validated uv version"),
        }));
    }
}

fn update(cfg: &Config, p: &Platform, out: &mut Vec<Step>) {
    if cfg.tagged_enabled("update.apt") {
        out.push(apt(&["update", "-qq"]));
        out.push(apt(&["upgrade", "-qq"]));
        if cfg.bool("update.apt.aptFull") {
            out.push(apt(&["dist-upgrade", "-qy"]));
            out.push(apt(&["--purge", "autoremove", "-qy"]));
        }
    }
    if cfg.bool("update.flatpak") {
        out.push(run_if(
            "command",
            "flatpak",
            Step::new("flatpak", &["update", "-y"]),
        ));
    }
    if cfg.bool("update.cargo") {
        out.push(run_if(
            "command",
            "rustup",
            run_if("command", "cargo", Step::new("rustup", &["update"])),
        ));
        out.push(run_if(
            "command",
            "cargo",
            run_if(
                "no-command",
                "cargo-binstall",
                Step::new("cargo", &["install", "cargo-binstall", "--locked"]),
            ),
        ));
        for pkg in cfg.strings("install.cargo") {
            let mut a = vec!["binstall".into(), "--no-confirm".into(), "--force".into()];
            a.extend(pkg.split_whitespace().map(str::to_owned));
            out.push(run_if("command", "cargo", Step::owned("cargo", a)));
        }
    }

    if cfg.bool("update.other.go") {
        out.push(run_if(
            "command",
            "go",
            Step::operation(Operation::GoInstall {
                version: "latest".into(),
                arch: p.go_arch.clone(),
            }),
        ));
    }
    if cfg.bool("update.other.node") {
        out.push(run_if(
            "command",
            "fnm",
            Step::operation(Operation::NodeInstall {
                version: "latest".into(),
                npm: vec![],
            }),
        ));
    }
}

fn configure(cfg: &Config, p: &Platform, root: &Path, out: &mut Vec<Step>) {
    if cfg.tagged_enabled("configure.dotfiles") {
        for pkg in cfg.strings("configure.dotfiles.packages") {
            if cfg.string("configure.dotfiles.stowMode").as_deref() == Some("override") {
                out.push(run_if(
                    "command",
                    "stow",
                    Step::owned(
                        "cp",
                        vec![
                            "-rT".into(),
                            "--remove-destination".into(),
                            root.join("dotfiles").join(&pkg).display().to_string(),
                            home(),
                        ],
                    ),
                ));
            }
            out.push(run_if(
                "command",
                "stow",
                Step::owned(
                    "stow",
                    vec![
                        "--no-folding".into(),
                        "--adopt".into(),
                        "-d".into(),
                        root.join("dotfiles").display().to_string(),
                        "-t".into(),
                        home(),
                        pkg,
                    ],
                ),
            ));
        }
    }
    let user = user();
    if cfg.bool("configure.apps.docker") {
        out.push(Step::operation(Operation::DockerConfig {
            user: user.clone(),
        }));
    }
    if cfg.bool("configure.apps.virtualbox") {
        out.push(Step::operation(Operation::VirtualBoxConfig { user }));
    }
    if cfg.tagged_enabled("configure.apps.vscodeExtensions") {
        for ext in cfg.strings("configure.apps.vscodeExtensions") {
            out.push(Step::operation(Operation::VsCodeExtension {
                extension: ext,
            }));
        }
    }
    if cfg.tagged_enabled("configure.desktopEnvironment")
        && cfg.tagged_enabled("configure.desktopEnvironment.common")
        && cfg.tagged_enabled("configure.desktopEnvironment.common.defaultTerm")
    {
        let term = cfg
            .string("configure.desktopEnvironment.common.defaultTerm")
            .expect("validated terminal");
        if p.desktop == "gnome" {
            out.push(Step::operation(Operation::GnomeTerminal { terminal: term }));
        } else if p.desktop == "cinnamon" {
            out.push(Step::owned(
                "gsettings",
                vec![
                    "set".into(),
                    "org.cinnamon.desktop.default-applications.terminal".into(),
                    "exec".into(),
                    term.clone(),
                ],
            ));
            out.push(Step::new(
                "gsettings",
                &[
                    "set",
                    "org.cinnamon.desktop.default-applications.terminal",
                    "exec-arg",
                    "",
                ],
            ));
        }
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
                    "org.gnome.settings-daemon.plugins.power",
                    "idle-dim",
                    "false",
                ],
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
        out.push(Step::bash(
            "if ! command -v gnome-tweaks >/dev/null || ! command -v gnome-extensions >/dev/null; then sudo apt-get update -qq; sudo apt-get install -qq gnome-tweaks gnome-shell-extensions; fi",
            vec![],
        ));
        if cfg.tagged_enabled("configure.desktopEnvironment.gnome.extensions") {
            for ext in cfg.strings("configure.desktopEnvironment.gnome.extensions") {
                out.push(Step::operation(Operation::GnomeExtension {
                    extension: ext,
                }));
            }
        }
        if cfg.bool("configure.desktopEnvironment.gnome.MacOSDock") {
            out.push(run_if(
                "command",
                "gnome-extensions",
                Step::bash(
                    "gnome-extensions list | grep -Eq 'dash-to-dock|ubuntu-dock' || exit 0; dconf write /org/gnome/shell/extensions/dash-to-dock/dock-position \"'BOTTOM'\"; dconf write /org/gnome/shell/extensions/dash-to-dock/dash-max-icon-size 32; dconf write /org/gnome/shell/extensions/dash-to-dock/dock-fixed false; dconf write /org/gnome/shell/extensions/dash-to-dock/autohide true; dconf write /org/gnome/shell/extensions/dash-to-dock/require-pressure-to-show false; dconf write /org/gnome/shell/extensions/dash-to-dock/intellihide true; dconf write /org/gnome/shell/extensions/dash-to-dock/intellihide-mode \"'FOCUS_APPLICATION_WINDOWS'\"; dconf write /org/gnome/shell/extensions/dash-to-dock/extend-height false; dconf write /org/gnome/shell/extensions/dash-to-dock/click-action \"'minimize-or-previews'\"",
                    vec![],
                ),
            ));
        }
        if cfg.bool("configure.desktopEnvironment.gnome.smoothRoundedCorners") {
            out.push(run_if(
                "command",
                "gnome-extensions",
                Step::bash(
                    "gnome-extensions list | grep -q rounded-window-corners || exit 0; dconf write /org/gnome/shell/extensions/rounded-window-corners-reborn/global-rounded-corner-settings \"{'padding': <{'left': uint32 1, 'right': 1, 'top': 1, 'bottom': 1}>, 'keepRoundedCorners': <{'maximized': false, 'fullscreen': false}>, 'borderRadius': <uint32 16>, 'smoothing': <0.5>, 'borderColor': <(0.5, 0.5, 0.5, 1.0)>, 'enabled': <true>}\"",
                    vec![],
                ),
            ));
        }
    }
}

fn debian_sources(codename: &str) -> String {
    format!(
        "deb http://deb.debian.org/debian {0} main contrib non-free non-free-firmware\n\
deb-src http://deb.debian.org/debian {0} main contrib non-free non-free-firmware\n\n\
deb http://deb.debian.org/debian-security {0}-security main contrib non-free non-free-firmware\n\
deb-src http://deb.debian.org/debian-security {0}-security main contrib non-free non-free-firmware\n\n\
# {0} updates, to get updates before a point release is made;\n\
# see https://www.debian.org/doc/manuals/debian-reference/ch02.en.html#_updates_and_backports\n\
deb http://deb.debian.org/debian {0}-updates main contrib non-free non-free-firmware\n\
deb-src http://deb.debian.org/debian {0}-updates main contrib non-free non-free-firmware\n",
        codename
    )
}

fn repo_packages(cfg: &Config) -> Vec<String> {
    cfg.sequence("install.addRepos")
        .into_iter()
        .flat_map(|r| {
            field(r, "packages")
                .and_then(|v| untag(v).as_sequence())
                .into_iter()
                .flatten()
        })
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn pinning_text(value: &serde_yaml::Value) -> String {
    match untag(value) {
        serde_yaml::Value::String(s) => format!("{s}\n"),
        other => serde_yaml::to_string(other).expect("serialize pinning"),
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "~".into())
}

fn home_path(path: &str) -> String {
    format!("{}/{}", home(), path)
}

fn user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".into())
}
