use anyhow::{bail, Context, Result};
use serde_yaml::{Mapping, Value};
use std::{fs, path::Path};

#[derive(Debug, Clone)]
pub struct Config {
    root: Value,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let root: Value =
            serde_yaml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        let cfg = Self { root };
        cfg.validate()
            .with_context(|| format!("validate {}", path.display()))?;
        Ok(cfg)
    }

    pub fn at(&self, path: &str) -> Option<&Value> {
        let mut value = &self.root;
        for part in path.split('.') {
            value = untag(value).as_mapping()?.get(Value::String(part.into()))?;
        }
        Some(value)
    }

    pub fn enabled(&self, path: &str) -> bool {
        tag(self.at(path)).as_deref() != Some("!disabled")
    }

    pub fn tagged_enabled(&self, path: &str) -> bool {
        tag(self.at(path)).as_deref() == Some("!enabled")
    }

    pub fn bool(&self, path: &str) -> bool {
        self.at(path)
            .and_then(|v| untag(v).as_bool())
            .unwrap_or(false)
    }

    pub fn string(&self, path: &str) -> Option<String> {
        self.at(path).and_then(|v| match untag(v) {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    }

    pub fn strings(&self, path: &str) -> Vec<String> {
        self.at(path)
            .and_then(|v| untag(v).as_sequence())
            .into_iter()
            .flatten()
            .filter_map(|v| untag(v).as_str().map(str::to_owned))
            .collect()
    }

    pub fn sequence(&self, path: &str) -> Vec<&Value> {
        self.at(path)
            .and_then(|v| untag(v).as_sequence())
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    fn validate(&self) -> Result<()> {
        let root = mapping(&self.root, "")?;
        expect_keys(
            root,
            "",
            &["metadata", "check", "install", "update", "configure"],
        )?;
        self.validate_metadata()?;
        self.validate_check()?;
        self.validate_install()?;
        self.validate_update()?;
        self.validate_configure()?;
        Ok(())
    }

    fn validate_metadata(&self) -> Result<()> {
        let m = self.map_at("metadata")?;
        expect_keys(m, "metadata", &["description", "distro", "DE"])?;
        self.required_string("metadata.description")?;
        self.required_string("metadata.distro")?;
        self.required_string("metadata.DE")?;
        Ok(())
    }

    fn validate_check(&self) -> Result<()> {
        let m = self.map_at("check")?;
        expect_keys(
            m,
            "check",
            &[
                "distroCfg",
                "purge",
                "deps",
                "rustupCheck",
                "appimaged",
                "nerdfont",
            ],
        )?;
        self.required_bool("check.distroCfg")?;
        self.required_tagged_string_sequence("check.purge")?;
        self.required_tagged_string_sequence("check.deps")?;
        self.required_bool("check.rustupCheck")?;
        self.required_bool("check.appimaged")?;
        self.required_tagged_scalar("check.nerdfont")?;
        validate_name(self.string("check.nerdfont").as_deref(), "check.nerdfont")?;
        Ok(())
    }

    fn validate_install(&self) -> Result<()> {
        let m = self.map_at("install")?;
        expect_keys(
            m,
            "install",
            &[
                "check",
                "apt",
                "addRepos",
                "flatpak",
                "cargo",
                "npm",
                "binaries",
                "languages",
            ],
        )?;
        self.required_bool("install.check")?;
        self.required_tagged_string_sequence("install.apt")?;
        self.required_tagged_string_sequence("install.flatpak")?;
        self.required_tagged_string_sequence("install.cargo")?;
        self.required_tagged_string_sequence("install.npm")?;
        self.validate_add_repos()?;
        self.validate_binaries()?;
        self.validate_languages()?;
        Ok(())
    }

    fn validate_add_repos(&self) -> Result<()> {
        self.required_tagged_sequence("install.addRepos")?;
        for (i, repo) in self.sequence("install.addRepos").into_iter().enumerate() {
            let path = format!("install.addRepos[{i}]");
            let m = mapping(repo, &path)?;
            expect_keys(
                m,
                &path,
                &[
                    "sourceName",
                    "remoteKey",
                    "keyPath",
                    "repo",
                    "pinning",
                    "packages",
                ],
            )?;
            validate_name(
                field_string(repo, "sourceName").as_deref(),
                &format!("{path}.sourceName"),
            )?;
            validate_urlish(
                field_string(repo, "remoteKey").as_deref(),
                &format!("{path}.remoteKey"),
            )?;
            validate_abs_path(
                field_string(repo, "keyPath").as_deref(),
                &format!("{path}.keyPath"),
            )?;
            validate_repo(
                field_string(repo, "repo").as_deref(),
                &format!("{path}.repo"),
            )?;
            let packages = field(repo, "packages")
                .and_then(|v| untag(v).as_sequence())
                .ok_or_else(|| anyhow::anyhow!("{path}.packages must be a sequence"))?;
            for (j, pkg) in packages.iter().enumerate() {
                validate_pkg(untag(pkg).as_str(), &format!("{path}.packages[{j}]"))?;
            }
        }
        Ok(())
    }

    fn validate_binaries(&self) -> Result<()> {
        self.required_tagged_sequence("install.binaries")?;
        for (i, binary) in self.sequence("install.binaries").into_iter().enumerate() {
            let path = format!("install.binaries[{i}]");
            let m = mapping(binary, &path)?;
            expect_keys(m, &path, &["name", "url"])?;
            let name = field_string(binary, "name")
                .ok_or_else(|| anyhow::anyhow!("{path}.name must be a string"))?;
            validate_binary_name(&name, &format!("{path}.name"))?;
            validate_urlish(
                field_string(binary, "url").as_deref(),
                &format!("{path}.url"),
            )?;
        }
        Ok(())
    }

    fn validate_languages(&self) -> Result<()> {
        let m = self.map_at("install.languages")?;
        expect_keys(
            m,
            "install.languages",
            &["goVersion", "nodeVersion", "pyenv", "uv"],
        )?;
        self.required_tagged_version("install.languages.goVersion")?;
        self.required_tagged_version("install.languages.nodeVersion")?;
        self.required_tagged_map("install.languages.pyenv")?;
        let py = self.map_at("install.languages.pyenv")?;
        expect_keys(py, "install.languages.pyenv", &["update", "version", "pip"])?;
        self.required_bool("install.languages.pyenv.update")?;
        self.required_version("install.languages.pyenv.version")?;
        self.required_bool("install.languages.pyenv.pip")?;
        self.required_tagged_map("install.languages.uv")?;
        let uv = self.map_at("install.languages.uv")?;
        expect_keys(uv, "install.languages.uv", &["version"])?;
        self.required_tagged_version("install.languages.uv.version")?;
        Ok(())
    }

    fn validate_update(&self) -> Result<()> {
        let m = self.map_at("update")?;
        expect_keys(m, "update", &["check", "apt", "flatpak", "cargo", "other"])?;
        self.required_bool("update.check")?;
        self.required_tagged_map("update.apt")?;
        let apt = self.map_at("update.apt")?;
        expect_keys(apt, "update.apt", &["aptFull"])?;
        self.required_bool("update.apt.aptFull")?;
        self.required_bool("update.flatpak")?;
        self.required_bool("update.cargo")?;
        let other = self.map_at("update.other")?;
        expect_keys(other, "update.other", &["yq", "go", "node"])?;
        self.required_bool("update.other.yq")?;
        self.required_bool("update.other.go")?;
        self.required_bool("update.other.node")?;
        Ok(())
    }

    fn validate_configure(&self) -> Result<()> {
        let m = self.map_at("configure")?;
        expect_keys(
            m,
            "configure",
            &["check", "dotfiles", "apps", "desktopEnvironment"],
        )?;
        self.required_bool("configure.check")?;
        self.required_tagged_map("configure.dotfiles")?;
        let dotfiles = self.map_at("configure.dotfiles")?;
        expect_keys(dotfiles, "configure.dotfiles", &["stowMode", "packages"])?;
        match self
            .required_string("configure.dotfiles.stowMode")?
            .as_str()
        {
            "override" | "backup" => {}
            other => bail!("configure.dotfiles.stowMode has unsupported value {other:?}"),
        }
        self.required_string_sequence("configure.dotfiles.packages")?;
        let apps = self.map_at("configure.apps")?;
        expect_keys(
            apps,
            "configure.apps",
            &["docker", "virtualbox", "vscodeExtensions"],
        )?;
        self.required_bool("configure.apps.docker")?;
        self.required_bool("configure.apps.virtualbox")?;
        self.required_tagged_string_sequence("configure.apps.vscodeExtensions")?;
        self.required_tagged_map("configure.desktopEnvironment")?;
        let de = self.map_at("configure.desktopEnvironment")?;
        expect_keys(
            de,
            "configure.desktopEnvironment",
            &["common", "gnome", "cinnamon"],
        )?;
        self.required_tagged_map("configure.desktopEnvironment.common")?;
        let common = self.map_at("configure.desktopEnvironment.common")?;
        expect_keys(
            common,
            "configure.desktopEnvironment.common",
            &["defaultTerm"],
        )?;
        self.required_tagged_scalar("configure.desktopEnvironment.common.defaultTerm")?;
        validate_name(
            self.string("configure.desktopEnvironment.common.defaultTerm")
                .as_deref(),
            "configure.desktopEnvironment.common.defaultTerm",
        )?;
        self.required_tagged_map("configure.desktopEnvironment.gnome")?;
        let gnome = self.map_at("configure.desktopEnvironment.gnome")?;
        expect_keys(
            gnome,
            "configure.desktopEnvironment.gnome",
            &[
                "settings",
                "extensions",
                "MacOSDock",
                "smoothRoundedCorners",
            ],
        )?;
        self.required_bool("configure.desktopEnvironment.gnome.settings")?;
        self.required_tagged_string_sequence("configure.desktopEnvironment.gnome.extensions")?;
        for (i, ext) in self
            .strings("configure.desktopEnvironment.gnome.extensions")
            .iter()
            .enumerate()
        {
            validate_extension(
                ext,
                &format!("configure.desktopEnvironment.gnome.extensions[{i}]"),
            )?;
        }
        self.required_bool("configure.desktopEnvironment.gnome.MacOSDock")?;
        self.required_bool("configure.desktopEnvironment.gnome.smoothRoundedCorners")?;
        validate_tag(
            self.at("configure.desktopEnvironment.cinnamon"),
            "configure.desktopEnvironment.cinnamon",
        )?;
        Ok(())
    }

    fn map_at(&self, path: &str) -> Result<&Mapping> {
        let v = self
            .at(path)
            .ok_or_else(|| anyhow::anyhow!("missing required field {path}"))?;
        mapping(v, path)
    }

    fn required_string(&self, path: &str) -> Result<String> {
        self.string(path)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("{path} must be a non-empty string"))
    }

    fn required_version(&self, path: &str) -> Result<String> {
        let s = self.required_string(path)?;
        validate_version(&s, path)?;
        Ok(s)
    }

    fn required_bool(&self, path: &str) -> Result<()> {
        match self.at(path).map(untag).and_then(Value::as_bool) {
            Some(_) => Ok(()),
            None => bail!("{path} must be a boolean"),
        }
    }

    fn required_tagged_scalar(&self, path: &str) -> Result<()> {
        validate_tag(self.at(path), path)?;
        self.required_string(path)?;
        Ok(())
    }

    fn required_tagged_version(&self, path: &str) -> Result<()> {
        validate_tag(self.at(path), path)?;
        self.required_version(path)?;
        Ok(())
    }

    fn required_tagged_sequence(&self, path: &str) -> Result<()> {
        validate_tag(self.at(path), path)?;
        match self.at(path).map(untag).and_then(Value::as_sequence) {
            Some(_) => Ok(()),
            None => bail!("{path} must be a sequence"),
        }
    }

    fn required_tagged_string_sequence(&self, path: &str) -> Result<()> {
        self.required_tagged_sequence(path)?;
        for (i, v) in self.sequence(path).into_iter().enumerate() {
            if untag(v).as_str().filter(|s| !s.trim().is_empty()).is_none() {
                bail!("{path}[{i}] must be a non-empty string");
            }
        }
        Ok(())
    }

    fn required_string_sequence(&self, path: &str) -> Result<()> {
        match self.at(path).map(untag).and_then(Value::as_sequence) {
            Some(values) => {
                for (i, v) in values.iter().enumerate() {
                    if untag(v).as_str().filter(|s| !s.trim().is_empty()).is_none() {
                        bail!("{path}[{i}] must be a non-empty string");
                    }
                }
                Ok(())
            }
            None => bail!("{path} must be a sequence"),
        }
    }

    fn required_tagged_map(&self, path: &str) -> Result<()> {
        validate_tag(self.at(path), path)?;
        self.map_at(path).map(|_| ())
    }
}

pub fn untag(mut value: &Value) -> &Value {
    while let Value::Tagged(t) = value {
        value = &t.value;
    }
    value
}

pub fn tag(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Tagged(t) => Some(t.tag.to_string()),
        _ => None,
    }
}

