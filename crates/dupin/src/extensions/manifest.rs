//! What an extension folder declares, read without running guest code: the `dupin` key of its
//! package.json.

use std::path::{Component, Path};

use dupin_broker::{RowField, RowKind};
use dupin_js_runtime::{DEFAULT_ENTRY, Diagnostic, is_module};
use serde_json::{Map, Value};

use super::{FileStamp, read_capped};

pub const PACKAGE_JSON: &str = "package.json";
/// The manifest of the Wasm extensions Dupin no longer runs, read only to say so.
pub const WASM_MANIFEST: &str = "extension.toml";
/// package.json also carries the author's tooling: scripts, dependencies and the like.
const MAX_PACKAGE_JSON_BYTES: usize = 64 * 1024;
const DUPIN_KEYS: &str = "`title`, `keyword`, and optionally `icon`, `main`, `defaultSearch`, `emptyQuery`, `match`, `preferences`, `capabilities` and `provides`";
/// Every key the `dupin` object may hold, as sdk/dupin.d.ts declares them.
const DUPIN_KEY_NAMES: [&str; 10] = [
    "title",
    "keyword",
    "icon",
    "main",
    "defaultSearch",
    "emptyQuery",
    "match",
    "preferences",
    "capabilities",
    "provides",
];
const PROVIDES_HINT: &str = "`provides` takes `filters`, as `{ \"<name>\": { \"stage\": \"matcher\" | \"sorter\" } }`, and `kinds`, as `{ \"<kind>\": { \"<action>\": { \"field\": \"<field>\" } } }`; see docs/features/advanced.md.";
/// Filters, and kind actions for each kind, one package may provide.
const MAX_CONTRIBUTIONS: usize = 8;
const DEFAULT_SEARCH_HINT: &str =
    "Set `\"defaultSearch\": true` to list the package's `items()` in the main search, or leave it out.";
const EMPTY_QUERY_HINT: &str =
    "Set `\"emptyQuery\": true` to have `search` answer before anything is typed, or leave it out.";
const MATCH_HINT: &str = "Set `\"match\": \"words\"` to find `items()` by every word typed, in the catalogue's order, or leave it out to rank them as apps are.";

pub(super) struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub keyword: String,
    /// What its keyword chip, Settings and its rows without an icon of their own show.
    pub icon: Option<PackageIcon>,
    /// Relative to the folder: a JavaScript or TypeScript module.
    pub entry: String,
    /// The package's `items()` catalogue also answers the default search.
    pub default_search: bool,
    /// `search` answers the keyword before anything is typed.
    pub empty_query: bool,
    /// How the `items()` catalogue is matched.
    pub matching: dupin_core::catalogue::Matching,
    pub preferences: super::preferences::Preferences,
    /// The most the package may do; the user grants each scope.
    pub capabilities: dupin_broker::Capabilities,
    pub provides: Provides,
}

/// A package's own icon, as its manifest names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageIcon {
    /// An SF Symbol, by its name.
    Symbol(String),
    /// An image file, relative to the folder, which may not have been found yet.
    File(String),
}

const ICON_HINT: &str =
    "Name an image in the package, such as `assets/icon.png`, or an SF Symbol, such as `sf:puzzlepiece`.";

fn parse_icon(icon: &str) -> Result<PackageIcon, Diagnostic> {
    dupin_js_runtime::row::check_icon(icon).map_err(|message| {
        Diagnostic::new("invalid-manifest", message.replace("`icon` must", "must"))
            .file(PACKAGE_JSON)
            .field("dupin.icon")
            .hint(ICON_HINT)
    })?;
    Ok(match icon.strip_prefix(dupin_core::item::SYMBOL_PREFIX) {
        Some(name) => PackageIcon::Symbol(name.to_owned()),
        None => PackageIcon::File(icon.to_owned()),
    })
}

/// What a package offers `[advanced]` to compose onto Dupin's own sources and kinds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provides {
    /// Its `filters` exports by name, with the stage each is.
    pub filters: std::collections::BTreeMap<String, dupin_js_runtime::FilterStage>,
    /// Its `actions` exports that act on rows of a kind, by kind and name, with the one field
    /// each receives.
    pub kinds: std::collections::BTreeMap<(RowKind, String), RowField>,
}

