use cozydot::config::Config;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{
    meta::ParseNestedMeta, AngleBracketedGenericArguments, Attribute, Fields, GenericArgument,
    Item, ItemEnum, ItemStruct, LitStr, PathArguments, Type,
};

const CONFIGURATION_REFERENCE: &str = include_str!("../docs/configuration.md");

#[derive(Default)]
struct SerdeOptions {
    rename: Option<String>,
    rename_all: Option<String>,
    tag: Option<String>,
    skip: bool,
}

#[derive(Clone, Copy)]
enum SerdeSite {
    Struct,
    Enum,
    Field,
    Variant,
}

fn serde_name(meta: ParseNestedMeta<'_>) -> syn::Result<String> {
    if meta.input.peek(syn::Token![=]) {
        return meta.value()?.parse::<LitStr>().map(|value| value.value());
    }
    let mut deserialize = None;
    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("deserialize") {
            deserialize = Some(nested.value()?.parse::<LitStr>()?.value());
            Ok(())
        } else if nested.path.is_ident("serialize") {
            let _ = nested.value()?.parse::<LitStr>()?;
            Ok(())
        } else {
            Err(nested.error("unsupported serde rename option in configuration model"))
        }
    })?;
    deserialize.ok_or_else(|| meta.error("serde rename must specify deserialize name"))
}

fn serde_options(attrs: &[Attribute], site: SerdeSite) -> Result<SerdeOptions, String> {
    let mut options = SerdeOptions::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") && matches!(site, SerdeSite::Field | SerdeSite::Variant)
            {
                options.rename = Some(serde_name(meta)?);
            } else if meta.path.is_ident("rename_all")
                && matches!(site, SerdeSite::Struct | SerdeSite::Enum)
            {
                options.rename_all = Some(serde_name(meta)?);
            } else if meta.path.is_ident("tag") && matches!(site, SerdeSite::Enum) {
                options.tag = Some(meta.value()?.parse::<LitStr>()?.value());
            } else if (meta.path.is_ident("skip") || meta.path.is_ident("skip_deserializing"))
                && matches!(site, SerdeSite::Field | SerdeSite::Variant)
            {
                options.skip = true;
            } else if meta.path.is_ident("default") && matches!(site, SerdeSite::Field) {
                if meta.input.peek(syn::Token![=]) {
                    return Err(meta.error("value-bearing serde default is unsupported"));
                }
            } else if meta.path.is_ident("deserialize_with") && matches!(site, SerdeSite::Field) {
                let _ = meta.value()?.parse::<LitStr>()?;
            } else if meta.path.is_ident("deny_unknown_fields")
                && matches!(site, SerdeSite::Struct | SerdeSite::Enum)
            {
                if meta.input.peek(syn::Token![=]) {
                    return Err(
                        meta.error("value-bearing serde deny_unknown_fields is unsupported")
                    );
                }
            } else if meta.path.is_ident("content")
                || meta.path.is_ident("untagged")
                || meta.path.is_ident("flatten")
                || meta.path.is_ident("alias")
            {
                return Err(meta.error("unsupported serde representation in configuration model"));
            } else {
                return Err(meta.error("unsupported serde option in configuration model"));
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    }
    Ok(options)
}

fn renamed(ident: &str, rename_all: Option<&str>) -> Result<String, String> {
    match rename_all {
        None => Ok(ident.to_owned()),
        Some("lowercase") => Ok(ident.to_ascii_lowercase()),
        Some("kebab-case") => {
            let mut name = String::new();
            for (index, character) in ident.chars().enumerate() {
                if character.is_ascii_uppercase() && index != 0 {
                    name.push('-');
                }
                name.push(character.to_ascii_lowercase());
            }
            Ok(name)
        }
        Some(value) => Err(format!(
            "unsupported reachable serde rename_all rule {value:?}"
        )),
    }
}

#[derive(Clone, Copy)]
enum Definition<'a> {
    Struct(&'a ItemStruct),
    Enum(&'a ItemEnum),
}

type SourceContract = (BTreeSet<String>, BTreeMap<String, BTreeSet<String>>);

struct Model<'a> {
    definitions: BTreeMap<String, Definition<'a>>,
    fields: BTreeSet<String>,
    domains: BTreeMap<String, BTreeSet<String>>,
    stack: Vec<String>,
}

