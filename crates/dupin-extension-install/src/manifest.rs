//! A minimal reader for the `dupin` key of package.json: what an installer needs before the
//! launcher's own loader sees the package. `capabilities` is checked by the capability broker,
//! so the scopes recorded in extensions.lock are the ones it grants.

use std::io::Read;
use std::path::Path;

use serde_json::{Map, Value};

use crate::Error;

pub(crate) const PACKAGE_JSON: &str = "package.json";
const DEFAULT_MAIN: &str = "main.js";
const MAX_PACKAGE_JSON_BYTES: u64 = 64 * 1024;

/// What an installer needs to know about a package, read without running it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Package {
    /// package.json's `name`: the extension's id and its folder name.
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) title: String,
    pub(crate) keyword: String,
    /// Relative to the package folder.
    pub(crate) main: String,
    /// Each declared scope as the capability broker names it, such as `files:notes`, sorted.
    pub(crate) capabilities: Vec<String>,
}

/// Reads and checks package.json in `folder`.
pub(crate) fn read(folder: &Path) -> Result<Package, Error> {
    let file = std::fs::File::open(folder.join(PACKAGE_JSON)).map_err(|error| {
        Error::new("missing-manifest", format!("couldn't be read: {error}"))
            .file(PACKAGE_JSON)
            .hint("A package needs package.json with `name`, `version` and a `dupin` key holding `title` and `keyword`.")
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_PACKAGE_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::new("invalid-manifest", error.to_string()).file(PACKAGE_JSON))?;
    if bytes.len() as u64 > MAX_PACKAGE_JSON_BYTES {
        return Err(Error::new("invalid-manifest", "is larger than 64 KiB").file(PACKAGE_JSON));
    }
    let Ok(Value::Object(package)) = serde_json::from_slice::<Value>(&bytes) else {
        return Err(Error::new("invalid-manifest", "must be a JSON object")
            .file(PACKAGE_JSON)
            .hint("package.json must be valid JSON, without comments."));
    };
    parse(&package)
}

fn parse(package: &Map<String, Value>) -> Result<Package, Error> {
    let invalid = |field: &str, message: &str| {
        Error::new("invalid-manifest", message.to_owned())
            .file(PACKAGE_JSON)
            .field(field.to_owned())
    };
    let Some(dupin) = package.get("dupin") else {
        return Err(Error::new("missing-manifest", "has no `dupin` key")
            .file(PACKAGE_JSON)
            .field("dupin")
            .hint("Add a `dupin` key holding `title` and `keyword` to make the folder a Dupin extension."));
    };
    let Value::Object(dupin) = dupin else {
        return Err(invalid("dupin", "must be an object"));
    };
    if let Some(key) = dupin.keys().find(|key| {
        !matches!(
            key.as_str(),
            "title"
                | "keyword"
                | "main"
                | "defaultSearch"
                | "emptyQuery"
                | "match"
                | "preferences"
                | "capabilities"
        )
    }) {
        return Err(invalid(&format!("dupin.{key}"), "is not a key Dupin reads"));
    }
    for key in ["defaultSearch", "emptyQuery"] {
        if dupin.get(key).is_some_and(|value| !value.is_boolean()) {
            return Err(invalid(&format!("dupin.{key}"), "must be true or false"));
        }
    }
    let string = |object: &Map<String, Value>, key: &str, field: &str| match object.get(key) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        None => Ok(None),
        Some(_) => Err(invalid(field, "must be a string")),
    };
    let required = |object: &Map<String, Value>, key: &str, field: &str| {
        string(object, key, field)?.ok_or_else(|| invalid(field, "is missing"))
    };
    let name = required(package, "name", "name")?;
    if !valid_name(&name) {
        return Err(
            invalid("name", "must be 1–64 lowercase letters, digits or hyphens")
                .hint("Use a name such as `word-count`; it becomes the extension's folder name."),
        );
    }
    let version = required(package, "version", "version")?;
    if version.is_empty()
        || version.len() > 32
        || version.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(invalid("version", "must be 1–32 characters without spaces"));
    }
    let title = required(dupin, "title", "dupin.title")?;
    if title.trim().is_empty() || title.chars().count() > 80 || title.chars().any(char::is_control) {
        return Err(invalid("dupin.title", "must be 1–80 printable characters"));
    }
    let keyword = required(dupin, "keyword", "dupin.keyword")?;
    if !valid_keyword(&keyword) {
        return Err(invalid("dupin.keyword", "must be 1–32 bytes without spaces"));
    }
    let main = string(dupin, "main", "dupin.main")?.unwrap_or_else(|| DEFAULT_MAIN.into());
    let main = main.strip_prefix("./").unwrap_or(&main).to_owned();
    let capabilities = match dupin.get("capabilities") {
        None => Vec::new(),
        Some(capabilities) => scopes(capabilities)?,
    };
    Ok(Package {
        name,
        version,
        title,
        keyword,
        main,
        capabilities,
    })
}