pub fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    untag(value).as_mapping()?.get(Value::String(name.into()))
}

pub fn field_string(value: &Value, name: &str) -> Option<String> {
    field(value, name).and_then(|v| match untag(v) {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn mapping<'a>(value: &'a Value, path: &str) -> Result<&'a Mapping> {
    untag(value)
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("{path} must be a mapping"))
}

fn expect_keys(map: &Mapping, path: &str, allowed: &[&str]) -> Result<()> {
    for key in map.keys() {
        let Some(key) = key.as_str() else {
            bail!("{path} contains a non-string key");
        };
        if !allowed.contains(&key) {
            bail!("{path} contains unknown field {key}");
        }
    }
    for key in allowed {
        if !map.contains_key(Value::String((*key).into())) {
            bail!("{path} missing required field {key}");
        }
    }
    Ok(())
}

fn validate_tag(value: Option<&Value>, path: &str) -> Result<()> {
    match tag(value).as_deref() {
        Some("!enabled" | "!disabled") => Ok(()),
        Some(other) => bail!("{path} has unsupported tag {other}"),
        None => bail!("{path} must be tagged !enabled or !disabled"),
    }
}

fn validate_name(value: Option<&str>, path: &str) -> Result<()> {
    let value = value.ok_or_else(|| anyhow::anyhow!("{path} must be a string"))?;
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && !value.is_empty()
    {
        Ok(())
    } else {
        bail!("{path} contains unsupported characters")
    }
}

