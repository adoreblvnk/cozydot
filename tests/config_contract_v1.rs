use serde_yaml::{Mapping, Value};
use std::collections::BTreeSet;

const BEGINNER: &str = include_str!("../docs/examples/config-v1-beginner.yaml");
const FULL: &str = include_str!("../docs/examples/full.yaml");

fn parse(document: &str) -> Value {
    serde_yaml::from_str(document).expect("canonical contract fixture must be valid YAML")
}

fn mapping<'a>(value: &'a Value, path: &str) -> &'a Mapping {
    value
        .as_mapping()
        .unwrap_or_else(|| panic!("{path} must be a mapping"))
}

fn field<'a>(map: &'a Mapping, name: &str, path: &str) -> &'a Value {
    map.get(Value::String(name.to_owned()))
        .unwrap_or_else(|| panic!("{path}.{name} must be present"))
}

fn string_set<'a>(value: &'a Value, path: &str) -> BTreeSet<&'a str> {
    value
        .as_sequence()
        .unwrap_or_else(|| panic!("{path} must be a sequence"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("{path} entries must be strings"))
        })
        .collect()
}

#[test]
fn canonical_fixtures_use_only_exact_version_1_0_0() {
    for (name, document) in [("beginner", BEGINNER), ("full", FULL)] {
        let root = parse(document);
        let root = mapping(&root, name);
        assert_eq!(
            field(root, "version", name),
            &Value::String("1.0.0".to_owned()),
            "{name}"
        );
        assert!(
            !root.contains_key(Value::String("schema".to_owned())),
            "{name}"
        );
    }
}

#[test]
fn canonical_yaml_does_not_expose_removed_or_internal_vocabulary() {
    for (name, document) in [("beginner", BEGINNER), ("full", FULL)] {
        for forbidden in [
            "schema:",
            "direct:",
            "provides:",
            "release:",
            "layout:",
            "rustup",
            "fnm",
            "uv:",
        ] {
            assert!(
                !document.to_ascii_lowercase().contains(forbidden),
                "{name} exposes forbidden vocabulary {forbidden:?}"
            );
        }
    }
}

#[test]
fn full_fixture_uses_flat_typed_apt_repositories() {
    let root = parse(FULL);
    let root = mapping(&root, "full");
    let packages = mapping(field(root, "packages", "full"), "packages");
    let apt = mapping(field(packages, "apt", "packages"), "packages.apt");
    let repositories = field(apt, "repositories", "packages.apt")
        .as_sequence()
        .expect("packages.apt.repositories must be a sequence");

    assert!(!repositories.is_empty());
    for (index, repository) in repositories.iter().enumerate() {
        let path = format!("packages.apt.repositories[{index}]");
        let repository = mapping(repository, &path);
        for required in ["name", "key", "urls", "packages"] {
            field(repository, required, &path);
        }
        for removed in ["source", "layout", "type", "architecture", "signed_by"] {
            assert!(
                !repository.contains_key(Value::String(removed.to_owned())),
                "{path}.{removed} must not exist"
            );
        }

        let suite = repository.contains_key(Value::String("suite".to_owned()));
        let components = repository.contains_key(Value::String("components".to_owned()));
        let exact_path = repository.contains_key(Value::String("path".to_owned()));
        assert_eq!(suite, components, "{path}: suite and components are paired");
        assert_ne!(suite, exact_path, "{path}: choose suite/components or path");
    }
}

#[test]
fn full_fixture_requires_ubuntu_or_debian_and_gnome() {
    let root = parse(FULL);
    let root = mapping(&root, "full");
    let system = mapping(field(root, "system", "full"), "system");
    let require = mapping(field(system, "require", "system"), "system.require");

    assert_eq!(
        string_set(
            field(require, "distros", "system.require"),
            "system.require.distros"
        ),
        BTreeSet::from(["debian", "ubuntu"])
    );
    assert_eq!(
        string_set(
            field(require, "desktops", "system.require"),
            "system.require.desktops"
        ),
        BTreeSet::from(["gnome"])
    );
}

#[test]
fn full_fixture_uses_practical_repository_and_binary_shapes() {
    let root = parse(FULL);
    let root = mapping(&root, "full");
    let packages = mapping(field(root, "packages", "full"), "packages");
    let apt = mapping(field(packages, "apt", "packages"), "packages.apt");
    let repositories = field(apt, "repositories", "packages.apt")
        .as_sequence()
        .expect("repositories must be a sequence");
    assert!(repositories.iter().any(|repository| {
        mapping(repository, "repository")
            .get(Value::String("suite".to_owned()))
            .and_then(Value::as_str)
            == Some("*")
    }));
    let binaries = field(packages, "binaries", "packages")
        .as_sequence()
        .expect("packages.binaries must be a sequence");
    let providers = binaries
        .iter()
        .map(|binary| {
            let binary = mapping(binary, "binary");
            let source = mapping(field(binary, "source", "binary"), "binary.source");
            field(source, "provider", "binary.source")
                .as_str()
                .expect("provider must be a string")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(providers, BTreeSet::from(["github"]));
    assert!(binaries.iter().all(|binary| {
        let binary = mapping(binary, "binary");
        let source = mapping(field(binary, "source", "binary"), "binary.source");
        !source.contains_key(Value::String("sha256".to_owned()))
    }));
}