/// The scopes `capabilities` declares, named as the capability broker keys its grants, such as
/// `http:https://api.example.com`, `files:notes` or `clipboard:write`.
fn scopes(capabilities: &Value) -> Result<Vec<String>, Error> {
    let capabilities = dupin_broker::Capabilities::parse(capabilities).map_err(|diagnostics| {
        let diagnostic = &diagnostics[0];
        let error = Error::new("invalid-manifest", diagnostic.message.clone())
            .file(PACKAGE_JSON)
            .field(format!("dupin.{}", diagnostic.field));
        match diagnostic.hint {
            Some(hint) => error.hint(hint),
            None => error,
        }
    })?;
    let mut scopes: Vec<_> = capabilities
        .scopes()
        .map(|scope| scope.key().to_string())
        .collect();
    scopes.sort();
    Ok(scopes)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_keyword(keyword: &str) -> bool {
    !keyword.is_empty()
        && keyword.len() <= 32
        && !keyword.chars().any(|ch| ch.is_whitespace() || ch.is_control())
}

/// The keyword another folder in the extensions directory answers, so an install can refuse to
/// take it.
pub(crate) fn keyword_of(folder: &Path) -> Option<String> {
    read(folder).ok().map(|package| package.keyword)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parsed(value: Value) -> Result<Package, Error> {
        let Value::Object(package) = value else {
            unreachable!()
        };
        parse(&package)
    }

    #[test]
    fn the_dupin_key_describes_the_package() {
        let package = parsed(json!({
            "name": "word-count",
            "version": "0.1.0",
            "scripts": {"postinstall": "never runs"},
            "dupin": {
                "title": "Word Count",
                "keyword": "wc",
                "main": "./dist/main.js",
                "capabilities": {
                    "http": [
                        {"origin": "https://b.example", "methods": ["GET"]},
                        {"origin": "https://a.example", "methods": ["GET"]}
                    ],
                    "clipboard": ["write"]
                }
            }
        }))
        .unwrap();
        assert_eq!(package.main, "dist/main.js");
        assert_eq!(
            package.capabilities,
            [
                "clipboard:write",
                "http:https://a.example",
                "http:https://b.example"
            ]
        );
    }

    #[test]
    fn problems_name_the_key() {
        let base = || json!({"name": "wc", "version": "1", "dupin": {"title": "T", "keyword": "wc"}});
        let mut no_dupin = base();
        no_dupin.as_object_mut().unwrap().remove("dupin");
        assert_eq!(parsed(no_dupin).unwrap_err().code, "missing-manifest");
        let mut scoped = base();
        scoped["name"] = json!("@me/wc");
        assert_eq!(parsed(scoped).unwrap_err().field.as_deref(), Some("name"));
        let mut extra = base();
        extra["dupin"]["entry"] = json!("x.js");
        assert_eq!(parsed(extra).unwrap_err().field.as_deref(), Some("dupin.entry"));
        let mut spaced = base();
        spaced["dupin"]["keyword"] = json!("w c");
        assert_eq!(
            parsed(spaced).unwrap_err().field.as_deref(),
            Some("dupin.keyword")
        );
        let mut bad_scope = base();
        bad_scope["dupin"]["capabilities"] = json!({"http": 1});
        assert_eq!(
            parsed(bad_scope).unwrap_err().field.as_deref(),
            Some("dupin.capabilities.http")
        );
        let mut unknown = base();
        unknown["dupin"]["capabilities"] = json!({"camera": true});
        assert_eq!(
            parsed(unknown).unwrap_err().field.as_deref(),
            Some("dupin.capabilities.camera")
        );
    }
}