impl<'a> Model<'a> {
    fn from_files(files: &'a [syn::File]) -> Result<Self, String> {
        let mut model = Self {
            definitions: BTreeMap::new(),
            fields: BTreeSet::new(),
            domains: BTreeMap::new(),
            stack: Vec::new(),
        };
        for file in files {
            for item in &file.items {
                let entry = match item {
                    Item::Struct(item) => Some((item.ident.to_string(), Definition::Struct(item))),
                    Item::Enum(item) => Some((item.ident.to_string(), Definition::Enum(item))),
                    _ => None,
                };
                if let Some((name, definition)) = entry {
                    if model.definitions.insert(name.clone(), definition).is_some() {
                        return Err(format!("duplicate model type definition {name}"));
                    }
                }
            }
        }
        Ok(model)
    }

    fn visit_type(&mut self, ty: &Type, path: &str) -> Result<(), String> {
        let Type::Path(type_path) = ty else {
            return Err(format!("unsupported reachable non-path type at {path}"));
        };
        if type_path.qself.is_some() {
            return Err(format!("qualified reachable type at {path} is unsupported"));
        }
        let segment = type_path
            .path
            .segments
            .last()
            .ok_or_else(|| format!("empty reachable type path at {path}"))?;
        let name = segment.ident.to_string();
        match name.as_str() {
            "Option" | "Box" => self.visit_type(single_type_argument(segment, path)?, path),
            "Vec" => self.visit_type(single_type_argument(segment, path)?, &format!("{path}[]")),
            "BTreeMap" | "HashMap" => {
                let arguments = type_arguments(segment, path)?;
                if arguments.len() != 2 {
                    return Err(format!("{name} at {path} must have two type arguments"));
                }
                self.visit_map_key(arguments[0], path)?;
                self.visit_type(arguments[1], path)
            }
            // Only scalar leaves outside the model tree belong here. HttpsUrl is validated by
            // the domain module; every other project type must have a scanned definition.
            "String" | "str" | "bool" | "usize" | "u8" | "u16" | "u32" | "u64" | "i8" | "i16"
            | "i32" | "i64" | "f32" | "f64" | "HttpsUrl" => Ok(()),
            _ => {
                if !matches!(segment.arguments, PathArguments::None) {
                    return Err(format!(
                        "unsupported generic model type {name} reachable at {path}"
                    ));
                }
                self.visit_definition(&name, path)
            }
        }
    }

    fn visit_map_key(&mut self, ty: &Type, path: &str) -> Result<(), String> {
        let Type::Path(type_path) = ty else {
            return Err(format!("unsupported map key type at {path}"));
        };
        let segment = type_path
            .path
            .segments
            .last()
            .ok_or_else(|| format!("empty map key type at {path}"))?;
        let name = segment.ident.to_string();
        if matches!(name.as_str(), "String" | "str") {
            return Ok(());
        }
        let Some(definition) = self.definitions.get(&name).copied() else {
            return Err(format!(
                "missing map key type definition for {name} at {path}"
            ));
        };
        match definition {
            Definition::Enum(item)
                if item
                    .variants
                    .iter()
                    .all(|variant| matches!(variant.fields, Fields::Unit)) =>
            {
                self.visit_enum(item, path)
            }
            _ => Err(format!(
                "map key type {name} at {path} must be a scalar or unit enum"
            )),
        }
    }

    fn visit_definition(&mut self, name: &str, path: &str) -> Result<(), String> {
        if self.stack.iter().any(|active| active == name) {
            return Err(format!("recursive model type {name} reachable at {path}"));
        }
        let Some(definition) = self.definitions.get(name).copied() else {
            return Err(format!(
                "missing model type definition for {name} at {path}"
            ));
        };
        self.stack.push(name.to_owned());
        let result = match definition {
            Definition::Struct(item) => self.visit_struct(item, path),
            Definition::Enum(item) => self.visit_enum(item, path),
        };
        self.stack.pop();
        result
    }

