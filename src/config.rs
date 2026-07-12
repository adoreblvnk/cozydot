use anyhow::{bail, Context, Result};
use serde_yaml::Value;
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
        for key in ["metadata", "check", "install", "update", "configure"] {
            if cfg.at(key).is_none() {
                bail!("missing required config section: {key}")
            }
        }
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
        tag(self.at(path)) != Some("!disabled")
    }
    pub fn tagged_enabled(&self, path: &str) -> bool {
        tag(self.at(path)) == Some("!enabled")
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
}

pub fn untag(mut value: &Value) -> &Value {
    while let Value::Tagged(t) = value {
        value = &t.value;
    }
    value
}
pub fn tag(value: Option<&Value>) -> Option<&str> {
    match value? {
        Value::Tagged(t) => Some(t.tag.to_string().leak()),
        _ => None,
    }
}
pub fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    untag(value).as_mapping()?.get(Value::String(name.into()))
}
pub fn field_string(value: &Value, name: &str) -> Option<String> {
    field(value, name)
        .and_then(|v| untag(v).as_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_custom_tags() {
        let c: Value = serde_yaml::from_str("x: !enabled [a]").unwrap();
        let c = Config { root: c };
        assert!(c.tagged_enabled("x"));
        assert_eq!(c.strings("x"), ["a"]);
    }
}
