use crate::{
    config::{field, field_string, untag, Config},
    operations::Operation,
    platform::Platform,
    runner::{Condition, Step},
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

fn apt_owned(args: Vec<String>) -> Step {
    sudo(std::iter::once("apt-get".into()).chain(args).collect())
}

fn run_if(condition: Condition, action: Step) -> Step {
    Step::conditional(condition, action)
}

fn add_check(cfg: &Config, p: &Platform, root: &Path, out: &mut Vec<Step>) {
    out.push(Step::shell(
        "[ -L \"$2\" ] || cp \"$1\" \"$2\"",
        vec![
            root.join("dotfiles/bash/.bashrc").display().to_string(),
            home_path(".bashrc"),
        ],
    ));
    if cfg.bool("check.distroCfg") {
        match p.distro.as_str() {
            "ubuntu" => {
                out.push(Step::workflow(Operation::SnapCleanup));
                out.push(
                    sudo(vec![
                        "tee".into(),
                        "/etc/apt/preferences.d/nosnap.pref".into(),
                    ])
                    .input("Package: snapd\nPin: release a=*\nPin-Priority: -10\n".into()),
                );
                out.push(run_if(
                    Condition::PackageMissing("ubuntu-restricted-extras".into()),
                    Step::workflow(Operation::AptCodecs {
                        package: "ubuntu-restricted-extras".into(),
                    }),
                ));
                out.push(run_if(
                    Condition::CommandExists("unattended-upgrades".into()),
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
                Condition::PackageMissing("mint-meta-codecs".into()),
                Step::workflow(Operation::AptCodecs {
                    package: "mint-meta-codecs".into(),
                }),
            )),
            "debian" => {
                let user = user();
                out.push(run_if(
                    Condition::GroupMissingUser {
                        group: "sudo".into(),
                        user: user.clone(),
                    },
                    sudo(vec!["adduser".into(), user, "sudo".into()]),
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
                Condition::PackageInstalled(pkg.clone()),
                apt_owned(vec!["purge".into(), "-qq".into(), pkg]),
            ));
        }
    }
    if cfg.tagged_enabled("check.deps") {
        out.push(apt(&["update", "-qq"]));
        out.push(Step::workflow(Operation::AptPackages {
            packages: cfg.strings("check.deps"),
        }));
    }
    if cfg.bool("check.rustupCheck") {
        out.push(run_if(
            Condition::CommandMissing("rustup".into()),
            Step::workflow(Operation::RustupBootstrap),
        ));
    }
    if cfg.bool("check.appimaged") {
        out.push(Step::workflow(Operation::Appimaged {
            arch: p.uname_arch.clone(),
        }));
    }
    if cfg.tagged_enabled("check.nerdfont") {
        if let Some(font) = cfg.string("check.nerdfont") {
            out.push(Step::workflow(Operation::NerdFont { font }));
        }
    }
}

pub fn plan(command: &str, cfg: &Config, p: &Platform, root: &Path) -> Result<Vec<Step>> {
    plan_with_check(command, cfg, p, root, true)
}

fn plan_with_check(
    command: &str,
    cfg: &Config,
    p: &Platform,
    root: &Path,
    prepend_check: bool,
) -> Result<Vec<Step>> {
    let mut out = vec![];
    match command {
        "check" => add_check(cfg, p, root, &mut out),
        "install" => {
            if prepend_check && cfg.bool("install.check") {
                add_check(cfg, p, root, &mut out)
            }
            install(cfg, p, &mut out)
        }
        "update" => {
            if prepend_check && cfg.bool("update.check") {
                add_check(cfg, p, root, &mut out)
            }
            update(cfg, p, &mut out)
        }
        "configure" => {
            if prepend_check && cfg.bool("configure.check") {
                add_check(cfg, p, root, &mut out)
            }
            configure(cfg, p, root, &mut out)
        }
        _ => bail!("unknown command {command}"),
    }
    Ok(out)
}

pub fn plan_apply(cfg: &Config, p: &Platform, root: &Path) -> Result<Vec<Step>> {
    let mut out = Vec::new();
    if cfg.bool("install.check") || cfg.bool("update.check") || cfg.bool("configure.check") {
        add_check(cfg, p, root, &mut out);
    }
    out.extend(plan_with_check("install", cfg, p, root, false)?);
    out.extend(plan_with_check("update", cfg, p, root, false)?);
    out.extend(plan_with_check("configure", cfg, p, root, false)?);
    Ok(out)
}

fn install(cfg: &Config, p: &Platform, out: &mut Vec<Step>) {
    if cfg.tagged_enabled("install.apt") {
        out.push(apt(&["update", "-qq"]));
        out.push(Step::workflow(Operation::AptPackages {
            packages: cfg.strings("install.apt"),
        }));
    }
    if cfg.tagged_enabled("install.addRepos") {
        for repo in cfg.sequence("install.addRepos") {
            let name = field_string(repo, "sourceName").expect("validated sourceName");
            let key = p.expand(&field_string(repo, "remoteKey").expect("validated remoteKey"));
            let keypath = field_string(repo, "keyPath").expect("validated keyPath");
            let entry = p.expand_shell_arch(&field_string(repo, "repo").expect("validated repo"));
            if keypath.ends_with(".gpg") {
                out.push(Step::workflow(Operation::RepositoryKey {
                    url: key,
                    destination: keypath.clone(),
                }));
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
        out.push(Step::workflow(Operation::AptPackages {
            packages: repo_packages(cfg),
        }));
    }
    if cfg.tagged_enabled("install.flatpak") {
        out.push(run_if(
            Condition::CommandExists("flatpak".into()),
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
        out.push(run_if(
            Condition::CommandExists("flatpak".into()),
            Step::owned("flatpak", a),
        ));
    }
    if cfg.tagged_enabled("install.cargo") {
        out.push(Step::workflow(Operation::CargoPackages {
            packages: cfg.strings("install.cargo"),
            force: false,
        }));
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
            out.push(Step::workflow(Operation::DownloadBinary {
                name,
                url,
                repo,
                pattern,
            }));
        }
    }
    if cfg.tagged_enabled("install.languages.goVersion") {
        out.push(Step::workflow(Operation::GoInstall {
            version: cfg
                .string("install.languages.goVersion")
                .expect("validated goVersion"),
            arch: p.go_arch.clone(),
        }));
    }
    if cfg.tagged_enabled("install.languages.nodeVersion") {
        out.push(Step::workflow(Operation::NodeInstall {
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
        out.push(run_if(
            Condition::CommandExists("npm".into()),
            Step::owned("npm", a),
        ));
    }
    if cfg.tagged_enabled("install.languages.pyenv") {
        out.push(Step::workflow(Operation::PyenvInstall {
            update: cfg.bool("install.languages.pyenv.update"),
            version: cfg
                .string("install.languages.pyenv.version")
                .expect("validated pyenv version"),
            pip: cfg.bool("install.languages.pyenv.pip"),
        }));
    }
    if cfg.tagged_enabled("install.languages.uv") {
        out.push(Step::workflow(Operation::UvInstall {
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
            Condition::CommandExists("flatpak".into()),
            Step::new("flatpak", &["update", "-y"]),
        ));
    }
    if cfg.bool("update.cargo") {
        out.push(run_if(
            Condition::CommandExists("rustup".into()),
            Step::new("rustup", &["update"]),
        ));
        out.push(Step::workflow(Operation::CargoPackages {
            packages: cfg.strings("install.cargo"),
            force: true,
        }));
    }

    if cfg.bool("update.other.go") {
        out.push(run_if(
            Condition::CommandExists("go".into()),
            Step::workflow(Operation::GoInstall {
                version: "latest".into(),
                arch: p.go_arch.clone(),
            }),
        ));
    }
    if cfg.bool("update.other.node") {
        out.push(run_if(
            Condition::CommandExists("fnm".into()),
            Step::workflow(Operation::NodeInstall {
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
                out.push(Step::owned(
                    "cp",
                    vec![
                        "-rT".into(),
                        "--remove-destination".into(),
                        root.join("dotfiles").join(&pkg).display().to_string(),
                        home(),
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
                    home(),
                    pkg,
                ],
            ));
        }
    }
    let user = user();
    if cfg.bool("configure.apps.docker") {
        out.push(Step::workflow(Operation::DockerConfig {
            user: user.clone(),
        }));
    }
    if cfg.bool("configure.apps.virtualbox") {
        out.push(Step::workflow(Operation::VirtualBoxConfig { user }));
    }
    if cfg.tagged_enabled("configure.apps.vscodeExtensions") {
        for ext in cfg.strings("configure.apps.vscodeExtensions") {
            out.push(Step::workflow(Operation::VsCodeExtension {
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
            out.push(Step::workflow(Operation::GnomeTerminal { terminal: term }));
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
        out.push(Step::workflow(Operation::GnomeDependencies));
        if cfg.tagged_enabled("configure.desktopEnvironment.gnome.extensions") {
            for ext in cfg.strings("configure.desktopEnvironment.gnome.extensions") {
                out.push(Step::workflow(Operation::GnomeExtension { extension: ext }));
            }
        }
        if cfg.bool("configure.desktopEnvironment.gnome.MacOSDock") {
            out.push(run_if(
                Condition::CommandExists("gnome-extensions".into()),
                Step::workflow(Operation::GnomeDockSettings),
            ));
        }
        if cfg.bool("configure.desktopEnvironment.gnome.smoothRoundedCorners") {
            out.push(run_if(
                Condition::CommandExists("gnome-extensions".into()),
                Step::workflow(Operation::GnomeRoundedCornersSettings),
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