    fn visit_struct(&mut self, item: &ItemStruct, path: &str) -> Result<(), String> {
        let container = serde_options(&item.attrs, SerdeSite::Struct)?;
        match &item.fields {
            Fields::Named(fields) => {
                for field in &fields.named {
                    let options = serde_options(&field.attrs, SerdeSite::Field)?;
                    if options.skip {
                        continue;
                    }
                    let ident = field
                        .ident
                        .as_ref()
                        .ok_or_else(|| format!("unnamed field in model struct at {path}"))?
                        .to_string();
                    let name = options
                        .rename
                        .unwrap_or(renamed(&ident, container.rename_all.as_deref())?);
                    let child = if path.is_empty() {
                        name
                    } else {
                        format!("{path}.{name}")
                    };
                    self.fields.insert(child.clone());
                    self.visit_type(&field.ty, &child)?;
                }
                Ok(())
            }
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                self.visit_type(&fields.unnamed[0].ty, path)
            }
            Fields::Unit => Ok(()),
            Fields::Unnamed(_) => Err(format!(
                "tuple struct {} reachable at {path} is unsupported",
                item.ident
            )),
        }
    }

    fn visit_enum(&mut self, item: &ItemEnum, path: &str) -> Result<(), String> {
        let options = serde_options(&item.attrs, SerdeSite::Enum)?;
        let mut values = BTreeSet::new();
        for variant in &item.variants {
            let variant_options = serde_options(&variant.attrs, SerdeSite::Variant)?;
            if variant_options.skip {
                continue;
            }
            let ident = variant.ident.to_string();
            values.insert(
                variant_options
                    .rename
                    .unwrap_or(renamed(&ident, options.rename_all.as_deref())?),
            );
        }
        if let Some(tag) = options.tag {
            let tag_path = format!("{path}.{tag}");
            self.fields.insert(tag_path.clone());
            self.domains.entry(tag_path).or_default().extend(values);
            for variant in &item.variants {
                if serde_options(&variant.attrs, SerdeSite::Variant)?.skip {
                    continue;
                }
                match &variant.fields {
                    Fields::Named(fields) => {
                        for field in &fields.named {
                            let field_options = serde_options(&field.attrs, SerdeSite::Field)?;
                            if field_options.skip {
                                continue;
                            }
                            let ident = field.ident.as_ref().expect("named field").to_string();
                            let name = field_options.rename.unwrap_or(ident);
                            let child = format!("{path}.{name}");
                            self.fields.insert(child.clone());
                            self.visit_type(&field.ty, &child)?;
                        }
                    }
                    Fields::Unit => {}
                    Fields::Unnamed(_) => {
                        return Err(format!(
                            "tagged tuple variant {}::{} at {path} is unsupported",
                            item.ident, variant.ident
                        ));
                    }
                }
            }
            Ok(())
        } else if item
            .variants
            .iter()
            .all(|variant| matches!(variant.fields, Fields::Unit))
        {
            self.domains
                .entry(path.strip_suffix("[]").unwrap_or(path).to_owned())
                .or_default()
                .extend(values);
            Ok(())
        } else {
            Err(format!(
                "reachable data enum {} at {path} must use an internal serde tag",
                item.ident
            ))
        }
    }
}

fn type_arguments<'a>(segment: &'a syn::PathSegment, path: &str) -> Result<Vec<&'a Type>, String> {
    let PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) =
        &segment.arguments
    else {
        return Err(format!(
            "{} at {path} requires type arguments",
            segment.ident
        ));
    };
    args.iter()
        .map(|argument| match argument {
            GenericArgument::Type(ty) => Ok(ty),
            _ => Err(format!(
                "non-type generic argument on {} at {path} is unsupported",
                segment.ident
            )),
        })
        .collect()
}

fn single_type_argument<'a>(segment: &'a syn::PathSegment, path: &str) -> Result<&'a Type, String> {
    let arguments = type_arguments(segment, path)?;
    if arguments.len() == 1 {
        Ok(arguments[0])
    } else {
        Err(format!(
            "{} at {path} must have one type argument",
            segment.ident
        ))
    }
}