/// The `provides` value of a manifest, checked against the scopes `capabilities` declares: a
/// filter sees titles, a matcher also the query, and a kind action the one field it names.
fn parse_provides(value: &Value, capabilities: &dupin_broker::Capabilities) -> Result<Provides, Diagnostic> {
    use dupin_broker::{Scope, ScopeKey};
    let problem = |field: &str, message: String| {
        Diagnostic::new("invalid-manifest", message)
            .file(PACKAGE_JSON)
            .field(format!("dupin.provides{field}"))
            .hint(PROVIDES_HINT)
    };
    let object = |value: &Value, field: &str| match value {
        Value::Object(object) => Ok(object.clone()),
        _ => Err(problem(field, "must be an object".into())),
    };
    let named = |name: &str, field: &str| {
        let valid = (1..=64).contains(&name.len())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if valid {
            Ok(())
        } else {
            Err(problem(
                field,
                "must be named by 1–64 letters, digits or `_`".into(),
            ))
        }
    };
    let declared = |kind: RowKind, field: RowField| {
        matches!(
            capabilities.get(&ScopeKey::Rows(kind)),
            Some(Scope::Rows { fields, .. }) if fields.contains(&field)
        )
    };
    let mut provides = Provides::default();
    for (key, value) in object(value, "")? {
        match key.as_str() {
            "filters" => {
                let filters = object(&value, ".filters")?;
                if filters.len() > MAX_CONTRIBUTIONS {
                    return Err(problem(
                        ".filters",
                        format!("provides more than {MAX_CONTRIBUTIONS} filters"),
                    ));
                }
                for (name, filter) in filters {
                    let field = format!(".filters.{name}");
                    named(&name, &field)?;
                    let filter = object(&filter, &field)?;
                    if let Some(key) = filter.keys().find(|key| *key != "stage") {
                        return Err(problem(
                            &format!("{field}.{key}"),
                            "is not a key Dupin reads".into(),
                        ));
                    }
                    let stage = match filter.get("stage").and_then(Value::as_str) {
                        Some("matcher") => dupin_js_runtime::FilterStage::Matcher,
                        Some("sorter") => dupin_js_runtime::FilterStage::Sorter,
                        _ => {
                            return Err(problem(
                                &format!("{field}.stage"),
                                "must be \"matcher\" or \"sorter\"".into(),
                            ));
                        }
                    };
                    if !RowKind::ALL
                        .into_iter()
                        .any(|kind| declared(kind, RowField::Title))
                    {
                        return Err(problem(
                            &field,
                            "needs `capabilities.rows` to declare the `title` of a kind of row it filters"
                                .into(),
                        ));
                    }
                    if stage == dupin_js_runtime::FilterStage::Matcher
                        && capabilities.get(&ScopeKey::Query).is_none()
                    {
                        return Err(problem(
                            &field,
                            "is a matcher, so it needs `capabilities.query`".into(),
                        ));
                    }
                    provides.filters.insert(name, stage);
                }
            }
            "kinds" => {
                for (kind_name, actions) in object(&value, ".kinds")? {
                    let field = format!(".kinds.{kind_name}");
                    let kind = RowKind::ALL
                        .into_iter()
                        .find(|kind| kind.as_str() == kind_name)
                        .ok_or_else(|| {
                            problem(
                                &field,
                                format!("`{kind_name}` isn't a kind; use text, url, file or app"),
                            )
                        })?;
                    let actions = object(&actions, &field)?;
                    if actions.len() > MAX_CONTRIBUTIONS {
                        return Err(problem(
                            &field,
                            format!("provides more than {MAX_CONTRIBUTIONS} actions"),
                        ));
                    }
                    for (name, action) in actions {
                        let field = format!("{field}.{name}");
                        named(&name, &field)?;
                        let action = object(&action, &field)?;
                        if let Some(key) = action.keys().find(|key| *key != "field") {
                            return Err(problem(
                                &format!("{field}.{key}"),
                                "is not a key Dupin reads".into(),
                            ));
                        }
                        let value = action.get("field").and_then(Value::as_str).unwrap_or_default();
                        let row_field = kind
                            .fields()
                            .iter()
                            .copied()
                            .filter(|known| *known != RowField::Title)
                            .find(|known| known.as_str() == value)
                            .ok_or_else(|| {
                                let known: Vec<_> = kind
                                    .fields()
                                    .iter()
                                    .filter(|known| **known != RowField::Title)
                                    .map(|known| known.as_str())
                                    .collect();
                                problem(
                                    &format!("{field}.field"),
                                    format!("must be one of {} for {} rows", known.join(", "), kind.as_str()),
                                )
                            })?;
                        if !declared(kind, row_field) {
                            return Err(problem(
                                &field,
                                format!(
                                    "receives the {} of {} rows, so `capabilities.rows.{}` must declare it",
                                    row_field.as_str(),
                                    kind.as_str(),
                                    kind.as_str()
                                ),
                            ));
                        }
                        provides.kinds.insert((kind, name), row_field);
                    }
                }
            }
            _ => return Err(problem(&format!(".{key}"), "is not a key Dupin reads".into())),
        }
    }
    Ok(provides)
}

