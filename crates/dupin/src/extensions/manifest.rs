//! What an extension folder declares, read without running guest code: the `dupin` key of
//! package.json for a JavaScript package, or extension.toml for a Wasm module.

use std::path::{Component, Path};

use dupin_broker::Capabilities;
use dupin_js_runtime::{DEFAULT_ENTRY, Diagnostic};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{FileStamp, read_capped};

pub const PACKAGE_JSON: &str = "package.json";
pub const WASM_MANIFEST: &str = "extension.toml";
/// package.json also carries the author's tooling: scripts, dependencies and the like.
const MAX_PACKAGE_JSON_BYTES: usize = 64 * 1024;
const MAX_WASM_MANIFEST_BYTES: usize = 16 * 1024;
const DUPIN_KEYS: &str = "`title`, `keyword`, and optionally `main` and `capabilities`";

pub(super) struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub keyword: String,
    /// Relative to the folder: a .js or .mjs module, or a .wasm module.
    pub entry: String,
    /// The most the package may ask for; a Wasm module declares nothing.
    pub capabilities: Capabilities,
    /// The manifest file's bytes as read, so a revision covers what it declares.
    pub digest: blake3::Hash,
}

/// The manifest files in one folder, so a later scan can tell whether any appeared, changed or
/// went away.
#[derive(PartialEq, Eq)]
pub(super) struct Stamps {
    package: Option<FileStamp>,
    wasm: Option<FileStamp>,
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
            wasm: stamp(WASM_MANIFEST)?,
        })
    }
}

/// A package.json with a `dupin` key makes a JavaScript extension; otherwise extension.toml
/// makes a Wasm one. Beside extension.toml, package.json may belong only to the author's
/// tooling, so there it counts only when it parses and holds a non-null `dupin` key.
pub(super) fn read(stamps: &Stamps) -> Result<Manifest, Diagnostic> {
    match stamps {
        Stamps {
            package,
            wasm: Some(wasm),
        } => {
            let declares_dupin = package.as_ref().is_some_and(|package| {
                parse_package(&package.path)
                    .is_ok_and(|(package, _)| package.get("dupin").is_some_and(|dupin| !dupin.is_null()))
            });
            if declares_dupin {
                return Err(Diagnostic::new(
                    "ambiguous-manifest",
                    "sits beside a package.json with a `dupin` key",
                )
                .file(WASM_MANIFEST)
                .hint("Delete extension.toml: package.json's `dupin` key already describes this JavaScript extension."));
            }
            wasm_manifest(&wasm.path)
        }
        Stamps {
            package: Some(package),
            wasm: None,
        } => {
            let (package, digest) = parse_package(&package.path)?;
            match package.get("dupin") {
                Some(dupin) => javascript(&package, dupin, digest),
                None => Err(Diagnostic::new("missing-manifest", "has no `dupin` key")
                    .file(PACKAGE_JSON)
                    .field("dupin")
                    .hint(format!(
                        "Add a `dupin` key holding {DUPIN_KEYS} to make the folder a Dupin extension."
                    ))),
            }
        }
        Stamps {
            package: None,
            wasm: None,
        } => Err(Diagnostic::new("missing-manifest", "is missing")
            .file(PACKAGE_JSON)
            .hint(
                "Create package.json with `name`, `version` and a `dupin` key holding `title` and `keyword`.",
            )),
    }
}

fn parse_package(path: &Path) -> Result<(Map<String, Value>, blake3::Hash), Diagnostic> {
    let invalid = |message: String| Diagnostic::new("invalid-manifest", message).file(PACKAGE_JSON);
    let bytes = read_capped(path, MAX_PACKAGE_JSON_BYTES).map_err(invalid)?;
    let digest = blake3::hash(&bytes);
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
        Value::Object(package) => Ok((package, digest)),
        _ => Err(invalid("must be a JSON object".into())),
    }
}