fn rust_files_below(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in
        fs::read_dir(directory).map_err(|error| format!("read {}: {error}", directory.display()))?
    {
        let path = entry
            .map_err(|error| format!("read {} entry: {error}", directory.display()))?
            .path();
        if path.is_dir() {
            rust_files_below(&path, paths)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
    Ok(())
}

fn model_files(root: &Path) -> Result<Vec<syn::File>, String> {
    let mut paths = vec![root.join("model.rs")];
    rust_files_below(&root.join("model"), &mut paths)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("read {}: {error}", path.display()))?;
            syn::parse_file(&source).map_err(|error| format!("parse {}: {error}", path.display()))
        })
        .collect()
}

fn contract_from_files(files: &[syn::File]) -> Result<SourceContract, String> {
    let mut model = Model::from_files(files)?;
    model
        .visit_definition("Config", "")
        .map(|()| (model.fields, model.domains))
}

fn source_contract() -> SourceContract {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/config");
    let files =
        model_files(&root).expect("configuration model files must be discoverable and valid Rust");
    contract_from_files(&files)
        .expect("public configuration model must have a supported reachable shape")
}

fn reference_field_rows(root_fields: &BTreeSet<String>) -> BTreeSet<String> {
    CONFIGURATION_REFERENCE
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|line| line.split_once("` |"))
        .map(|(field, _)| field)
        .filter(|field| {
            root_fields
                .iter()
                .any(|root| *field == root || field.starts_with(&format!("{root}.")))
        })
        .map(str::to_owned)
        .collect()
}

fn reference_row_in<'a>(document: &'a str, path: &str) -> Vec<&'a str> {
    let prefix = format!("| `{path}` |");
    document
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("missing field table row for {path}"))
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

fn reference_row(path: &str) -> Vec<&str> {
    reference_row_in(CONFIGURATION_REFERENCE, path)
}

fn code_values(cell: &str) -> BTreeSet<String> {
    cell.split('`')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, value)| value.to_owned())
        .collect()
}

fn verify_documented_domains(
    reference: &str,
    domains: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    for (path, expected) in domains {
        let row = reference_row_in(reference, path);
        if row.len() != 3 {
            return Err(format!("malformed field table row for {path}"));
        }
        let documented = code_values(row[1]);
        if documented != *expected {
            return Err(format!(
                "{path} accepted values differ: source {expected:?}, documentation {documented:?}"
            ));
        }
    }
    Ok(())
}

#[test]
fn configuration_reference_field_inventory_matches_deserialize_model() {
    let (implementation_fields, _) = source_contract();
    let root_fields = implementation_fields
        .iter()
        .filter(|path| !path.contains('.'))
        .cloned()
        .collect();
    let documented_fields = reference_field_rows(&root_fields);
    assert_eq!(documented_fields, implementation_fields);
}

#[test]
fn configuration_reference_covers_source_derived_enum_domains_by_section() {
    let (_, domains) = source_contract();
    verify_documented_domains(CONFIGURATION_REFERENCE, &domains)
        .expect("each source-derived domain must exactly match its accepted-value table cell");
}

#[test]
fn source_contract_discovers_nested_model_files_and_fields() {
    let directory = tempfile::tempdir().expect("temporary model directory");
    let root = directory.path();
    fs::create_dir_all(root.join("model/child")).expect("create nested model directory");
    fs::write(
        root.join("model.rs"),
        "struct Config { child: Child }\nmod child;\n",
    )
    .expect("write model root");
    fs::write(
        root.join("model/child/mod.rs"),
        "struct Child { added: bool }\n",
    )
    .expect("write nested model module");

    let files = model_files(root).expect("discover nested model files");
    let (fields, _) = contract_from_files(&files).expect("traverse nested model field");
    assert_eq!(
        fields,
        BTreeSet::from(["child".into(), "child.added".into()])
    );
}

#[test]
fn exact_domain_comparison_rejects_missing_and_extra_values() {
    let domains = BTreeMap::from([(
        "mode".to_owned(),
        BTreeSet::from(["managed".to_owned(), "preserve".to_owned()]),
    )]);
    for reference in [
        "| `mode` | Required: `managed` | Notes |",
        "| `mode` | Required: `managed`, `preserve`, or `other` | Notes |",
    ] {
        assert!(verify_documented_domains(reference, &domains).is_err());
    }
}