fn validate_pkg(value: Option<&str>, path: &str) -> Result<()> {
    let value = value.ok_or_else(|| anyhow::anyhow!("{path} must be a string"))?;
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-+._*".contains(&b))
        && !value.is_empty()
    {
        Ok(())
    } else {
        bail!("{path} contains unsupported package characters")
    }
}

fn validate_binary_name(value: &str, path: &str) -> Result<()> {
    validate_name(Some(value), path)?;
    if value.ends_with(".AppImage") || value.ends_with(".deb") {
        Ok(())
    } else {
        bail!("{path} must end with .AppImage or .deb")
    }
}

fn validate_abs_path(value: Option<&str>, path: &str) -> Result<()> {
    let value = value.ok_or_else(|| anyhow::anyhow!("{path} must be a string"))?;
    if value.starts_with('/')
        && !value.contains("..")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-._".contains(&b))
    {
        Ok(())
    } else {
        bail!("{path} must be an absolute safe path")
    }
}

fn validate_repo(value: Option<&str>, path: &str) -> Result<()> {
    let value = value.ok_or_else(|| anyhow::anyhow!("{path} must be a string"))?;
    if value.starts_with("deb ") && !value.contains('\n') && !value.contains(';') {
        Ok(())
    } else {
        bail!("{path} must be a single deb repository line")
    }
}

