use cozydot::{
    config::{AptUpdate, BinarySource, Config, HttpsUrl, SourceMode, Theme},
    platform::Platform,
};

const BEGINNER: &str = include_str!("../docs/examples/config-v1-beginner.yaml");
const FULL: &str = include_str!("../docs/examples/config-v1-full.yaml");
const EXHAUSTIVE: &str = include_str!("../docs/examples/config-v1-exhaustive.yaml");

fn platform(distro: &str, upstream: &str, codename: &str, desktop: &str, arch: &str) -> Platform {
    Platform::from_release_parts(
        distro.into(),
        upstream.into(),
        codename.into(),
        codename.into(),
        desktop.into(),
        arch,
    )
    .unwrap()
}

fn error(yaml: &str) -> String {
    Config::parse(yaml).unwrap_err().to_string()
}

fn reject(yaml: &str, expected: &str) {
    let error = error(yaml);
    assert!(
        error.contains(expected),
        "expected {expected:?} in {error:?}\n{yaml}"
    );
}

fn reject_with_path_and_value(yaml: &str, expected_path: &str, invalid_value: &str) {
    let error = error(yaml);
    assert!(
        error.contains(expected_path),
        "expected path {expected_path:?} in {error:?}\n{yaml}"
    );
    assert!(
        error.contains(invalid_value),
        "expected value {invalid_value:?} in {error:?}\n{yaml}"
    );
}

#[test]
fn canonical_fixtures_parse_and_validate_claimed_platforms() {
    Config::parse(BEGINNER).unwrap();
    let full = Config::parse(FULL).unwrap();
    full.validate_for_platform(&platform("ubuntu", "ubuntu", "noble", "gnome", "amd64"))
        .unwrap();
    full.validate_for_platform(&platform("debian", "debian", "trixie", "gnome", "amd64"))
        .unwrap();
    Config::parse(EXHAUSTIVE)
        .unwrap()
        .validate_for_platform(&platform("ubuntu", "ubuntu", "noble", "gnome", "amd64"))
        .unwrap();
}

#[test]
fn exact_version_preflight_matrix() {
    Config::parse("version: 1.0.0").unwrap();
    for yaml in [
        "{}",
        "version: 2.0.0",
        "version: '1.0'",
        "version: 1",
        "version: true",
        "version: null",
        "version: []",
        "version: {}",
    ] {
        reject(yaml, "version");
    }
}

#[test]
fn null_empty_and_redundant_false_are_rejected() {
    for yaml in [
        "version: 1.0.0\nsystem: null",
        "version: 1.0.0\nsystem: {}",
        "version: 1.0.0\npackages: {}",
        "version: 1.0.0\npackages:\n  flatpak: []",
        "version: 1.0.0\nsystem:\n  ensure_admin: false",
        "version: 1.0.0\ndesktop:\n  gnome:\n    dock: false",
        "version: 1.0.0\nupdates:\n  flatpak: false",
    ] {
        assert!(Config::parse(yaml).is_err(), "accepted {yaml}");
    }
    Config::parse("version: 1.0.0\ndesktop:\n  idle:\n    dim: false").unwrap();
}

#[test]
fn recursive_unknown_fields_and_removed_forms_are_rejected() {
    for yaml in [
        "schema: 1",
        "version: 1.0.0\napps: {}",
        "version: 1.0.0\nsystem:\n  distro: auto",
        "version: 1.0.0\npackages:\n  direct: []",
        "version: 1.0.0\npackages:\n  apt: [curl]",
        "version: 1.0.0\nupdates:\n  apt: off",
    ] {
        assert!(Config::parse(yaml).is_err(), "accepted {yaml}");
    }
}

#[test]
fn yaml_extension_and_duplicate_key_matrix() {
    for yaml in [
        "%YAML 1.2\n---\nversion: 1.0.0",
        "%TAG !e! tag:example.com,2026:\n---\nversion: 1.0.0",
        "version: !custom 1.0.0",
        "version: &v 1.0.0\ntools: {rust: *v}",
        "version: 1.0.0\n---\nversion: 1.0.0",
        "version: 1.0.0\nversion: 1.0.0",
    ] {
        assert!(Config::parse(yaml).is_err(), "accepted {yaml}");
    }
    Config::parse("version: 1.0.0 # !tag &anchor *alias %YAML\nfonts:\n  nerd: ['literal-name']")
        .unwrap();
}