#[test]
fn source_contract_rejects_unknown_external_types() {
    let files = [syn::parse_file("struct Config { value: ImportedType }").expect("valid Rust")];
    let error = contract_from_files(&files).expect_err("unknown type must not be opaque");
    assert!(error.contains("missing model type definition for ImportedType"));
}

#[test]
fn source_contract_rejects_adjacent_enum_tagging() {
    let files = [syn::parse_file(
        "struct Config { source: Source }\n\
         #[serde(tag = \"kind\", content = \"value\")]\n\
         enum Source { Local { path: String } }",
    )
    .expect("valid Rust")];
    let error = contract_from_files(&files).expect_err("adjacent tagging must be unsupported");
    assert!(error.contains("unsupported serde representation"));
}

#[test]
fn configuration_reference_rows_preserve_contract_semantics() {
    for path in [
        "system.apt.sources.mode",
        "system.apt.sources.components",
        "packages.cargo",
        "packages.npm",
        "packages.apt.repositories[].suite",
        "packages.apt.repositories[].path",
        "system.apt.unattended_upgrades",
        "system.ubuntu.snap",
        "integrations.docker.logging",
        "integrations.docker.logging.driver",
        "integrations.docker.logging.max_size",
        "desktop.theme",
        "desktop.gnome.extensions",
        "updates.apt",
        "updates.flatpak",
        "updates.tools.rust",
        "updates.packages.binaries",
    ] {
        let row = reference_row(path);
        assert_eq!(row.len(), 3, "malformed field table row for {path}");
        let accepted = row[1].to_ascii_lowercase();
        assert!(
            accepted.contains("optional") || accepted.contains("required"),
            "{path} must state required/optional status"
        );
        assert!(
            !row[2].is_empty(),
            "{path} must document contract semantics"
        );
    }

    let apt = reference_row("updates.apt");
    for behavior in [
        "apt-get upgrade",
        "apt-get full-upgrade",
        "apt-get autoremove --purge",
        "permanently removes",
    ] {
        assert!(apt[2].contains(behavior), "updates.apt omits {behavior:?}");
    }
    let unattended_upgrades = reference_row("system.apt.unattended_upgrades");
    for behavior in [
        "disables/stops `unattended-upgrades.service`",
        "purges the `unattended-upgrades` package when installed",
    ] {
        assert!(
            unattended_upgrades[2].contains(behavior),
            "system.apt.unattended_upgrades omits {behavior:?}"
        );
    }
    let snap = reference_row("system.ubuntu.snap");
    for behavior in [
        "irreversibly removes every installed Snap with `snap remove --purge`",
        "disables/stops Snap units",
        "purges `snapd`",
        "recursively deletes `$HOME/snap`, `/snap`, `/var/snap`, and `/var/lib/snapd`",
        "publishes Cozydot's no-Snap APT pin",
    ] {
        assert!(
            snap[2].contains(behavior),
            "system.ubuntu.snap omits {behavior:?}"
        );
    }
    let docker_logging = reference_row("integrations.docker.logging");
    for behavior in [
        "atomically merges only the owned `log-driver` and optional `log-opts.max-size` keys",
        "preserving unrelated valid JSON",
        "does not restart/reload Docker or active containers",
        "operator must restart/reload Docker as appropriate to activate the setting",
    ] {
        assert!(
            docker_logging[2].contains(behavior),
            "integrations.docker.logging omits {behavior:?}"
        );
    }
    assert!(
        reference_row("desktop.theme")[2].contains("GNOME/Cinnamon only"),
        "desktop.theme must retain its applicability"
    );
    for path in [
        "packages.cargo",
        "packages.npm",
        "updates.flatpak",
        "updates.tools.rust",
        "updates.packages.binaries",
    ] {
        assert!(
            reference_row(path)[2].contains("Requires"),
            "{path} must retain its prerequisite"
        );
    }
    assert!(
        Config::parse("version: 1.0.0").is_ok(),
        "omitting every optional top-level field must remain valid"
    );
}
