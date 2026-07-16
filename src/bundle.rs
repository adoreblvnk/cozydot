#[derive(Clone, Copy, Debug)]
pub struct Record {
    pub path: &'static str,
    pub bytes: &'static [u8],
    pub mode: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct PresetRecord {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

pub fn records() -> &'static [Record] {
    RECORDS
}

pub fn preset(name: &str) -> Option<&'static PresetRecord> {
    PRESETS.iter().find(|preset| preset.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Component, Path};

    #[test]
    fn embedded_bundle_is_complete_sorted_unique_safe_and_nonempty() {
        assert!(!RECORDS.is_empty());
        for pair in RECORDS.windows(2) {
            assert!(pair[0].path < pair[1].path);
        }
        for record in RECORDS {
            let path = Path::new(record.path);
            assert!(!record.bytes.is_empty(), "{} is empty", record.path);
            assert!(path
                .components()
                .all(|part| matches!(part, Component::Normal(_))));
            assert!(!record.path.contains(['\t', '\n', '\r']));
            assert_eq!(
                record.mode,
                if record.bytes.starts_with(b"#!") {
                    0o755
                } else {
                    0o644
                },
                "{} has a nondeterministic mode",
                record.path
            );
        }
        assert!(RECORDS
            .iter()
            .any(|record| record.path == "dotfiles/bash/.bashrc"));
        let script = RECORDS
            .iter()
            .find(|record| record.path == "dotfiles/bin/.local/bin/round")
            .unwrap();
        assert_ne!(script.mode & 0o111, 0);
    }

    #[test]
    fn embedded_presets_are_sorted_unique_valid_and_nonempty() {
        assert_eq!(
            PRESETS.iter().map(|preset| preset.name).collect::<Vec<_>>(),
            ["cli", "cozydot", "full", "vm"]
        );
        for preset in PRESETS {
            assert!(!preset.bytes.is_empty(), "{} is empty", preset.name);
            assert!(!preset.bytes.windows(2).any(|bytes| bytes == b"\n\n"));
            let text = std::str::from_utf8(preset.bytes).unwrap();
            crate::config::Config::parse(text).unwrap();
        }
    }
}