fn javascript(
    package: &Map<String, Value>,
    dupin: &Value,
    digest: blake3::Hash,
) -> Result<Manifest, Diagnostic> {
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
        capabilities: Capabilities::default(),
        digest,
    };
    let fields = Fields {
        file: PACKAGE_JSON,
        id: "name",
        name: "dupin.title",
        version: "version",
        keyword: "dupin.keyword",
    };
    fields.validate(&manifest)?;
    let entry = relative_inside(&manifest.entry)
        .filter(|entry| {
            Path::new(entry)
                .extension()
                .is_some_and(|extension| extension == "js" || extension == "mjs")
        })
        .ok_or_else(|| {
            field(
                "dupin.main",
                "must be a relative path to a .js or .mjs file inside the folder",
                MAIN_HINT,
            )
        })?;
    let capabilities = match dupin.get("capabilities") {
        Some(capabilities) => declared(capabilities)?,
        None => Capabilities::default(),
    };
    Ok(Manifest {
        entry,
        capabilities,
        ..manifest
    })
}

/// The broker's own reading of `capabilities`, so what Settings lists and the broker grants
/// is what the package was loaded with.
fn declared(capabilities: &Value) -> Result<Capabilities, Diagnostic> {
    Capabilities::parse(capabilities).map_err(|problems| {
        let problem = &problems[0];
        let diagnostic = Diagnostic::new(problem.code, problem.message.clone())
            .file(PACKAGE_JSON)
            .field(format!("dupin.{}", problem.field));
        diagnostic.hint(problem.hint.unwrap_or(CAPABILITIES_HINT))
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WasmManifest {
    id: String,
    name: String,
    version: String,
    keyword: String,
    entry: Option<String>,
}

fn wasm_manifest(path: &Path) -> Result<Manifest, Diagnostic> {
    let bytes = read_capped(path, MAX_WASM_MANIFEST_BYTES)
        .map_err(|error| Diagnostic::new("invalid-manifest", error).file(WASM_MANIFEST))?;
    let source = std::str::from_utf8(&bytes).map_err(|_| {
        Diagnostic::new("invalid-manifest", "is not UTF-8")
            .file(WASM_MANIFEST)
            .hint("Save extension.toml as UTF-8.")
    })?;
    let manifest: WasmManifest = toml::from_str(source).map_err(|error| toml_problem(source, &error))?;
    let not_wasm = || {
        Diagnostic::new(
            "invalid-manifest",
            "must be a relative path to a .wasm file inside the folder",
        )
        .file(WASM_MANIFEST)
        .field("entry")
        .hint("extension.toml describes Wasm modules only; describe a JavaScript extension with a `dupin` key in package.json.")
    };
    let entry = manifest.entry.ok_or_else(not_wasm)?;
    let manifest = Manifest {
        id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        keyword: manifest.keyword,
        entry,
        capabilities: Capabilities::default(),
        digest: blake3::hash(&bytes),
    };
    let fields = Fields {
        file: WASM_MANIFEST,
        id: "id",
        name: "name",
        version: "version",
        keyword: "keyword",
    };
    fields.validate(&manifest)?;
    let entry = relative_inside(&manifest.entry)
        .filter(|entry| super::is_wasm(Path::new(entry)))
        .ok_or_else(not_wasm)?;
    Ok(Manifest { entry, ..manifest })
}

fn toml_problem(source: &str, error: &toml::de::Error) -> Diagnostic {
    let mut diagnostic = Diagnostic::new("invalid-manifest", error.message().trim_end())
        .file(WASM_MANIFEST)
        .hint("extension.toml takes `id`, `name`, `version`, `keyword` and `entry`, all strings.");
    if let Some(span) = error.span() {
        let before = &source[..span.start.min(source.len())];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().unwrap_or_default().chars().count() + 1;
        diagnostic = diagnostic.at(line as u32, column as u32);
    }
    diagnostic
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
const CAPABILITIES_HINT: &str =
    "docs/features/extensions.md, \"Asking for more\", shows what each capability takes.";

/// Where each shared value lives in one manifest format, for its diagnostics.
struct Fields {
    file: &'static str,
    id: &'static str,
    name: &'static str,
    version: &'static str,
    keyword: &'static str,
}

impl Fields {
    fn validate(&self, manifest: &Manifest) -> Result<(), Diagnostic> {
        let invalid = |field: &str, message: &str, hint: &str| {
            Diagnostic::new("invalid-manifest", message)
                .file(self.file)
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
                self.id,
                "must be 1–64 lowercase letters, digits or hyphens",
                ID_HINT,
            ));
        }
        let name = &manifest.name;
        if name.trim().is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(invalid(
                self.name,
                "must be 1–80 printable characters",
                TITLE_HINT,
            ));
        }
        let unspaced = |value: &str| !value.chars().any(|ch| ch.is_whitespace() || ch.is_control());
        if manifest.version.is_empty() || manifest.version.len() > 32 || !unspaced(&manifest.version) {
            return Err(invalid(
                self.version,
                "must be 1–32 characters without spaces",
                VERSION_HINT,
            ));
        }
        if manifest.keyword.is_empty() || manifest.keyword.len() > 32 || !unspaced(&manifest.keyword) {
            return Err(invalid(
                self.keyword,
                "must be 1–32 bytes without spaces",
                KEYWORD_HINT,
            ));
        }
        Ok(())
    }
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
        assert!(manifest.capabilities.is_empty());
        let declaring = read_folder(&[(
            PACKAGE_JSON,
            &package(r#"{"title": "W", "keyword": "wc", "capabilities": {"clipboard": ["write"]}}"#),
        )])
        .unwrap();
        let scopes: Vec<_> = declaring
            .capabilities
            .scopes()
            .map(|scope| scope.key().to_string())
            .collect();
        assert_eq!(scopes, ["clipboard:write"]);
        assert_ne!(declaring.digest, manifest.digest);
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
                r#"{"title": "W", "keyword": "wc", "capabilities": {"http": []}}"#,
                "empty",
                "dupin.capabilities.http",
            ),
            (
                r#"{"title": "W", "keyword": "wc", "capabilities": {"teleport": true}}"#,
                "unknown-capability",
                "dupin.capabilities.teleport",
            ),
            (
                r#"{"title": "W", "keyword": "wc", "capabilities": {"clipboard": true}}"#,
                "invalid-type",
                "dupin.capabilities.clipboard",
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
    fn the_folder_decides_its_runtime_unambiguously() {
        let wasm = read_folder(&[(WASM_MANIFEST, WASM)]).unwrap();
        assert_eq!(wasm.entry, "guest.wasm");
        let oversized = format!(
            r#"{{"name": "tooling", "x": "{}"}}"#,
            " ".repeat(MAX_PACKAGE_JSON_BYTES)
        );
        for tooling in [
            r#"{"name": "tooling"}"#,
            r#"{ "name": "tooling", }"#,
            "[]",
            r#"{"name": "tooling", "dupin": null}"#,
            &oversized,
        ] {
            let wasm = read_folder(&[(PACKAGE_JSON, tooling), (WASM_MANIFEST, WASM)])
                .unwrap_or_else(|diagnostic| panic!("{tooling:.40}: {diagnostic}"));
            assert_eq!(wasm.entry, "guest.wasm");
        }
        let dotted = read_folder(&[(WASM_MANIFEST, &WASM.replace("guest.wasm", "./guest.wasm"))]).unwrap();
        assert_eq!(dotted.entry, "guest.wasm");

        let both = problem(&[
            (PACKAGE_JSON, &package(r#"{"title": "W", "keyword": "wc"}"#)),
            (WASM_MANIFEST, WASM),
        ]);
        assert_eq!(
            (both.code.as_ref(), both.file.as_deref()),
            ("ambiguous-manifest", Some(WASM_MANIFEST))
        );

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

        for toml in [
            WASM.replace("entry = \"guest.wasm\"\n", ""),
            WASM.replace("guest.wasm", "main.js"),
            WASM.replace("guest.wasm", "../guest.wasm"),
        ] {
            let javascript = problem(&[(WASM_MANIFEST, &toml)]);
            assert_eq!(
                (javascript.code.as_ref(), javascript.field.as_deref()),
                ("invalid-manifest", Some("entry")),
                "{toml}"
            );
            assert!(javascript.hint.as_deref().unwrap().contains("package.json"));
        }
    }
}