/// The manifest files in one folder, so a later scan can tell whether any appeared, changed or
/// went away.
#[derive(PartialEq, Eq)]
pub(super) struct Stamps {
    package: Option<FileStamp>,
    wasm_leftover: bool,
}

impl Stamps {
    pub(super) fn read(folder: &Path) -> Result<Self, Diagnostic> {
        let stamp = |name: &str| {
            let path = folder.join(name);
            if !path.exists() {
                return Ok(None);
            }
            path.canonicalize()
                .and_then(FileStamp::read)
                .map(Some)
                .map_err(|error| {
                    Diagnostic::new("invalid-manifest", format!("Couldn't read it: {error}")).file(name)
                })
        };
        Ok(Self {
            package: stamp(PACKAGE_JSON)?,
            wasm_leftover: folder.join(WASM_MANIFEST).exists(),
        })
    }
}

/// A package.json with a `dupin` key makes an extension. A folder that holds a Wasm guest's
/// extension.toml and no such key is told that Wasm extensions are gone, not that package.json
/// is missing, since a folder of that shape once loaded. A package.json that can't be read is
/// reported before that: it may be the port in progress, and its own error locates the fix.
pub(super) fn read(stamps: &Stamps) -> Result<Manifest, Diagnostic> {
    let package = stamps
        .package
        .as_ref()
        .map(|package| parse_package(&package.path))
        .transpose()?;
    let declares_dupin = package
        .as_ref()
        .and_then(|package| package.get("dupin"))
        .is_some_and(|dupin| !dupin.is_null());
    if stamps.wasm_leftover && !declares_dupin {
        return Err(Diagnostic::new(
            "wasm-removed",
            "Wasm extensions were removed; port to JavaScript",
        )
        .file(WASM_MANIFEST)
        .hint(format!(
            "Describe the extension in package.json, with `name`, `version` and a `dupin` key holding {DUPIN_KEYS}, export `search` from main.js and delete extension.toml. `dupin ext new` writes a starting point."
        )));
    }
    let Some(package) = package else {
        return Err(Diagnostic::new("missing-manifest", "is missing")
            .file(PACKAGE_JSON)
            .hint(
                "Create package.json with `name`, `version` and a `dupin` key holding `title` and `keyword`.",
            ));
    };
    match package.get("dupin") {
        Some(dupin) => javascript(&package, dupin),
        None => Err(Diagnostic::new("missing-manifest", "has no `dupin` key")
            .file(PACKAGE_JSON)
            .field("dupin")
            .hint(format!(
                "Add a `dupin` key holding {DUPIN_KEYS} to make the folder a Dupin extension."
            ))),
    }
}

fn parse_package(path: &Path) -> Result<Map<String, Value>, Diagnostic> {
    let invalid = |message: String| Diagnostic::new("invalid-manifest", message).file(PACKAGE_JSON);
    let bytes = read_capped(path, MAX_PACKAGE_JSON_BYTES).map_err(invalid)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        let message = error.to_string();
        let message = message.split(" at line ").next().unwrap_or(&message).to_owned();
        let diagnostic = invalid(message).hint("package.json must be valid JSON, without comments.");
        match (u32::try_from(error.line()), u32::try_from(error.column())) {
            (Ok(line), Ok(column)) if line > 0 => diagnostic.at(line, column),
            _ => diagnostic,
        }
    })?;
    match value {
        Value::Object(package) => Ok(package),
        _ => Err(invalid("must be a JSON object".into())),
    }
}

