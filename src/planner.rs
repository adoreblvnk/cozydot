use crate::{
    config::{field, field_string, untag, Config},
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

const SNAP_CLEANUP: &str = r#"
if command -v snap >/dev/null; then
  snap list 2>/dev/null | grep -Ev '^(Name|bare|core[0-9][0-9]|snapd\s)' | awk '{print $1}' |
    while IFS= read -r pkg; do [ -n "$pkg" ] && snap remove --purge "$pkg"; done
  if snap list 2>/dev/null | grep -q '^core[0-9][0-9]'; then
    sudo snap remove --purge "$(snap list 2>/dev/null | grep -o '^core[0-9][0-9]' | head -n 1)" || true
  fi
  sudo snap remove --purge bare || true
  sudo snap remove --purge snapd || true
fi
if systemctl -q is-active snapd; then sudo systemctl stop snapd; sudo systemctl disable snapd; fi
if command -v snap >/dev/null; then sudo apt-get purge -qq snapd >/dev/null; fi
if systemctl -q is-active snapd.mounts-pre.target; then sudo systemctl stop snapd.mounts-pre.target; fi
if [ -d "$HOME/snap" ] || [ -d /snap ] || [ -d /var/snap ] || [ -d /var/lib/snapd ]; then
  sudo rm -rf "$HOME/snap" /snap /var/snap /var/lib/snapd
fi
"#;

const APPIMAGED: &str = r#"
arch=$1
if ! systemctl --user -q is-active appimaged; then
  systemctl --user stop appimaged.service || true
  sudo apt-get remove -qy appimagelauncher >/dev/null 2>&1 || true
  [ -f "$HOME/.config/systemd/user/default.target.wants/appimagelauncherd.service" ] &&
    rm "$HOME/.config/systemd/user/default.target.wants/appimagelauncherd.service"
  rm -rf "$HOME/.local/share/applications/appimage"*
  mkdir -p "$HOME/Applications"
  url=$(curl -sSL https://api.github.com/repos/probonopd/go-appimage/releases/tags/continuous |
    yq ".assets[].browser_download_url | select(. == \"*appimaged*${arch}.AppImage\")")
  tmp=$(mktemp "${TMPDIR:-/tmp}/appimaged.XXXXXX")
  trap 'rm -f "$tmp"' EXIT
  curl -fL -o "$tmp" "$url"
  chmod +x "$tmp"
  mv "$tmp" "$HOME/Applications/appimaged.AppImage"
  "$HOME/Applications/appimaged.AppImage"
fi
fuse2_lib=$(apt-cache show libfuse2t64 >/dev/null 2>&1 && printf libfuse2t64 || printf libfuse2)
if ! dpkg -s "$fuse2_lib" >/dev/null 2>&1; then
  sudo apt-get update -qq
  sudo apt-get install -qq "$fuse2_lib"
fi
"#;

const NERDFONT: &str = r#"
font=$1
if [ -z "$(fc-list :family="$font NF")" ]; then
  if [ ! -d "/usr/share/fonts/$font" ]; then
    sudo mkdir -p "/usr/share/fonts/$font"
    tmp=$(mktemp "${TMPDIR:-/tmp}/font.XXXXXX.tar.xz")
    trap 'rm -f "$tmp"' EXIT
    curl -fL -o "$tmp" "https://github.com/ryanoasis/nerd-fonts/releases/latest/download/$font.tar.xz"
    sudo tar -xJ -C "/usr/share/fonts/$font" -f "$tmp" >/dev/null
  fi
  fc-cache -f
fi
"#;

const RESOLVE_URL: &str = r#"
resolve_url() {
expr=$1
shift
for kv in "$@"; do export "$kv"; done
case "$expr" in
  https://*) printf '%s' "$expr"; return ;;
esac
case "$expr" in
  '$('curl\ -sSL\ https://*\ \|\ yq\ *')') ;;
  *) printf 'unsupported URL lookup: %s\n' "$expr" >&2; exit 2 ;;
esac
inner=${expr#'$(curl -sSL '}
inner=${inner%')'}
api=${inner%% | yq *}
filter=${inner#* | yq }
filter=${filter#\"}; filter=${filter%\"}
filter=${filter//\$\{GO_ARCH\}/$GO_ARCH}
filter=${filter//\$\{LINUX_ARCH\}/$LINUX_ARCH}
filter=${filter//\$\{X64_ARCH\}/$X64_ARCH}
filter=${filter//\$\{UNAME_ARCH\}/$UNAME_ARCH}
filter=${filter//\$\{ARM64_SUFFIX\}/$ARM64_SUFFIX}
curl -sSL "$api" | yq "$filter"
}
"#;

const DOWNLOAD_BINARY: &str = r#"
name=$1; expr=$2; shift 2
dest="$HOME/Applications/$name"
cmd=${name%.deb}
[ ! -f "$dest" ] || exit 0
case "$name" in *.deb) command -v "$cmd" >/dev/null && exit 0 ;; esac
mkdir -p "$HOME/Applications"
url=$(resolve_url "$expr" "$@")
tmp=$(mktemp "${TMPDIR:-/tmp}/${name}.XXXXXX")
trap 'rm -f "$tmp"' EXIT
curl -fL -o "$tmp" "$url"
case "$name" in
  *.AppImage) chmod +x "$tmp"; mv "$tmp" "$dest" ;;
  *.deb) mv "$tmp" "$dest"; sudo apt-get install -qq "$dest"; rm -f "$dest" ;;
  *) printf 'unsupported package: %s\n' "$name" >&2; exit 2 ;;
esac
"#;

const GO_INSTALL: &str = r#"
requested=$1; arch=$2
metadata=$(mktemp "${TMPDIR:-/tmp}/go-metadata.XXXXXX.json")
trap 'rm -rf "$metadata" "${tmp:-}" "${stage:-}"' EXIT
curl -fsSL -o "$metadata" "https://go.dev/dl/?mode=json&include=all"
version=$requested
if [ "$version" = latest ]; then version=$(yq '.[0].version' "$metadata" | cut -c 3-); fi
if command -v go >/dev/null && [ "$(go version | cut -d ' ' -f 3)" = "go$version" ]; then exit 0; fi
filename="go${version}.linux-${arch}.tar.gz"
checksum=$(yq ".[] | select(.version == \"go${version}\") | .files[] | select(.filename == \"${filename}\") | .sha256" "$metadata")
[ "${#checksum}" -eq 64 ] && printf '%s' "$checksum" | grep -Eq '^[0-9a-fA-F]+$'
tmp=$(mktemp "${TMPDIR:-/tmp}/go.XXXXXX.tar.gz")
stage=$(mktemp -d "${TMPDIR:-/tmp}/go-stage.XXXXXX")
curl -fL -o "$tmp" "https://go.dev/dl/${filename}"
printf '%s  %s\n' "$checksum" "$tmp" | sha256sum -c -
tar -tzf "$tmp" >/dev/null
tar -C "$stage" -xzf "$tmp"
[ -x "$stage/go/bin/go" ]
sudo rm -rf /usr/local/go
sudo mv "$stage/go" /usr/local/go
"#;

const NODE_INSTALL: &str = r#"
version=$1
fnm_path="${XDG_DATA_HOME:-$HOME/.local/share}/fnm"
if ! command -v fnm >/dev/null; then
  tmp=$(mktemp "${TMPDIR:-/tmp}/fnm-install.XXXXXX")
  trap 'rm -f "$tmp"' EXIT
  curl -fsSL -o "$tmp" https://fnm.vercel.app/install
  bash "$tmp" --skip-shell
  export PATH="$fnm_path:$PATH"
fi
eval "$(fnm env --shell bash)"
if [ "$version" = latest ]; then fnm install --lts --use; else fnm install "$version" --use; fi
fnm default "$(fnm current)"
"#;

const PYENV_INSTALL: &str = r#"
update=$1; version=$2; pip=$3
if ! command -v pyenv >/dev/null; then
  if [ -d "$HOME/.pyenv" ]; then
    printf 'pyenv directory exists but pyenv is not in PATH\n' >&2
    exit 2
  fi
  tmp=$(mktemp "${TMPDIR:-/tmp}/pyenv.XXXXXX")
  trap 'rm -f "$tmp"' EXIT
  curl -fL -o "$tmp" https://pyenv.run
  bash "$tmp"
  export PATH="$HOME/.pyenv/bin:$PATH"
  eval "$(pyenv init - bash)"
else
  [ "$update" != true ] || pyenv update >/dev/null
fi
latest=$(pyenv latest -k "$version")
if [ "$(pyenv version-name)" != "$latest" ]; then
  pyenv versions --bare | grep -Fxq "$latest" || pyenv install "$latest"
  pyenv global "$latest"
fi
if [ "$pip" = true ]; then "python$version" -m pip install -q --upgrade pip; fi
"#;

const UV_INSTALL: &str = r#"
version_enabled=$1; version=$2
if ! command -v uv >/dev/null; then
  tmp=$(mktemp "${TMPDIR:-/tmp}/uv-install.XXXXXX")
  trap 'rm -f "$tmp"' EXIT
  curl -LsSf -o "$tmp" https://astral.sh/uv/install.sh
  sh "$tmp"
  export PATH="$HOME/.local/bin:$PATH"
else
  uv self update -q
fi
if [ "$version_enabled" = true ]; then
  local_py_ver=$(uv python find --managed-python --show-version "$version" 2>/dev/null || true)
  latest_py_ver=$(uv python list "$version" | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -n 1)
  [ -z "$latest_py_ver" ] && exit 0
  [ "$local_py_ver" = "$latest_py_ver" ] || uv python install "$latest_py_ver"
fi
"#;

const DOCKER_CONFIG: &str = r#"
user=$1
command -v docker >/dev/null || exit 0
if ! getent group docker | grep -Fq "$user"; then
  sudo groupadd -f docker
  sudo usermod -aG docker "$user"
  newgrp docker || true
fi
sudo mkdir -p /etc/docker
sudo touch /etc/docker/daemon.json
if [ "$(yq -r '.log-driver' /etc/docker/daemon.json 2>/dev/null || true)" != local ]; then
  printf '%s\n' '{"log-driver":"local","log-opts":{"max-size":"10m"}}' | sudo tee /etc/docker/daemon.json >/dev/null
fi
"#;

const VBOX_CONFIG: &str = r#"
user=$1
command -v virtualbox >/dev/null || exit 0
if ! getent group vboxusers | grep -Fq "$user"; then
  sudo groupadd -f vboxusers
  sudo usermod -aG vboxusers "$user"
  newgrp vboxusers || true
fi
"#;

const VSCODE_EXT: &str = r#"
ext=$1
command -v code >/dev/null || exit 0
code --list-extensions | grep -Fxq "$ext" || code --install-extension "$ext"
"#;

const GNOME_EXT: &str = r#"
ext=$1
command -v gnome-extensions >/dev/null || exit 0
if ! gnome-extensions list | grep -Fxq "$ext"; then
  ver=$(curl -sSL "https://extensions.gnome.org/extension-info/?uuid=$ext" |
    yq '[.shell_version_map[].version] | max')
  tmp=$(mktemp "${TMPDIR:-/tmp}/${ext}.XXXXXX.zip")
  trap 'rm -f "$tmp"' EXIT
  curl -fL -o "$tmp" "https://extensions.gnome.org/extension-data/$(tr -d @ <<<"$ext").v$ver.shell-extension.zip"
  gnome-extensions install --force "$tmp"
else
  gnome-extensions enable "$ext"
fi
"#;

const GNOME_TERMINAL: &str = r#"
term=$1
if gsettings get org.gnome.settings-daemon.plugins.media-keys terminal >/dev/null 2>&1; then
  gsettings set org.gnome.desktop.default-applications.terminal exec "$term"
  gsettings set org.gnome.desktop.default-applications.terminal exec-arg ''
else
  dconf write /org/gnome/settings-daemon/plugins/media-keys/custom-keybindings "['/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/']"
  dconf write /org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/name "'Terminal'"
  dconf write /org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/command "'$term'"
  dconf write /org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/binding "'<Primary><Alt>T'"
fi
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

fn run_if(kind: &str, checked: impl Into<String>, command: Step) -> Step {
    let mut args = vec![kind.into(), checked.into(), command.program];
    args.extend(command.args);
    Step::bash(RUN_IF, args)
}

fn run_if2(kind: &str, a: impl Into<String>, b: impl Into<String>, command: Step) -> Step {
    let mut args = vec![kind.into(), a.into(), b.into(), command.program];
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
                out.push(Step::bash(SNAP_CLEANUP, vec![]));
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
                out.push(run_if2(
                    "group-missing-user",
                    "sudo",
                    user.clone(),
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
        out.push(Step::bash(APPIMAGED, vec![p.uname_arch.clone()]));
    }
    if cfg.tagged_enabled("check.nerdfont") {
        if let Some(font) = cfg.string("check.nerdfont") {
            out.push(Step::bash(NERDFONT, vec![font]));
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
            let url = p.expand(&field_string(b, "url").expect("validated url"));
            let mut args = vec![name, url];
            args.extend(resolve_env(p));
            out.push(Step::bash(
                format!("{RESOLVE_URL}\n{DOWNLOAD_BINARY}"),
                args,
            ));
        }
    }
    if cfg.tagged_enabled("install.languages.goVersion") {
        out.push(Step::bash(
            GO_INSTALL,
            vec![
                cfg.string("install.languages.goVersion")
                    .expect("validated goVersion"),
                p.go_arch.clone(),
            ],
        ));
    }
    if cfg.tagged_enabled("install.languages.nodeVersion") {
        let mut args = vec![cfg
            .string("install.languages.nodeVersion")
            .expect("validated nodeVersion")];
        if cfg.tagged_enabled("install.npm") {
            args.extend(cfg.strings("install.npm"));
        }
        out.push(Step::bash(
            format!("{NODE_INSTALL}\nshift; [ \"$#\" -eq 0 ] || npm install --global \"$@\""),
            args,
        ));
    }
    if cfg.tagged_enabled("install.npm") && !cfg.tagged_enabled("install.languages.nodeVersion") {
        let mut a = vec!["install".into(), "--global".into()];
        a.extend(cfg.strings("install.npm"));
        out.push(run_if("command", "npm", Step::owned("npm", a)));
    }
    if cfg.tagged_enabled("install.languages.pyenv") {
        out.push(Step::bash(
            PYENV_INSTALL,
            vec![
                cfg.bool("install.languages.pyenv.update").to_string(),
                cfg.string("install.languages.pyenv.version")
                    .expect("validated pyenv version"),
                cfg.bool("install.languages.pyenv.pip").to_string(),
            ],
        ));
    }
    if cfg.tagged_enabled("install.languages.uv") {
        out.push(Step::bash(
            UV_INSTALL,
            vec![
                cfg.tagged_enabled("install.languages.uv.version")
                    .to_string(),
                cfg.string("install.languages.uv.version")
                    .expect("validated uv version"),
            ],
        ));
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
    if cfg.bool("update.other.yq") {
        out.push(Step::bash(
            "local_ver=$(yq -V 2>/dev/null | cut -d ' ' -f 4 || true); latest=$(curl -sSL https://api.github.com/repos/mikefarah/yq/releases/latest | yq '.tag_name'); [ \"$local_ver\" = \"$latest\" ] && exit 0; tmp=$(mktemp \"${TMPDIR:-/tmp}/yq.XXXXXX\"); trap 'rm -f \"$tmp\"' EXIT; curl -fL -o \"$tmp\" \"https://github.com/mikefarah/yq/releases/latest/download/yq_linux_$1\"; chmod +x \"$tmp\"; sudo mv \"$tmp\" /usr/bin/yq",
            vec![p.go_arch.clone()],
        ));
    }
    if cfg.bool("update.other.go") {
        out.push(run_if(
            "command",
            "go",
            Step::bash(GO_INSTALL, vec!["latest".into(), p.go_arch.clone()]),
        ));
    }
    if cfg.bool("update.other.node") {
        out.push(run_if(
            "command",
            "fnm",
            Step::bash(NODE_INSTALL, vec!["latest".into()]),
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
        out.push(Step::bash(DOCKER_CONFIG, vec![user.clone()]));
    }
    if cfg.bool("configure.apps.virtualbox") {
        out.push(Step::bash(VBOX_CONFIG, vec![user]));
    }
    if cfg.tagged_enabled("configure.apps.vscodeExtensions") {
        for ext in cfg.strings("configure.apps.vscodeExtensions") {
            out.push(Step::bash(VSCODE_EXT, vec![ext]));
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
            out.push(Step::bash(GNOME_TERMINAL, vec![term]));
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
                out.push(Step::bash(GNOME_EXT, vec![ext]));
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

fn resolve_env(p: &Platform) -> Vec<String> {
    vec![
        format!("GO_ARCH={}", p.go_arch),
        format!("LINUX_ARCH={}", p.linux_arch),
        format!("X64_ARCH={}", p.x64_arch),
        format!("UNAME_ARCH={}", p.uname_arch),
        format!("ARM64_SUFFIX={}", p.arm64_suffix),
    ]
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