#[test]
fn repository_layout_star_path_and_system_rules() {
    let repository = |fields: &str| {
        format!("version: 1.0.0\npackages:\n  apt:\n    repositories:\n      - name: repo\n        key: https://example.com/key\n        urls: {{default: https://example.com/repo}}\n{fields}        packages: [pkg]\n")
    };
    Config::parse(&repository(
        "        suite: \"*\"\n        components: [\"*\"]\n",
    ))
    .unwrap();
    Config::parse(&repository("        path: ./\n")).unwrap();
    for fields in [
        "        suite: stable\n",
        "        components: [main]\n",
        "        suite: stable\n        components: [main]\n        path: ./\n",
        "        suite: stable\n        components: [system]\n",
        "        path: /absolute/\n",
        "        path: ../bad/\n",
    ] {
        assert!(
            Config::parse(&repository(fields)).is_err(),
            "accepted {fields}"
        );
    }
    let config = Config::parse(&repository(
        "        suite: system\n        components: [main]\n",
    ))
    .unwrap();
    reject_platform(
        &config,
        &platform("ubuntu", "ubuntu", "noble", "gnome", "amd64"),
        "default URL",
    );
}

fn reject_platform(config: &Config, platform: &Platform, expected: &str) {
    let error = config
        .validate_for_platform(platform)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(expected),
        "expected {expected:?} in {error:?}"
    );
}

#[test]
fn binary_provider_hash_key_and_native_architecture_matrix() {
    let fixed = |urls: &str, hashes: &str| {
        format!("version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: deb\n      commands: [app]\n      source:\n        provider: url\n        urls: {urls}\n        sha256: {hashes}\n")
    };
    let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let config = Config::parse(&fixed(
        "{amd64: https://example.com/app.deb}",
        &format!("{{amd64: {hash}}}"),
    ))
    .unwrap();
    config
        .validate_for_platform(&platform("ubuntu", "ubuntu", "noble", "none", "amd64"))
        .unwrap();
    reject_platform(
        &config,
        &platform("ubuntu", "ubuntu", "noble", "none", "arm64"),
        "packages.binaries[0].source.arm64",
    );
    for yaml in [
        fixed("{}", "{}"),
        fixed(
            "{amd64: https://example.com/app.deb}",
            &format!("{{arm64: {hash}}}"),
        ),
        fixed("{amd64: https://example.com/app.deb}", "{amd64: ABCD}"),
    ] {
        assert!(Config::parse(&yaml).is_err(), "accepted {yaml}");
    }
}

#[test]
fn scalar_grammar_matrix() {
    for yaml in [
        "version: 1.0.0\npackages:\n  apt:\n    install: ['curl;id']",
        "version: 1.0.0\npackages:\n  cargo: ['crate@1']\ntools: {rust: stable}",
        "version: 1.0.0\npackages:\n  npm: ['Name']\ntools: {node: lts}",
        "version: 1.0.0\npackages:\n  flatpak: [com.example]",
        "version: 1.0.0\ntools: {rust: nightly-2026-02-29}",
        "version: 1.0.0\ndesktop:\n  idle: {timeout: 1d}",
    ] {
        assert!(Config::parse(yaml).is_err(), "accepted {yaml}");
    }
}

#[test]
fn package_tool_and_update_effectiveness_rules() {
    reject("version: 1.0.0\npackages:\n  cargo: [bat]", "tools.rust");
    reject("version: 1.0.0\npackages:\n  npm: [pkg]", "tools.node");
    for yaml in [
        "version: 1.0.0\ntools: {rust: '1.85'}\nupdates:\n  tools: {rust: true}",
        "version: 1.0.0\ntools: {go: '1.24'}\nupdates:\n  tools: {go: true}",
        "version: 1.0.0\nupdates:\n  packages: {cargo: true}",
        "version: 1.0.0\nupdates:\n  packages: {binaries: true}",
        "version: 1.0.0\nupdates:\n  fonts: true",
    ] {
        assert!(Config::parse(yaml).is_err(), "accepted {yaml}");
    }
    Config::parse("version: 1.0.0\ntools: {rust: '1.85', node: '22', python: '3.13'}").unwrap();
}

#[test]
fn platform_requirements_components_and_desktop_rules() {
    let required = Config::parse(
        "version: 1.0.0\nsystem:\n  require:\n    distros: [ubuntu]\n    desktops: [gnome]",
    )
    .unwrap();
    reject_platform(
        &required,
        &platform("debian", "debian", "trixie", "gnome", "amd64"),
        "not allowed",
    );
    reject_platform(
        &required,
        &platform("ubuntu", "ubuntu", "noble", "none", "amd64"),
        "not allowed",
    );
    let desktop = Config::parse("version: 1.0.0\ndesktop: {theme: dark}").unwrap();
    reject_platform(
        &desktop,
        &platform("ubuntu", "ubuntu", "noble", "none", "amd64"),
        "GNOME or Cinnamon",
    );
    desktop
        .validate_for_platform(&platform("ubuntu", "ubuntu", "noble", "cinnamon", "amd64"))
        .unwrap();
    let gnome = Config::parse("version: 1.0.0\ndesktop:\n  gnome: {dock: true}").unwrap();
    reject_platform(
        &gnome,
        &platform("ubuntu", "ubuntu", "noble", "cinnamon", "amd64"),
        "requires resolved GNOME",
    );
}