fn javascript(package: &Map<String, Value>, dupin: &Value) -> Result<Manifest, Diagnostic> {
    let field = |field: &str, message: &str, hint: &str| {
        Diagnostic::new("invalid-manifest", message)
            .file(PACKAGE_JSON)
            .field(field)
            .hint(hint)
    };
    let string = |object: &Map<String, Value>, key: &str, field_name: &str, hint: &str| match object.get(key)
    {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        None => Ok(None),
        Some(_) => Err(field(field_name, "must be a string", hint)),
    };
    let required = |object: &Map<String, Value>, key: &str, field_name: &str, hint: &str| {
        string(object, key, field_name, hint)?.ok_or_else(|| field(field_name, "is missing", hint))
    };
    let flag = |object: &Map<String, Value>, key: &str, hint: &str| match object.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(field(&format!("dupin.{key}"), "must be true or false", hint)),
    };

    let Value::Object(dupin) = dupin else {
        return Err(field(
            "dupin",
            "must be an object",
            &format!("Give `dupin` {DUPIN_KEYS}."),
        ));
    };
    if let Some(key) = dupin.keys().find(|key| !DUPIN_KEY_NAMES.contains(&key.as_str())) {
        return Err(field(
            &format!("dupin.{key}"),
            "is not a key Dupin reads",
            &format!("The `dupin` key takes {DUPIN_KEYS}."),
        ));
    }
    let manifest = Manifest {
        id: required(package, "name", "name", ID_HINT)?,
        name: required(dupin, "title", "dupin.title", TITLE_HINT)?,
        version: required(package, "version", "version", VERSION_HINT)?,
        keyword: required(dupin, "keyword", "dupin.keyword", KEYWORD_HINT)?,
        icon: string(dupin, "icon", "dupin.icon", ICON_HINT)?
            .map(|icon| parse_icon(&icon))
            .transpose()?,
        entry: string(dupin, "main", "dupin.main", MAIN_HINT)?.unwrap_or_else(|| DEFAULT_ENTRY.into()),
        default_search: flag(dupin, "defaultSearch", DEFAULT_SEARCH_HINT)?,
        empty_query: flag(dupin, "emptyQuery", EMPTY_QUERY_HINT)?,
        matching: match string(dupin, "match", "dupin.match", MATCH_HINT)?.as_deref() {
            None | Some("fuzzy") => dupin_core::catalogue::Matching::Fuzzy,
            Some("words") => dupin_core::catalogue::Matching::Words,
            Some(_) => {
                return Err(field("dupin.match", "must be \"fuzzy\" or \"words\"", MATCH_HINT));
            }
        },
        preferences: match dupin.get("preferences") {
            Some(preferences) => super::preferences::Preferences::parse(preferences)?,
            None => super::preferences::Preferences::default(),
        },
        capabilities: dupin_broker::Capabilities::default(),
        provides: Provides::default(),
    };
    validate(&manifest)?;
    let entry = relative_inside(&manifest.entry)
        .filter(|entry| is_module(entry))
        .ok_or_else(|| {
            field(
                "dupin.main",
                "must be a relative path to a .js, .mjs, .ts, .mts or .tsx file inside the folder",
                MAIN_HINT,
            )
        })?;
    let capabilities = match dupin.get("capabilities") {
        Some(capabilities) => parse_capabilities(capabilities)?,
        None => dupin_broker::Capabilities::default(),
    };
    let provides = match dupin.get("provides") {
        Some(provides) => parse_provides(provides, &capabilities)?,
        None => Provides::default(),
    };
    Ok(Manifest {
        entry,
        capabilities,
        provides,
        ..manifest
    })
}

/// The capability broker's own reading of the declaration, so what Settings lists and what a
/// call is checked against can't differ. The first problem is reported; fixing it shows the next.
pub(super) fn parse_capabilities(capabilities: &Value) -> Result<dupin_broker::Capabilities, Diagnostic> {
    dupin_broker::Capabilities::parse(capabilities).map_err(|problems| {
        let problem = &problems[0];
        let diagnostic = Diagnostic::new(problem.code, problem.message.clone())
            .file(PACKAGE_JSON)
            .field(format!("dupin.{}", problem.field));
        match problem.hint {
            Some(hint) => diagnostic.hint(hint),
            None => diagnostic.hint("See the capability sections of docs/features/extensions.md."),
        }
    })
}

