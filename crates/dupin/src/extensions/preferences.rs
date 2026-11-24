//! What a package lets the user choose, from the `preferences` of its manifest, and what is
//! chosen now. A feature that ships as a bundled package sets the values from its own Settings
//! rows; any other package gets the defaults it declares.
//!
//! The package Dupin shipped for a built-in that reads config.toml is also handed that
//! built-in's settings, by its package ID, so settings made before the port keep working and a
//! changed copy, which is third-party, never sees them. A catalogue loaded with other settings
//! than the current ones is asked for again from the start.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use dupin_extension_bridge::PackageId;
use dupin_js_runtime::Diagnostic;
use serde_json::{Map, Value};

use super::manifest::PACKAGE_JSON;

const MAX_PREFERENCES: usize = 16;
const MAX_NAME_BYTES: usize = 64;
const MAX_TITLE_BYTES: usize = 128;
const MAX_OPTIONS: usize = 32;
const MAX_OPTION_BYTES: usize = 64;
const HINT: &str = "Declare `{ \"name\": { \"type\": \"checkbox\", \"title\": \"…\", \"default\": true } }`, a `select` with `options` and a `default` among them, or a `number` with whole `min`, `max` and `default`.";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Checkbox,
    Select(Vec<String>),
    /// A whole number from `min` to `max`.
    Number(i64, i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Preference {
    kind: Kind,
    default: Value,
}

/// The preferences a package declares, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preferences(Vec<(String, Preference)>);

impl Preferences {
    pub(super) fn parse(value: &Value) -> Result<Self, Diagnostic> {
        let invalid = |field: String, message: &str| {
            Diagnostic::new("invalid-manifest", message)
                .file(PACKAGE_JSON)
                .field(field)
                .hint(HINT)
        };
        let Value::Object(declared) = value else {
            return Err(invalid("dupin.preferences".into(), "must be an object"));
        };
        if declared.len() > MAX_PREFERENCES {
            return Err(invalid(
                "dupin.preferences".into(),
                &format!("declares more than {MAX_PREFERENCES} preferences"),
            ));
        }
        let mut preferences = Vec::with_capacity(declared.len());
        for (name, declaration) in declared {
            let field = |key: &str| format!("dupin.preferences.{name}{key}");
            let named = !name.is_empty()
                && name.len() <= MAX_NAME_BYTES
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
            if !named {
                return Err(invalid(field(""), "must be named with letters, digits or `_`"));
            }
            let Value::Object(declaration) = declaration else {
                return Err(invalid(field(""), "must be an object"));
            };
            if let Some(key) = declaration.keys().find(|key| {
                !matches!(
                    key.as_str(),
                    "type" | "title" | "options" | "default" | "min" | "max"
                )
            }) {
                return Err(invalid(field(&format!(".{key}")), "is not a preference field"));
            }
            let title = declaration.get("title").and_then(Value::as_str);
            if !title.is_some_and(|title| {
                !title.trim().is_empty()
                    && title.len() <= MAX_TITLE_BYTES
                    && !title.chars().any(char::is_control)
            }) {
                return Err(invalid(
                    field(".title"),
                    &format!("must be one line of 1–{MAX_TITLE_BYTES} bytes"),
                ));
            }
            let default = declaration.get("default").cloned().unwrap_or(Value::Null);
            let kind = declaration.get("type").and_then(Value::as_str);
            if kind != Some("select") && declaration.contains_key("options") {
                return Err(invalid(field(".options"), "only goes with `select`"));
            }
            if let Some(key) = ["min", "max"]
                .into_iter()
                .find(|key| kind != Some("number") && declaration.contains_key(*key))
            {
                return Err(invalid(field(&format!(".{key}")), "only goes with `number`"));
            }
            let kind = match kind {
                Some("checkbox") => {
                    if !default.is_boolean() {
                        return Err(invalid(field(".default"), "must be true or false"));
                    }
                    Kind::Checkbox
                }
                Some("select") => {
                    let options: Vec<String> = declaration
                        .get("options")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|option| option.as_str())
                        .filter(|option| {
                            !option.is_empty()
                                && option.len() <= MAX_OPTION_BYTES
                                && !option.chars().any(char::is_control)
                        })
                        .map(str::to_owned)
                        .collect();
                    let listed = declaration
                        .get("options")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    if options.is_empty() || options.len() != listed || options.len() > MAX_OPTIONS {
                        return Err(invalid(
                            field(".options"),
                            &format!("must list 1–{MAX_OPTIONS} one-line strings"),
                        ));
                    }
                    if !default
                        .as_str()
                        .is_some_and(|default| options.iter().any(|option| option == default))
                    {
                        return Err(invalid(field(".default"), "must be one of `options`"));
                    }
                    Kind::Select(options)
                }
                Some("number") => {
                    let bound = |key: &str| declaration.get(key).and_then(Value::as_i64);
                    let (Some(min), Some(max)) = (bound("min"), bound("max")) else {
                        return Err(invalid(field(".min"), "and `max` must be whole numbers"));
                    };
                    if min > max {
                        return Err(invalid(field(".min"), "must not be above `max`"));
                    }
                    if !default
                        .as_i64()
                        .is_some_and(|default| (min..=max).contains(&default))
                    {
                        return Err(invalid(
                            field(".default"),
                            "must be a whole number from `min` to `max`",
                        ));
                    }
                    Kind::Number(min, max)
                }
                _ => {
                    return Err(invalid(
                        field(".type"),
                        "must be \"checkbox\", \"select\" or \"number\"",
                    ));
                }
            };
            preferences.push((name.clone(), Preference { kind, default }));
        }
        Ok(Self(preferences))
    }

    /// Every declared preference: the value `package`'s feature chose, else its default.
    pub(super) fn values(&self, package: &str) -> Value {
        let chosen = lock_read().get(package).cloned().unwrap_or_default();
        let values: Map<String, Value> = self
            .0
            .iter()
            .map(|(name, preference)| {
                let value = chosen
                    .get(name)
                    .filter(|value| preference.accepts(value))
                    .unwrap_or(&preference.default);
                (name.clone(), value.clone())
            })
            .collect();
        Value::Object(values)
    }
}