#[test]
fn urls_and_binary_provider_fields_are_strict() {
    let repository = |key: &str, url: &str| {
        format!(
        "version: 1.0.0\npackages:\n  apt:\n    repositories:\n      - name: repo\n        key: {key:?}\n        urls: {{default: {url:?}}}\n        suite: stable\n        components: [main]\n        packages: [pkg]\n"
    )
    };
    for invalid in [
        "http://example.com/key",
        "https:///key",
        "https://.",
        "https://_bad.example/key",
        "https://example..com/key",
        "https://-example.com/key",
        "https://example-.com/key",
        "https://example.com\\evil",
        "https://%65xample.com/key",
        "https://example.com:/key",
        "https://2130706433/key",
        "https://127.1/key",
        "https://user@example.com/key",
        "https://example.com/key#fragment",
        "https://example.com/${ARCH}",
    ] {
        assert!(Config::parse(&repository(invalid, "https://example.com/repo")).is_err());
        assert!(Config::parse(&repository("https://example.com/key", invalid)).is_err());
    }
    Config::parse(&repository(
        "https://münchen.example/key",
        "https://192.0.2.1/repo",
    ))
    .unwrap();
    for yaml in [
        "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: deb\n      commands: [app]\n      source: {provider: github, repository: owner/repo, assets: {amd64: {include: 'app-*.deb'}}, urls: {amd64: https://example.com/app.deb}}",
        "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: deb\n      commands: [app]\n      source: {provider: url, urls: {amd64: https://example.com/app.deb}, sha256: {amd64: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}, repository: owner/repo}",
        "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: deb\n      commands: [app]\n      source: {provider: other}",
    ] {
        assert!(Config::parse(yaml).is_err(), "accepted {yaml}");
    }
}

#[test]
fn platform_selection_uses_exact_then_upstream_then_default() {
    let managed = Config::parse(
        "version: 1.0.0\nsystem:\n  apt:\n    sources:\n      mode: managed\n      components:\n        ubuntu: [main, universe]\n        debian: [main, contrib, non-free, non-free-firmware]",
    )
    .unwrap();
    managed
        .validate_for_platform(&platform("ubuntu", "ubuntu", "noble", "none", "amd64"))
        .unwrap();
    managed
        .validate_for_platform(&platform("debian", "debian", "trixie", "none", "amd64"))
        .unwrap();
    reject_platform(
        &managed,
        &platform("pop", "ubuntu", "noble", "none", "amd64"),
        "use preserve",
    );

    let repository = Config::parse(
        "version: 1.0.0\npackages:\n  apt:\n    repositories:\n      - name: repo\n        key: https://example.com/key\n        urls:\n          ubuntu: https://ubuntu.example/repo\n        suite: system\n        components: [main]\n        packages: [pkg]",
    )
    .unwrap();
    let pop = Platform::from_release_parts(
        "pop".into(),
        "ubuntu".into(),
        "cosmic".into(),
        "noble".into(),
        "none".into(),
        "amd64",
    )
    .unwrap();
    repository.validate_for_platform(&pop).unwrap();
    reject_platform(
        &repository,
        &platform("debian", "debian", "trixie", "none", "amd64"),
        "no URL",
    );
}

#[test]
fn binary_commands_have_one_global_definition_owner() {
    let yaml = "version: 1.0.0
packages:
  binaries:
    - name: first
      format: appimage
      commands: [shared-command]
      source:
        provider: github
        repository: owner/first
        assets: {amd64: {include: 'first-*.AppImage'}}
    - name: second
      format: appimage
      commands: [shared-command]
      source:
        provider: github
        repository: owner/second
        assets: {amd64: {include: 'second-*.AppImage'}}";
    let error = error(yaml);
    assert!(
        error.contains("packages.binaries[0].commands[0]"),
        "{error}"
    );
    assert!(
        error.contains("packages.binaries[1].commands[0]"),
        "{error}"
    );
    assert!(error.contains("shared-command"), "{error}");
}