/// `path` spelt as the folder-relative file name the loaders compare entries with, or `None`
/// when it could lead outside the folder.
fn relative_inside(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

const ID_HINT: &str = "Use a name such as `word-count`.";
const TITLE_HINT: &str = "Use the name Settings and the keyword chip should show, such as `Word Count`.";
const VERSION_HINT: &str = "Use a version such as `0.1.0`.";
const KEYWORD_HINT: &str = "Use a short word such as `wc`.";
const MAIN_HINT: &str = "Leave `main` out to use main.js.";

fn validate(manifest: &Manifest) -> Result<(), Diagnostic> {
    let invalid = |field: &str, message: &str, hint: &str| {
        Diagnostic::new("invalid-manifest", message)
            .file(PACKAGE_JSON)
            .field(field)
            .hint(hint)
    };
    let id = &manifest.id;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(invalid(
            "name",
            "must be 1–64 lowercase letters, digits or hyphens",
            ID_HINT,
        ));
    }
    let name = &manifest.name;
    if name.trim().is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(invalid(
            "dupin.title",
            "must be 1–80 printable characters",
            TITLE_HINT,
        ));
    }
    let unspaced = |value: &str| !value.chars().any(|ch| ch.is_whitespace() || ch.is_control());
    if manifest.version.is_empty() || manifest.version.len() > 32 || !unspaced(&manifest.version) {
        return Err(invalid(
            "version",
            "must be 1–32 characters without spaces",
            VERSION_HINT,
        ));
    }
    if manifest.keyword.is_empty() || manifest.keyword.len() > 32 || !unspaced(&manifest.keyword) {
        return Err(invalid(
            "dupin.keyword",
            "must be 1–32 bytes without spaces",
            KEYWORD_HINT,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(files: &[(&str, &str)]) -> tempfile::TempDir {
        let folder = tempfile::Builder::new()
            .prefix("dupin-manifest-")
            .tempdir()
            .unwrap();
        for (name, contents) in files {
            std::fs::write(folder.path().join(name), contents).unwrap();
        }
        folder
    }

    fn read_folder(files: &[(&str, &str)]) -> Result<Manifest, Diagnostic> {
        let folder = folder(files);
        read(&Stamps::read(folder.path())?)
    }

    fn problem(files: &[(&str, &str)]) -> Diagnostic {
        match read_folder(files) {
            Ok(manifest) => panic!("expected a problem, read {}", manifest.id),
            Err(diagnostic) => diagnostic,
        }
    }

    fn package(dupin: &str) -> String {
        format!(
            r#"{{"name": "word-count", "version": "0.1.0", "scripts": {{"build": "rolldown -c"}}, "dependencies": {{"x": "1"}}, "dupin": {dupin}}}"#
        )
    }

    const WASM: &str = "id = \"word-count\"\nname = \"Word Count\"\nversion = \"0.1.0\"\nkeyword = \"wc\"\nentry = \"guest.wasm\"\n";

    #[test]
    fn package_json_reads_the_dupin_key_and_ignores_the_rest() {
        let manifest = read_folder(&[(
            PACKAGE_JSON,
            &package(r#"{"title": "Word Count", "keyword": "wc"}"#),
        )])
        .unwrap();
        assert_eq!(
            (
                manifest.id.as_str(),
                manifest.name.as_str(),
                manifest.version.as_str(),
                manifest.keyword.as_str(),
                manifest.entry.as_str()
            ),
            ("word-count", "Word Count", "0.1.0", "wc", "main.js")
        );
        let manifest = read_folder(&[(
            PACKAGE_JSON,
            &package(r#"{"title": "W", "keyword": "wc", "main": "./dist/index.mjs", "capabilities": {}}"#),
        )])
        .unwrap();
        assert_eq!(manifest.entry, "dist/index.mjs");
        let manifest = read_folder(&[(
            PACKAGE_JSON,
            &package(r#"{"title": "W", "keyword": "wc", "main": "src/main.ts"}"#),
        )])
        .unwrap();
        assert_eq!(manifest.entry, "src/main.ts");
        for main in ["dist/./index.js", ".//dist/index.js", "./dist//index.js"] {
            let manifest = read_folder(&[(
                PACKAGE_JSON,
                &package(&format!(r#"{{"title": "W", "keyword": "wc", "main": "{main}"}}"#)),
            )])
            .unwrap();
            assert_eq!(manifest.entry, "dist/index.js", "{main}");
        }
    }

    #[test]
    fn package_json_problems_name_the_field_at_fault() {
        for (dupin, code, field) in [
            (
                r#"{"title": "W", "keyword": "wc", "entry": "x.js"}"#,
                "invalid-manifest",
                "dupin.entry",
            ),
            (r#"{"keyword": "wc"}"#, "invalid-manifest", "dupin.title"),
            (
                r#"{"title": "W", "keyword": "w c"}"#,
                "invalid-manifest",
                "dupin.keyword",
            ),
            (
                r#"{"title": "W", "keyword": 1}"#,
                "invalid-manifest",
                "dupin.keyword",
            ),
            (
                r#"{"title": "W", "keyword": "wc", "main": "../x.js"}"#,
                "invalid-manifest",
                "dupin.main",
            ),
            (
                r#"{"title": "W", "keyword": "wc", "main": "guest.wasm"}"#,
                "invalid-manifest",
                "dupin.main",
            ),
            (
                r#"{"title": "W", "keyword": "wc", "main": "dupin.d.ts"}"#,
                "invalid-manifest",
                "dupin.main",
            ),
            (
                r#"{"title": "W", "keyword": "wc", "capabilities": {"http": [{"origin": "http://example.com", "methods": ["GET"]}]}}"#,
                "invalid-origin",
                "dupin.capabilities.http[0].origin",
            ),
            (
                r#"{"title": "W", "keyword": "wc", "capabilities": {"camera": true}}"#,
                "unknown-capability",
                "dupin.capabilities.camera",
            ),
            (
                r#"{"title": "W", "keyword": "wc", "capabilities": []}"#,
                "invalid-type",
                "dupin.capabilities",
            ),
            ("[]", "invalid-manifest", "dupin"),
        ] {
            let diagnostic = problem(&[(PACKAGE_JSON, &package(dupin))]);
            assert_eq!(
                (
                    diagnostic.code.as_ref(),
                    diagnostic.file.as_deref(),
                    diagnostic.field.as_deref()
                ),
                (code, Some(PACKAGE_JSON), Some(field)),
                "{dupin}: {diagnostic}"
            );
            assert!(diagnostic.hint.is_some(), "{diagnostic:?}");
        }

        let scoped = problem(&[(
            PACKAGE_JSON,
            r#"{"name": "@you/word-count", "version": "1", "dupin": {"title": "W", "keyword": "wc"}}"#,
        )]);
        assert_eq!(scoped.field.as_deref(), Some("name"));
        let unversioned = problem(&[(
            PACKAGE_JSON,
            r#"{"name": "w", "dupin": {"title": "W", "keyword": "wc"}}"#,
        )]);
        assert_eq!(
            (unversioned.field.as_deref(), unversioned.message.as_str()),
            (Some("version"), "is missing")
        );

        let syntax = problem(&[(PACKAGE_JSON, "{\n  \"name\": \"w\",\n}")]);
        assert_eq!(
            (syntax.code.as_ref(), syntax.line, syntax.file.as_deref()),
            ("invalid-manifest", Some(3), Some(PACKAGE_JSON))
        );
        assert!(!syntax.message.contains("line"), "{syntax}");

        let oversized = problem(&[(PACKAGE_JSON, &" ".repeat(MAX_PACKAGE_JSON_BYTES + 1))]);
        assert!(oversized.message.contains("larger than 64 KiB"), "{oversized}");
    }

    #[test]
    fn the_sdk_declares_every_manifest_key() {
        let declarations = include_str!("../../../../sdk/dupin.d.ts");
        let (_, body) = declarations
            .split_once("export interface Manifest {\n")
            .expect("dupin.d.ts declares the manifest");
        let (body, _) = body.split_once("\n\t}\n").unwrap();
        let declared: std::collections::BTreeSet<_> = body
            .lines()
            .filter_map(|line| line.strip_prefix("\t\t"))
            .filter_map(|line| line.split_once(['?', ':']).map(|(key, _)| key))
            .filter(|key| key.chars().all(|c| c.is_ascii_alphanumeric()))
            .collect();
        let read: std::collections::BTreeSet<_> = DUPIN_KEY_NAMES.into_iter().collect();
        assert_eq!(declared, read);
    }

    #[test]
    fn an_icon_is_an_image_in_the_folder_or_a_symbol_name() {
        let icon = |value: &str| {
            read_folder(&[(
                PACKAGE_JSON,
                &package(&format!(r#"{{"title": "W", "keyword": "wc", "icon": {value}}}"#)),
            )])
            .map(|manifest| manifest.icon)
        };
        assert_eq!(
            icon(r#""sf:textformat.123""#).unwrap(),
            Some(PackageIcon::Symbol("textformat.123".into()))
        );
        assert_eq!(
            icon(r#""assets/icon.png""#).unwrap(),
            Some(PackageIcon::File("assets/icon.png".into()))
        );
        for (value, message) in [
            ("1", "must be a string"),
            (r#""sf:""#, "must name an SF Symbol"),
            (r#""sf:moon fill""#, "must name an SF Symbol"),
            (r#""sf:../moon""#, "must name an SF Symbol"),
            (r#""sf:moon/fill""#, "must name an SF Symbol"),
            (r#""../icon.png""#, "must be a path inside the package"),
            (r#""/tmp/icon.png""#, "must be a path inside the package"),
            (r#""assets\\icon.png""#, "must be a path inside the package"),
            (r#""icon.gif""#, "must be a path inside the package"),
            (r#""""#, "must be a path inside the package"),
        ] {
            let diagnostic = icon(value).unwrap_err();
            assert_eq!(
                (diagnostic.code.as_ref(), diagnostic.field.as_deref()),
                ("invalid-manifest", Some("dupin.icon")),
                "{value}"
            );
            assert!(diagnostic.message.starts_with(message), "{value}: {diagnostic}");
            assert!(diagnostic.hint.is_some(), "{value}");
        }
    }

    #[test]
    fn a_leftover_wasm_manifest_says_wasm_was_removed() {
        for package in [
            None,
            Some(r#"{"name": "tooling"}"#),
            Some(r#"{"name": "tooling", "dupin": null}"#),
        ] {
            let mut files = vec![(WASM_MANIFEST, WASM)];
            files.extend(package.map(|package| (PACKAGE_JSON, package)));
            let removed = problem(&files);
            assert_eq!(
                (removed.code.as_ref(), removed.file.as_deref()),
                ("wasm-removed", Some(WASM_MANIFEST)),
                "{package:.40?}"
            );
            assert_eq!(
                removed.to_string(),
                "extension.toml: Wasm extensions were removed; port to JavaScript"
            );
            assert!(removed.hint.as_deref().unwrap().contains("package.json"));
        }

        let ported = read_folder(&[
            (PACKAGE_JSON, &package(r#"{"title": "W", "keyword": "wc"}"#)),
            (WASM_MANIFEST, WASM),
        ])
        .unwrap();
        assert_eq!(ported.entry, "main.js");
    }

    #[test]
    fn a_broken_package_json_beside_a_leftover_wasm_manifest_reports_itself() {
        let oversized = format!(
            r#"{{"name": "w", "dupin": {{}}, "x": "{}"}}"#,
            " ".repeat(MAX_PACKAGE_JSON_BYTES)
        );
        for (package, message) in [
            (
                r#"{"name": "w", "dupin": {"title": "W", "keyword": "wc"},}"#,
                "trailing comma",
            ),
            ("[]", "must be a JSON object"),
            (oversized.as_str(), "larger than 64 KiB"),
        ] {
            let broken = problem(&[(WASM_MANIFEST, WASM), (PACKAGE_JSON, package)]);
            assert_eq!(
                (broken.code.as_ref(), broken.file.as_deref()),
                ("invalid-manifest", Some(PACKAGE_JSON)),
                "{package:.40?}"
            );
            assert!(broken.message.contains(message), "{broken}");
        }
    }

    #[test]
    fn a_folder_without_a_dupin_package_is_not_an_extension() {
        let plain = problem(&[(PACKAGE_JSON, r#"{"name": "plain", "version": "1.0.0"}"#)]);
        assert_eq!(
            (plain.code.as_ref(), plain.file.as_deref(), plain.field.as_deref()),
            ("missing-manifest", Some(PACKAGE_JSON), Some("dupin"))
        );
        let empty = problem(&[]);
        assert_eq!(
            (empty.code.as_ref(), empty.file.as_deref()),
            ("missing-manifest", Some(PACKAGE_JSON))
        );
    }

    fn provided(capabilities: Value, provides: Value) -> Result<Provides, Diagnostic> {
        let package: Map<String, Value> = serde_json::from_value(serde_json::json!({
            "name": "kana",
            "version": "0.1.0",
        }))
        .unwrap();
        let dupin = serde_json::json!({
            "title": "Kana",
            "keyword": "kana",
            "capabilities": capabilities,
            "provides": provides,
        });
        javascript(&package, &dupin).map(|manifest| manifest.provides)
    }

    #[test]
    fn provides_names_filters_and_kind_actions_the_declared_scopes_cover() {
        use dupin_js_runtime::FilterStage;
        let provides = provided(
            serde_json::json!({ "rows": { "app": ["title"], "file": ["path"] }, "query": ["read"] }),
            serde_json::json!({
                "filters": { "romaji": { "stage": "matcher" }, "pinned": { "stage": "sorter" } },
                "kinds": { "file": { "upload": { "field": "path" } } }
            }),
        )
        .unwrap();
        assert_eq!(
            provides.filters.into_iter().collect::<Vec<_>>(),
            [
                ("pinned".to_owned(), FilterStage::Sorter),
                ("romaji".to_owned(), FilterStage::Matcher)
            ]
        );
        assert_eq!(
            provides.kinds.into_iter().collect::<Vec<_>>(),
            [((RowKind::File, "upload".to_owned()), RowField::Path)]
        );
        let titles = serde_json::json!({ "rows": { "app": ["title"] } });
        for (capabilities, provides, field) in [
            (
                titles.clone(),
                serde_json::json!({ "filters": { "romaji": { "stage": "matcher" } } }),
                "dupin.provides.filters.romaji",
            ),
            (
                serde_json::json!({ "query": ["read"] }),
                serde_json::json!({ "filters": { "pinned": { "stage": "sorter" } } }),
                "dupin.provides.filters.pinned",
            ),
            (
                titles.clone(),
                serde_json::json!({ "filters": { "pinned": { "stage": "converter" } } }),
                "dupin.provides.filters.pinned.stage",
            ),
            (
                titles.clone(),
                serde_json::json!({ "filters": { "bad-name": { "stage": "sorter" } } }),
                "dupin.provides.filters.bad-name",
            ),
            (
                titles.clone(),
                serde_json::json!({ "kinds": { "app": { "open": { "field": "path" } } } }),
                "dupin.provides.kinds.app.open",
            ),
            (
                titles.clone(),
                serde_json::json!({ "kinds": { "app": { "open": { "field": "title" } } } }),
                "dupin.provides.kinds.app.open.field",
            ),
            (
                titles.clone(),
                serde_json::json!({ "kinds": { "folder": {} } }),
                "dupin.provides.kinds.folder",
            ),
            (
                titles.clone(),
                serde_json::json!({ "sources": {} }),
                "dupin.provides.sources",
            ),
            (
                titles.clone(),
                serde_json::json!({ "filters": { "pinned": { "stage": "sorter", "fields": ["path"] } } }),
                "dupin.provides.filters.pinned.fields",
            ),
        ] {
            let problem = provided(capabilities, provides.clone()).unwrap_err();
            assert_eq!(problem.field.as_deref(), Some(field), "{provides}: {problem:?}");
        }
        let many: Map<String, Value> = (0..=MAX_CONTRIBUTIONS)
            .map(|index| (format!("f{index}"), serde_json::json!({ "stage": "sorter" })))
            .collect();
        assert!(provided(titles, serde_json::json!({ "filters": many })).is_err());
    }
}
