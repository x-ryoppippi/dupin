//! What an extension folder declares, read without running guest code: the `dupin` key of its
//! package.json.

use std::path::{Component, Path};

use dupin_js_runtime::{DEFAULT_ENTRY, Diagnostic, is_module};
use serde_json::{Map, Value};

use super::{FileStamp, read_capped};

pub const PACKAGE_JSON: &str = "package.json";
/// The manifest of the Wasm extensions Dupin no longer runs, read only to say so.
pub const WASM_MANIFEST: &str = "extension.toml";
/// package.json also carries the author's tooling: scripts, dependencies and the like.
const MAX_PACKAGE_JSON_BYTES: usize = 64 * 1024;
const DUPIN_KEYS: &str = "`title`, `keyword`, and optionally `main` and `capabilities`";

pub(super) struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub keyword: String,
    /// Relative to the folder: a JavaScript or TypeScript module.
    pub entry: String,
    /// The most the package may do; the user grants each scope.
    pub capabilities: dupin_broker::Capabilities,
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

    let Value::Object(dupin) = dupin else {
        return Err(field(
            "dupin",
            "must be an object",
            &format!("Give `dupin` {DUPIN_KEYS}."),
        ));
    };
    if let Some(key) = dupin
        .keys()
        .find(|key| !matches!(key.as_str(), "title" | "keyword" | "main" | "capabilities"))
    {
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
        entry: string(dupin, "main", "dupin.main", MAIN_HINT)?.unwrap_or_else(|| DEFAULT_ENTRY.into()),
        capabilities: dupin_broker::Capabilities::default(),
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
    Ok(Manifest {
        entry,
        capabilities,
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
}
