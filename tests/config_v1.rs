use cozydot::{
    config::v1::ConfigV1,
    platform::{Architecture, Platform},
};

const MINIMAL: &str = include_str!("fixtures/config-v1-minimal.yaml");
const FULL: &str = include_str!("fixtures/config-v1-full.yaml");

fn error(yaml: &str) -> String {
    ConfigV1::parse(yaml).unwrap_err().to_string()
}

fn assert_rejected(yaml: &str, expected: &str) {
    let message = error(yaml);
    assert!(
        message.contains(expected),
        "expected {expected:?} in error:\n{message}\nYAML:\n{yaml}"
    );
}

fn platform(distro: &str, upstream: &str, architecture: Architecture) -> Platform {
    Platform::from_parts(
        distro.into(),
        upstream.into(),
        "noble".into(),
        "gnome".into(),
        architecture.canonical(),
    )
    .unwrap()
}

#[test]
fn parses_minimal_and_canonical_full_fixtures() {
    let minimal = ConfigV1::parse(MINIMAL).unwrap();
    assert_eq!(minimal.schema, 1);
    assert!(minimal.system.is_none());

    let full = ConfigV1::parse(FULL).unwrap();
    for platform in [
        platform("ubuntu", "ubuntu", Architecture::Amd64),
        platform("debian", "debian", Architecture::Arm64),
    ] {
        full.validate_for_platform(&platform).unwrap();
    }
    for architecture in [Architecture::Arm32, Architecture::Riscv64] {
        let platform = platform("ubuntu", "ubuntu", architecture);
        let message = full
            .validate_for_platform(&platform)
            .unwrap_err()
            .to_string();
        assert!(message.contains(&format!(
            "packages.direct[0].source.assets.{}",
            architecture.canonical()
        )));
        assert!(message.contains("obsidian"));
    }
}