impl Preference {
    fn accepts(&self, value: &Value) -> bool {
        match &self.kind {
            Kind::Checkbox => value.is_boolean(),
            Kind::Select(options) => value
                .as_str()
                .is_some_and(|value| options.iter().any(|option| option == value)),
            Kind::Number(min, max) => value.as_i64().is_some_and(|value| (*min..=*max).contains(&value)),
        }
    }
}

fn chosen() -> &'static RwLock<HashMap<String, Map<String, Value>>> {
    static CHOSEN: OnceLock<RwLock<HashMap<String, Map<String, Value>>>> = OnceLock::new();
    CHOSEN.get_or_init(RwLock::default)
}

fn lock_read() -> std::sync::RwLockReadGuard<'static, HashMap<String, Map<String, Value>>> {
    chosen().read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Chooses the values the package called `package` sees, from its feature's settings, whatever
/// copy of it runs. Returns whether they changed.
pub(crate) fn choose(package: &str, values: Map<String, Value>) -> bool {
    let mut chosen = chosen()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if chosen.get(package) == Some(&values) {
        return false;
    }
    chosen.insert(package.to_owned(), values);
    true
}

/// The config.toml settings a built-in hands the package it shipped as.
pub(crate) type Settings = dupin_js_runtime::Preferences;

fn held() -> &'static RwLock<HashMap<String, Arc<Settings>>> {
    static HELD: OnceLock<RwLock<HashMap<String, Arc<Settings>>>> = OnceLock::new();
    HELD.get_or_init(RwLock::default)
}

fn none() -> &'static Arc<Settings> {
    static NONE: OnceLock<Arc<Settings>> = OnceLock::new();
    NONE.get_or_init(Arc::default)
}

/// Sets the config.toml settings of `package`, and says whether they changed.
pub(crate) fn set(package: &str, settings: Settings) -> bool {
    let mut held = held().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if held.get(package).is_some_and(|current| **current == settings) {
        return false;
    }
    if settings.is_empty() {
        return held.remove(package).is_some();
    }
    held.insert(package.to_owned(), Arc::new(settings));
    true
}

/// The config.toml settings of `package`: the same value until they are set again.
pub(crate) fn of(package: &PackageId) -> Arc<Settings> {
    held()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(package.as_str())
        .unwrap_or_else(|| none())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_stay_the_same_value_until_they_change() {
        let package = PackageId::new("bundled:preferences-test").unwrap();
        assert!(of(&package).is_empty());
        let mut settings = Settings::new();
        settings.insert("a".into(), 1.into());
        assert!(set(package.as_str(), settings.clone()));
        let first = of(&package);
        assert!(!set(package.as_str(), settings));
        assert!(Arc::ptr_eq(&first, &of(&package)));
        assert!(set(package.as_str(), Settings::new()));
        assert!(of(&package).is_empty());
        assert!(!set(package.as_str(), Settings::new()));
    }

    #[test]
    fn declared_defaults_give_way_to_valid_chosen_values() {
        let preferences = Preferences::parse(&json!({
            "retention": { "type": "select", "title": "Keep", "options": ["1d", "7d"], "default": "7d" },
            "images": { "type": "checkbox", "title": "Images", "default": true },
            "entries": { "type": "number", "title": "Entries", "min": 1, "max": 10, "default": 5 },
        }))
        .unwrap();
        assert_eq!(
            preferences.values("preferences-test-a"),
            json!({ "retention": "7d", "images": true, "entries": 5 })
        );
        let mut values = Map::new();
        values.insert("retention".into(), json!("1d"));
        values.insert("images".into(), json!("no"));
        values.insert("entries".into(), json!(11));
        values.insert("undeclared".into(), json!(1));
        assert!(choose("preferences-test-a", values.clone()));
        assert!(!choose("preferences-test-a", values));
        assert_eq!(
            preferences.values("preferences-test-a"),
            json!({ "retention": "1d", "images": true, "entries": 5 })
        );
    }

    #[test]
    fn malformed_declarations_name_the_field() {
        for (declared, field) in [
            (json!([]), "dupin.preferences"),
            (json!({ "a-b": {} }), "dupin.preferences.a-b"),
            (
                json!({ "a": { "type": "select", "title": "A", "options": [], "default": "x" } }),
                "dupin.preferences.a.options",
            ),
            (
                json!({ "a": { "type": "select", "title": "A", "options": ["x"], "default": "y" } }),
                "dupin.preferences.a.default",
            ),
            (
                json!({ "a": { "type": "checkbox", "title": "A", "default": "yes" } }),
                "dupin.preferences.a.default",
            ),
            (
                json!({ "a": { "type": "text", "title": "A", "default": "" } }),
                "dupin.preferences.a.type",
            ),
            (
                json!({ "a": { "type": "number", "title": "A", "min": 1, "max": 3, "default": 4 } }),
                "dupin.preferences.a.default",
            ),
            (
                json!({ "a": { "type": "checkbox", "title": "A", "min": 1, "default": true } }),
                "dupin.preferences.a.min",
            ),
        ] {
            let problem = Preferences::parse(&declared).unwrap_err();
            assert_eq!(problem.field.as_deref(), Some(field), "{declared}");
        }
    }
}