fn validate_urlish(value: Option<&str>, path: &str) -> Result<()> {
    let value = value.ok_or_else(|| anyhow::anyhow!("{path} must be a string"))?;
    if value.starts_with("https://") {
        return Ok(());
    }
    if value.starts_with("$(curl -sSL https://") && value.ends_with(')') && !value.contains('\n') {
        return Ok(());
    }
    bail!("{path} must be an https URL or supported GitHub curl/yq lookup")
}

fn validate_version(value: &str, path: &str) -> Result<()> {
    if value == "latest"
        || value
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        Ok(())
    } else {
        bail!("{path} contains unsupported version characters")
    }
}

fn validate_extension(value: &str, path: &str) -> Result<()> {
    if value.contains('@')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_@.".contains(&b))
    {
        Ok(())
    } else {
        bail!("{path} contains unsupported extension characters")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_custom_tags_without_leaking() {
        let c: Value = serde_yaml::from_str("x: !enabled [a]").unwrap();
        let c = Config { root: c };
        assert!(c.tagged_enabled("x"));
        assert_eq!(c.strings("x"), ["a"]);
    }

    #[test]
    fn rejects_unknown_top_level_fields() {
        let err = Config::load(Path::new("tests/fixtures/invalid-extra.yaml")).unwrap_err();
        assert!(err.to_string().contains("validate"));
    }
}