#[test]
fn canonical_documentation_and_full_fixture_cannot_drift() {
    let documentation = include_str!("../docs/config-schema-v1.md");
    let documented = documentation
        .split_once("```yaml\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    assert_eq!(documented.trim(), FULL.trim());
    ConfigV1::parse(documented).unwrap();
}

#[test]
fn schema_is_required_and_strictly_integer_one() {
    for (yaml, expected) in [
        ("{}", "missing field `schema`"),
        ("schema: 2", "unsupported schema version 2"),
        ("schema: 0", "unsupported schema version 0"),
        ("schema: -1", "unsupported schema version -1"),
        ("schema: '1'", "integer 1"),
        ("schema: true", "integer 1"),
        ("schema: 1.0", "integer 1"),
        ("schema: null", "integer 1"),
    ] {
        assert_rejected(yaml, expected);
    }
    assert!(error("schema: 2").contains("cozydot init"));
}

#[test]
fn rejects_unknown_fields_and_wrong_yaml_shapes_with_paths() {
    for (yaml, expected) in [
        ("schema: 1\nextra: true", "unknown field `extra`"),
        (
            "schema: 1\nsystem:\n  apt:\n    mystery: true",
            "system.apt",
        ),
        ("schema: 1\npackages: []", "packages"),
        ("schema: 1\npackages:\n  apt: curl", "packages.apt"),
        (
            "schema: 1\ndesktop:\n  idle:\n    timeout: 15",
            "desktop.idle.timeout",
        ),
    ] {
        assert_rejected(yaml, expected);
    }
    assert!(ConfigV1::parse("schema: 1\n---\nschema: 1").is_err());
}

#[test]
fn yaml_extension_preflight_rejects_tokens_not_literal_characters() {
    for (yaml, expected) in [
        (
            "%TAG !e! tag:example.com,2026:\n---\nschema: 1",
            "YAML tags",
        ),
        (
            "%TAG !e! tag:example.com,2026:\n---\nschema: 1\nsystem: !e!enabled {}",
            "YAML tags",
        ),
        (
            r#"{schema: 1, fonts: {nerd: ["Name\\"]}, system: &shared {}, desktop: *shared}"#,
            "YAML anchors",
        ),
        (
            "schema: 1\nsystem: &shared\n  ensure_admin: true\ndesktop: *shared",
            "YAML anchors",
        ),
        (
            "schema: 1\nsystem: {ensure_admin: &enabled true}\ndesktop: {gnome: {dock: *enabled}}",
            "YAML anchors",
        ),
        ("schema: 1\nsystem: *shared", "YAML aliases"),
        ("{schema: 1, system: *shared}", "YAML aliases"),
        ("schema: 1\nfonts:\n  nerd: [!custom Name]", "YAML tags"),
        ("schema: 1\n---\nschema: 1", "multiple YAML documents"),
    ] {
        assert_rejected(yaml, expected);
    }

    for yaml in [
        r#"{schema: 1, fonts: {nerd: ["Name\\", "literal ! & *"]}}"#,
        "schema: 1\nfonts:\n  nerd:\n    - '!tag &anchor *alias' # !comment &ignored *ignored",
        "schema: 1\nfonts:\n  nerd:\n    - >-\n      literal !tag\n      second &anchor and *alias\n",
        "schema: 1\nfonts:\n  nerd:\n    - plain-value # !tag &anchor *alias\n",
        "schema: 1 # %TAG !e! tag:example.com &anchor *alias\n",
        "schema: 1\nfonts:\n  nerd:\n    - '%TAG !e! tag:example.com &anchor *alias'\n",
        "%YAML 1.2\n---\nschema: 1\nfonts:\n  nerd:\n    - >-\n      literal %TAG ! & *\n",
    ] {
        ConfigV1::parse(yaml).unwrap_or_else(|error| panic!("YAML {yaml:?}: {error}"));
    }
}

#[test]
fn validates_system_enums_components_and_dependencies() {
    for (yaml, expected) in [
        ("schema: 1\nsystem:\n  distro: Ubuntu", "system.distro"),
        ("schema: 1\nsystem:\n  desktop: KDE", "system.desktop"),
        (
            "schema: 1\nsystem:\n  apt:\n    components: [main]",
            "system.apt.components",
        ),
        (
            "schema: 1\nsystem:\n  apt:\n    sources: preserve\n    components: [main]",
            "sources: managed",
        ),
        (
            "schema: 1\nsystem:\n  apt:\n    sources: managed\n    components: []",
            "non-empty sequence",
        ),
        (
            "schema: 1\nsystem:\n  apt:\n    sources: managed\n    components: [main, main]",
            "duplicate value",
        ),
        (
            "schema: 1\nsystem:\n  distro: ubuntu\n  apt:\n    sources: managed\n    components: [contrib]",
            "unsupported by the configured distribution family",
        ),
        (
            "schema: 1\nsystem:\n  distro: debian\n  apt:\n    sources: managed\n    components: [universe]",
            "unsupported by the configured distribution family",
        ),
    ] {
        assert_rejected(yaml, expected);
    }

    let auto = ConfigV1::parse(
        "schema: 1\nsystem:\n  distro: auto\n  apt:\n    sources: managed\n    components: [contrib]",
    )
    .unwrap();
    auto.validate_for_platform(&platform("debian", "debian", Architecture::Amd64))
        .unwrap();
    assert!(auto
        .validate_for_platform(&platform("ubuntu", "ubuntu", Architecture::Amd64))
        .unwrap_err()
        .to_string()
        .contains("system.apt.components[0]"));
    let unsupported = Platform::from_parts(
        "arch".into(),
        "arch".into(),
        "rolling".into(),
        "none".into(),
        "amd64",
    )
    .unwrap();
    assert!(auto
        .validate_for_platform(&unsupported)
        .unwrap_err()
        .to_string()
        .contains("system.distro"));
    assert!(auto
        .validate_for_platform(&platform("ubuntu", "debian", Architecture::Amd64))
        .unwrap_err()
        .to_string()
        .contains("upstream family"));

    for distro in [
        "auto",
        "ubuntu",
        "linuxmint",
        "pop",
        "zorin",
        "deepin",
        "debian",
        "kali",
        "tails",
    ] {
        ConfigV1::parse(&format!("schema: 1\nsystem:\n  distro: {distro}")).unwrap();
    }
}

#[test]
fn validates_repository_urls_content_and_selection() {
    for (yaml, expected) in [
        (
            "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: http://example.com/key\n      source: { urls: { default: https://example.com/repo }, suite: stable, components: [main] }\n      packages: [pkg]",
            "packages.repositories[0].key",
        ),
        (
            "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: https://example.com/key\n      source: { urls: {}, suite: stable, components: [main] }\n      packages: [pkg]",
            "must contain default",
        ),
        (
            "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: https://example.com/key\n      source: { urls: { arch: https://example.com/repo }, suite: stable, components: [main] }\n      packages: [pkg]",
            "unknown field `arch`",
        ),
        (
            "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: https://example.com/key\n      source: { urls: { default: https://example.com/repo }, suite: '', components: [main] }\n      packages: [pkg]",
            "source.suite",
        ),
        (
            "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: https://example.com/key\n      source: { urls: { default: https://example.com/repo }, suite: stable, components: [] }\n      packages: [pkg]",
            "source.components",
        ),
        (
            "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: https://example.com/key\n      source: { urls: { default: https://example.com/repo }, suite: stable, components: [main] }\n      packages: []",
            "repositories[0].packages",
        ),
    ] {
        assert_rejected(yaml, expected);
    }

    assert_rejected(
        "schema: 1\nsystem:\n  distro: debian\npackages:\n  repositories:\n    - name: repo\n      key: https://example.com/key\n      source: { urls: { ubuntu: https://ubuntu.example }, suite: stable, components: [main] }\n      packages: [pkg]",
        "packages.repositories[0].source.urls",
    );

    let config = ConfigV1::parse(FULL).unwrap();
    let urls = &config.packages.unwrap().repositories.unwrap()[0]
        .source
        .urls;
    assert_eq!(
        urls.select("ubuntu").unwrap(),
        "https://cli.github.com/packages"
    );
    assert!(urls.select("unsupported").is_err());
    let distro_only = ConfigV1::parse(
        "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: https://example.com/key\n      source: { urls: { ubuntu: https://ubuntu.example }, suite: stable, components: [main] }\n      packages: [pkg]",
    )
    .unwrap();
    let distro_only = &distro_only.packages.unwrap().repositories.unwrap()[0]
        .source
        .urls;
    assert!(distro_only
        .select("debian")
        .unwrap_err()
        .to_string()
        .contains("no URL"));
}

#[test]
fn repository_urls_are_parsed_https_urls_without_credentials_or_fragments() {
    let repository = |key: &str, url: &str| {
        format!(
            "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: {key:?}\n      source: {{ urls: {{ default: {url:?} }}, suite: stable, components: [main] }}\n      packages: [pkg]"
        )
    };
    for invalid in [
        "http://example.com/key",
        "ftp://example.com/key",
        "https:///key",
        "https://.",
        "https://..",
        "https://-",
        "https://_bad.example",
        "https://example..com",
        "https://-example.com",
        "https://example-.com",
        "https://example.com\\evil",
        &format!("https://{}.example", "a".repeat(64)),
        "https://@:443/key",
        "https://user@example.com/key",
        "https://user:pass@example.com/key",
        "https://example.com/key#fragment",
        "https://example.com/a b",
        "https://example.com/${ARCH}",
        "https://example.com/\tkey",
    ] {
        assert_rejected(&repository(invalid, "https://example.com/repo"), ".key");
        assert_rejected(
            &repository("https://example.com/key", invalid),
            ".source.urls.default",
        );
    }

    for valid in [
        "https://example.com",
        "https://registry",
        "https://xn--mnchen-3ya.example",
        "https://münchen.example",
        "https://192.0.2.1",
        "https://[2001:db8::1]",
        "https://example.com/path/to/key?format=gpg",
        "https://[2001:db8::1]:8443/repository?channel=stable",
    ] {
        ConfigV1::parse(&repository(valid, valid)).unwrap();
    }
}

#[test]
fn rejects_repository_name_duplicates_empty_stems_and_collisions() {
    let repository = |name: &str| {
        format!(
            "    - name: {name:?}\n      key: https://example.com/key\n      source: {{ urls: {{ default: https://example.com/repo }}, suite: stable, components: [main] }}\n      packages: [pkg]\n"
        )
    };
    assert_rejected(
        &format!(
            "schema: 1\npackages:\n  repositories:\n{}{}",
            repository("same"),
            repository("same")
        ),
        "duplicate repository name",
    );
    assert_rejected(
        &format!(
            "schema: 1\npackages:\n  repositories:\n{}",
            repository("///")
        ),
        "empty repository filename stem",
    );
    assert_rejected(
        &format!(
            "schema: 1\npackages:\n  repositories:\n{}{}",
            repository("GitHub CLI"),
            repository("github_cli")
        ),
        "filename stem \"github-cli\" collides",
    );
    for name in [
        "${ARCH}",
        "$ARCH",
        "{{ distro }}",
        "{% distro %}",
        "line\nbreak",
    ] {
        assert_rejected(
            &format!(
                "schema: 1\npackages:\n  repositories:\n{}",
                repository(name)
            ),
            "packages.repositories[0].name",
        );
    }
}

#[test]
fn rejects_duplicate_and_empty_package_list_entries() {
    for (yaml, expected) in [
        ("schema: 1\npackages:\n  apt: [curl, curl]", "packages.apt[1]"),
        ("schema: 1\npackages:\n  cargo: ['']", "packages.cargo[0]"),
        ("schema: 1\npackages:\n  apt: [123]", "packages.apt[0]"),
        (
            "schema: 1\npackages:\n  cargo: ['bat --force']",
            "unversioned Cargo package name",
        ),
        ("schema: 1\nfonts:\n  nerd: [GeistMono, GeistMono]", "fonts.nerd[1]"),
        (
            "schema: 1\nintegrations:\n  vscode:\n    extensions: [rust-lang.rust-analyzer, rust-lang.rust-analyzer]",
            "integrations.vscode.extensions[1]",
        ),
        ("schema: 1\ndotfiles:\n  packages: []", "dotfiles.packages"),
        (
            "schema: 1\ndotfiles:\n  packages: [../bash]",
            "not a path",
        ),
    ] {
        assert_rejected(yaml, expected);
    }
}

#[test]
fn validates_manager_specific_package_identifier_grammars() {
    ConfigV1::parse(
        "schema: 1\npackages:\n  remove: [libc6]\n  apt: [g++, libssl3, foo.bar]\n  cargo: [serde-json, Cargo_Edit]\n  npm: [opencode-ai, '@scope/package_name']\n  flatpak: [com.bitwarden.desktop, org.gnome.Builder]",
    )
    .unwrap();

    for value in [
        "Curl",
        "curl:amd64",
        "curl|id",
        "curl&&id",
        "curl>file",
        "$(id)",
        "-curl",
        "curl/id",
    ] {
        assert_rejected(
            &format!("schema: 1\npackages:\n  apt: [{value:?}]"),
            "packages.apt[0]",
        );
        assert_rejected(
            &format!("schema: 1\npackages:\n  remove: [{value:?}]"),
            "packages.remove[0]",
        );
        let yaml = format!(
            "schema: 1\npackages:\n  repositories:\n    - name: repo\n      key: https://example.com/key\n      source: {{ urls: {{ default: https://example.com/repo }}, suite: stable, components: [main] }}\n      packages: [{value:?}]"
        );
        assert_rejected(&yaml, "packages.repositories[0].packages[0]");
    }

    for value in [
        "bat@1.0",
        "--locked",
        "owner/crate",
        "crate=1",
        "curl|id",
        "curl&&id",
        "crate>file",
        "$(id)",
        "crate name",
    ] {
        assert_rejected(
            &format!("schema: 1\npackages:\n  cargo: [{value:?}]"),
            "packages.cargo[0]",
        );
    }

    for value in [
        "Package",
        "package@1.0",
        "@scope/package@1",
        "@/package",
        "@scope/",
        "@scope/name/extra",
        "@scope",
        "--flag",
        "curl|id",
        "curl&&id",
        "package>file",
        "$(id)",
        "package name",
    ] {
        assert_rejected(
            &format!("schema: 1\npackages:\n  npm: [{value:?}]"),
            "packages.npm[0]",
        );
    }

    for value in [
        "com.example",
        "com..App",
        "com.example.app-id",
        "1com.example.App",
        "com.example.App/ref",
        "com.example.App@stable",
        "curl|id",
        "curl&&id",
        "com.example.App>file",
        "$(id)",
        "com.example.App name",
    ] {
        assert_rejected(
            &format!("schema: 1\npackages:\n  flatpak: [{value:?}]"),
            "packages.flatpak[0]",
        );
    }
}

#[test]
fn validates_direct_package_shape_coordinates_and_selectors() {
    let direct = |body: &str| format!("schema: 1\npackages:\n  direct:\n{body}");
    let base = |assets: &str| {
        format!(
            "    - name: app\n      format: deb\n      provides: [app]\n      source:\n        type: github\n        repository: owner/repo\n        assets:\n{assets}"
        )
    };
    for (yaml, expected) in [
        (
            direct("    - name: app\n      format: tar\n      provides: [app]\n      source: { type: github, repository: owner/repo, assets: { amd64: { include: 'app-*.deb', exclude: [] } } }\n"),
            "packages.direct[0].format",
        ),
        (
            direct("    - name: app\n      format: deb\n      provides: app\n      source: { type: github, repository: owner/repo, assets: { amd64: { include: 'app-*.deb', exclude: [] } } }\n"),
            "packages.direct[0].provides",
        ),
        (
            direct("    - name: app\n      format: deb\n      provides: []\n      source: { type: github, repository: owner/repo, assets: { amd64: { include: 'app-*.deb', exclude: [] } } }\n"),
            "packages.direct[0].provides",
        ),
        (
            direct("    - name: app\n      format: deb\n      provides: [app, app]\n      source: { type: github, repository: owner/repo, assets: { amd64: { include: 'app-*.deb', exclude: [] } } }\n"),
            "duplicate value",
        ),
        (
            direct("    - name: app\n      format: deb\n      provides: [/usr/bin/app]\n      source: { type: github, repository: owner/repo, assets: { amd64: { include: 'app-*.deb', exclude: [] } } }\n"),
            "must start with an ASCII alphanumeric",
        ),
        (direct(&base("          amd64: { include: 'app-*.deb', exclude: [] }\n").replace("owner/repo", "owner/repo/extra")), "source.repository"),
        (direct(&base("          x86_64: { include: 'app-*.deb', exclude: [] }\n")), "unknown field `x86_64`"),
        (direct(&base("          amd64: 'app-*.deb'\n")), "source.assets.amd64"),
        (direct(&base("          amd64: { exclude: [] }\n")), "missing field `include`"),
        (direct(&base("          amd64: { include: 'app-*.deb' }\n")), "missing field `exclude`"),
        (direct(&base("          amd64: { include: app.deb, exclude: [] }\n")), "anchored filename wildcard"),
        (direct(&base("          amd64: { include: '../app-*.deb', exclude: [] }\n")), "without paths"),
        (direct(&base("          amd64: { include: '${ARCH}-*.deb', exclude: [] }\n")), "without paths or substitutions"),
        (direct(&base("          amd64: { include: '$ARCH-*.deb', exclude: [] }\n")), "without paths or substitutions"),
        (direct(&base("          amd64: { include: 'app-[0-9]*.deb', exclude: [] }\n")), "anchored filename wildcard"),
        (direct(&base("          amd64: { include: 'app-*.deb', exclude: ['app-?.deb', 'app-?.deb'] }\n")), "duplicate value"),
    ] {
        assert_rejected(&yaml, expected);
    }

    let duplicate = direct(
        "    - name: app\n      format: deb\n      provides: [app]\n      source: { type: github, repository: owner/repo, assets: { amd64: { include: 'app-*.deb', exclude: [] } } }\n    - name: app\n      format: appimage\n      provides: [app2]\n      source: { type: github, repository: owner/repo2, assets: { amd64: { include: 'app-*.AppImage', exclude: [] } } }\n",
    );
    assert_rejected(&duplicate, "duplicate direct-package name");

    for name in [
        ".app",
        "-app",
        "app/name",
        "app name",
        "${APP}",
        "$APP",
        "{{ app }}",
        "app;id",
        "app\nname",
    ] {
        assert_rejected(
            &direct(
                &base("          amd64: { include: 'app-*.deb', exclude: [] }\n")
                    .replace("name: app", &format!("name: {name:?}")),
            ),
            "packages.direct[0].name",
        );
    }
    ConfigV1::parse(&direct(
        &base("          amd64: { include: 'app-*.deb', exclude: [] }\n")
            .replace("name: app", "name: App.image_1"),
    ))
    .unwrap();
}

#[test]
fn validates_tool_versions_and_requires_yaml_strings() {
    for (yaml, expected) in [
        ("schema: 1\ntools:\n  python: 3.13", "tools.python"),
        ("schema: 1\ntools:\n  python: '3'", "invalid version"),
        ("schema: 1\ntools:\n  go: stable", "invalid version"),
        ("schema: 1\ntools:\n  node: v22", "invalid version"),
        ("schema: 1\ntools:\n  rust: '../stable'", "invalid version"),
        (
            "schema: 1\ntools:\n  rust: 1.85.0-x86_64-unknown-linux-gnu",
            "tools.rust",
        ),
        (
            "schema: 1\ntools:\n  rust: stable-x86_64-unknown-linux-gnu",
            "tools.rust",
        ),
        (
            "schema: 1\ntools:\n  rust: nightly-2026-02-29",
            "tools.rust",
        ),
    ] {
        assert_rejected(yaml, expected);
    }
    for rust in [
        "stable",
        "beta",
        "nightly",
        "nightly-2026-07-14",
        "1.85",
        "1.85.0",
    ] {
        ConfigV1::parse(&format!("schema: 1\ntools:\n  rust: {rust:?}")).unwrap();
    }
    ConfigV1::parse(
        "schema: 1\ntools: { rust: nightly-2026-07-14, go: '1.24.5', node: '22', python: '3.13.5' }",
    )
    .unwrap();
}

#[test]
fn validates_durations_docker_sizes_integrations_and_desktop_ids() {
    for duration in ["-1m", "15", "1h30m", "1d", "", "é"] {
        assert_rejected(
            &format!("schema: 1\ndesktop:\n  idle:\n    timeout: {duration:?}"),
            "desktop.idle.timeout",
        );
    }
    ConfigV1::parse("schema: 1\ndesktop:\n  idle:\n    timeout: 0s\n    dim: false").unwrap();
    for (yaml, expected) in [
        (
            "schema: 1\nintegrations:\n  docker: true",
            "integrations.docker",
        ),
        (
            "schema: 1\nintegrations:\n  virtualbox: false",
            "integrations.virtualbox",
        ),
        (
            "schema: 1\nintegrations:\n  docker:\n    max_log_size: 10m",
            "local_log_driver: true",
        ),
        (
            "schema: 1\nintegrations:\n  docker:\n    local_log_driver: true\n    max_log_size: 0m",
            "positive integer",
        ),
        ("schema: 1\ndesktop:\n  theme: blue", "desktop.theme"),
        (
            "schema: 1\ndesktop:\n  terminal: /usr/bin/wezterm",
            "desktop.terminal",
        ),
        (
            "schema: 1\ndesktop:\n  gnome:\n    extensions: [not-a-uuid]",
            "desktop.gnome.extensions[0]",
        ),
        (
            "schema: 1\nintegrations:\n  vscode:\n    extensions: [rust-analyzer]",
            "integrations.vscode.extensions[0]",
        ),
    ] {
        assert_rejected(yaml, expected);
    }
}

#[test]
fn executable_names_use_one_safe_ascii_basename_grammar() {
    let direct = |executable: &str| {
        format!(
            "schema: 1\npackages:\n  direct:\n    - name: app\n      format: deb\n      provides: [{executable:?}]\n      source: {{ type: github, repository: owner/repo, assets: {{ amd64: {{ include: 'app-*.deb', exclude: [] }} }} }}"
        )
    };

    for valid in ["wezterm", "cargo-binstall", "c++", "app.image_1"] {
        ConfigV1::parse(&format!("schema: 1\ndesktop:\n  terminal: {valid:?}")).unwrap();
        ConfigV1::parse(&direct(valid)).unwrap();
    }

    for invalid in [
        "",
        ";",
        "app;rm",
        "app|less",
        "app&bg",
        "app>file",
        "app<input",
        "`app`",
        "$(app)",
        "/usr/bin/app",
        "dir/app",
        "dir\\app",
        "app name",
        "app\tname",
        ".app",
        "-app",
        "_app",
        "+app",
    ] {
        assert_rejected(
            &format!("schema: 1\ndesktop:\n  terminal: {invalid:?}"),
            "desktop.terminal",
        );
        assert_rejected(&direct(invalid), "packages.direct[0].provides[0]");
    }
}

#[test]
fn rejects_removed_legacy_tagged_and_shorthand_forms() {
    for (yaml, expected) in [
        ("metadata: {}", "schema"),
        ("schema: 1\napps: {}", "unknown field `apps`"),
        ("schema: 1\nsystem: !enabled {}", "YAML tags"),
        (
            "schema: 1\nsystem: &shared {}\ndesktop: *shared",
            "YAML anchors",
        ),
        ("schema: 1\ndotfiles: [bash]", "dotfiles"),
        ("schema: 1\ndesktop: gnome", "desktop"),
        ("schema: 1\nupdates:\n  apt: true", "updates.apt"),
        ("schema: 1\nupdates:\n  apt:\n    full: true", "updates.apt"),
        ("schema: 1\nupdates:\n  apt: disabled", "unknown variant"),
    ] {
        assert_rejected(yaml, expected);
    }
}

#[test]
fn omission_null_false_and_empty_collections_remain_distinct_typed_values() {
    let config = ConfigV1::parse(
        "schema: 1\nsystem:\n  ensure_admin: null\n  apt:\n    unattended_upgrades: false\n  ubuntu:\n    snap: false\n    codecs: false\npackages:\n  apt: []\nintegrations:\n  docker:\n    add_user_to_group: false\ndesktop:\n  idle:\n    timeout: null\n    dim: false\n  gnome:\n    extensions: []\n    dock: false\nupdates:\n  apt: null\n  flatpak: false",
    )
    .unwrap();
    let system = config.system.unwrap();
    assert_eq!(system.ensure_admin, None);
    assert_eq!(system.apt.unwrap().unattended_upgrades, Some(false));
    assert_eq!(system.ubuntu.unwrap().snap, Some(false));
    assert_eq!(config.packages.unwrap().apt, Some(vec![]));
    assert_eq!(config.updates.unwrap().apt, None);
}

#[test]
fn update_prerequisite_and_empty_list_cases_are_valid_no_op_intent() {
    ConfigV1::parse(
        "schema: 1\npackages:\n  flatpak: []\n  cargo: []\n  npm: []\n  direct: []\nupdates:\n  flatpak: true\n  tools: { rust: true, go: true, node: true }\n  packages: { cargo: true, npm: true, direct: true }",
    )
    .unwrap();
}