#[test]
fn scalar_diagnostics_report_complete_path_and_rejected_value() {
    for (yaml, path, value) in [
        (
            "version: 1.0.0\ntools: {rust: nightly-2026-02-29}",
            "tools.rust",
            "nightly-2026-02-29",
        ),
        (
            "version: 1.0.0\npackages:\n  apt:\n    repositories:\n      - name: repo\n        key: https://_bad.example/key\n        urls: {default: https://example.com/repo}\n        suite: stable\n        components: [main]\n        packages: [pkg]",
            "packages.apt.repositories[0].key",
            "https://_bad.example/key",
        ),
        (
            "version: 1.0.0\npackages:\n  apt:\n    install: ['curl;id']",
            "packages.apt.install[0]",
            "curl;id",
        ),
        (
            "version: 1.0.0\npackages:\n  apt:\n    repositories:\n      - name: repo\n        key: https://example.com/key\n        urls: {default: https://example.com/repo}\n        path: ../bad/\n        packages: [pkg]",
            "packages.apt.repositories[0].path",
            "../bad/",
        ),
        (
            "version: 1.0.0\ndesktop:\n  idle: {timeout: 1d}",
            "desktop.idle.timeout",
            "1d",
        ),
        (
            "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: appimage\n      commands: [app]\n      source:\n        provider: github\n        repository: owner/app\n        assets: {amd64: {include: 'app-[0-9].AppImage'}}",
            "packages.binaries[0].source.assets.amd64.include",
            "app-[0-9].AppImage",
        ),
        (
            "version: 1.0.0\npackages:\n  binaries:\n    - name: 'bad/name'\n      format: appimage\n      commands: [app]\n      source:\n        provider: github\n        repository: owner/app\n        assets: {amd64: {include: 'app-*.AppImage'}}",
            "packages.binaries[0].name",
            "bad/name",
        ),
        (
            "version: 1.0.0\npackages:\n  binaries:\n    - name: app\n      format: deb\n      commands: [app]\n      source:\n        provider: url\n        urls: {amd64: https://example.com/app.deb}\n        sha256: {amd64: ABCD}",
            "packages.binaries[0].source.sha256.amd64",
            "ABCD",
        ),
    ] {
        reject_with_path_and_value(yaml, path, value);
    }
}

#[test]
fn desktop_terminal_parse_validation_is_grammar_only() {
    for yaml in [
        "version: 1.0.0\ndesktop: {terminal: externally-installed-terminal}",
        "version: 1.0.0\npackages:\n  apt:\n    install: [gnome-terminal]\ndesktop: {terminal: gnome}",
        "version: 1.0.0\npackages:\n  apt:\n    install: [command-name-extra]\ndesktop: {terminal: command-name}",
        "version: 1.0.0\npackages:\n  cargo: [alacritty]\ntools: {rust: stable}\ndesktop: {terminal: alacritty}",
    ] {
        Config::parse(yaml).unwrap_or_else(|error| panic!("rejected {yaml}: {error}"));
    }
    reject(
        "version: 1.0.0\npackages:\n  apt:\n    install: [gnome-terminal]\ndesktop: {terminal: 'gnome terminal'}",
        "desktop.terminal",
    );
    Config::parse("version: 1.0.0\nintegrations:\n  docker: {add_user_to_group: true}").unwrap();
}

#[test]
fn platform_validation_rechecks_mutated_and_directly_deserialized_models() {
    let target = platform("ubuntu", "ubuntu", "noble", "none", "amd64");

    let mut mutated = Config::parse("version: 1.0.0\ntools: {rust: stable}").unwrap();
    mutated.tools.as_mut().unwrap().rust = Some("not valid".into());
    reject_platform(&mutated, &target, "tools.rust");

    let deserialized: Config = serde_yaml::from_str("version: 1.0.0\ntools: {}").unwrap();
    reject_platform(&deserialized, &target, "tools");
}

#[test]
fn planner_facing_model_surface_is_public_and_typed() {
    fn accepts_public_https_url(_: &HttpsUrl) {}

    let config = Config::parse(FULL).unwrap();
    accepts_public_https_url(
        &config
            .packages
            .as_ref()
            .and_then(|packages| packages.apt.as_ref())
            .and_then(|apt| apt.repositories.as_ref())
            .and_then(|repositories| repositories.first())
            .expect("full fixture has a repository")
            .key,
    );

    assert_eq!(
        config
            .system
            .as_ref()
            .and_then(|system| system.apt.as_ref())
            .and_then(|apt| apt.sources.as_ref())
            .map(|sources| sources.mode),
        Some(SourceMode::Managed)
    );
    assert_eq!(
        config.desktop.as_ref().and_then(|desktop| desktop.theme),
        Some(Theme::Dark)
    );
    assert_eq!(
        config.updates.as_ref().and_then(|updates| updates.apt),
        Some(AptUpdate::Full)
    );
    assert!(config
        .packages
        .as_ref()
        .and_then(|packages| packages.binaries.as_ref())
        .is_some_and(|binaries| binaries
            .iter()
            .any(|binary| matches!(&binary.source, BinarySource::Github { .. }))));
}
