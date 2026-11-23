use std::io::Read;
use std::path::{Component, Path};

use dupin_js_runtime::{DEFAULT_ENTRY, Diagnostic};
use serde_json::{Map, Value};

pub(crate) const MANIFEST_FILE: &str = "package.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// The parts of `package.json` the author tools need. The launcher validates the manifest in
/// full when it loads the package; this only rejects what would make a build or a development
/// copy go to the wrong place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageInfo {
    /// The extension's id, which also names its folder in the extensions directory.
    pub name: String,
    /// `dupin.main`: the module the launcher runs, relative to the package.
    pub(crate) main: String,
    /// `source`: what `dupin ext build` bundles into `main`. `None` means `main` is the source.
    pub(crate) source: Option<String>,
}

impl PackageInfo {
    pub(crate) fn needs_build(&self) -> bool {
        self.source.as_deref().is_some_and(|source| source != self.main)
    }
}

pub(crate) fn read_package(dir: &Path) -> Result<PackageInfo, Diagnostic> {
    let path = dir.join(MANIFEST_FILE);
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .and_then(|file| file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| {
            Diagnostic::new("missing-manifest", format!("Couldn't read it: {error}"))
                .file(MANIFEST_FILE)
                .hint("Create one with `dupin ext new <dir>`.")
        })?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid("", "is larger than 64 KiB", "Keep package.json small."));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        Diagnostic::new("invalid-manifest", error.to_string())
            .file(MANIFEST_FILE)
            .at(error.line() as u32, error.column() as u32)
            .hint("package.json must be valid JSON.")
    })?;
    let Some(root) = value.as_object() else {
        return Err(invalid(
            "",
            "must be a JSON object",
            "Start from `dupin ext new`.",
        ));
    };
    let name = string(
        root,
        "name",
        "name",
        "Use the extension's id, such as `word-count`.",
    )?;
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(invalid(
            "name",
            "must be 1–64 lowercase letters, digits or hyphens",
            "Use a name such as `word-count`, without an npm scope.",
        ));
    }
    string(root, "version", "version", "Use a version such as `0.1.0`.")?;
    let Some(dupin) = root.get("dupin").and_then(Value::as_object) else {
        return Err(invalid(
            "dupin",
            "must be an object with `title` and `keyword`",
            "Add `\"dupin\": { \"title\": \"Word Count\", \"keyword\": \"wc\" }`.",
        ));
    };
    string(
        dupin,
        "title",
        "dupin.title",
        "Use the name Settings should show.",
    )?;
    string(
        dupin,
        "keyword",
        "dupin.keyword",
        "Use a short word such as `wc`.",
    )?;
    let main = match dupin.get("main") {
        None => DEFAULT_ENTRY.to_owned(),
        Some(_) => string(dupin, "main", "dupin.main", "Use a path such as `dist/main.js`.")?,
    };
    let main = relative_path(&main, "dupin.main")?;
    if !(main.ends_with(".js") || main.ends_with(".mjs")) {
        return Err(invalid(
            "dupin.main",
            "must name a `.js` or `.mjs` file",
            "Point it at the JavaScript that `dupin ext build` writes, such as `dist/main.js`.",
        ));
    }
    let source = match root.get("source") {
        None => None,
        Some(_) => Some(relative_path(
            &string(root, "source", "source", "Use a path such as `src/main.ts`.")?,
            "source",
        )?),
    };
    Ok(PackageInfo { name, main, source })
}

fn string(object: &Map<String, Value>, key: &str, field: &str, hint: &str) -> Result<String, Diagnostic> {
    match object.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(invalid(field, "must be a string", hint)),
        None => Err(invalid(field, "is missing", hint)),
    }
}

/// `path` without `./`, as long as it stays inside the package.
fn relative_path(path: &str, field: &str) -> Result<String, Diagnostic> {
    let trimmed = path.strip_prefix("./").unwrap_or(path);
    let inside = !trimmed.is_empty()
        && Path::new(trimmed)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !inside {
        return Err(invalid(
            field,
            "must be a relative path inside the package",
            "Remove `..` and leading `/`.",
        ));
    }
    Ok(trimmed.to_owned())
}

fn invalid(field: &str, message: &str, hint: &str) -> Diagnostic {
    let diagnostic = Diagnostic::new("invalid-manifest", message)
        .file(MANIFEST_FILE)
        .hint(hint);
    if field.is_empty() {
        diagnostic
    } else {
        diagnostic.field(field)
    }
}
