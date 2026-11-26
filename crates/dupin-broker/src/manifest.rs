use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::scope::{
    ClipboardAccess, FilesAccess, FilesKind, Method, Origin, RowKind, Scope, ScopeKey, ScopeName,
    WindowsAccess,
};

const MAX_HTTP: usize = 32;
const MAX_NAMED: usize = 16;
const MAX_ARGS: usize = 32;
const MAX_ARG_BYTES: usize = 1024;
const MAX_QUOTA_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RETENTION_DAYS: u64 = 3650;
const MAX_CALENDAR_DAYS: u64 = 366;
const SCRIPT_PREFIX: &str = "@script:";

/// One problem in a capability declaration, in the shape the extension runtime reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// A stable, kebab-case name for the kind of problem.
    pub code: &'static str,
    /// The manifest path at fault, such as `capabilities.http[0].origin`.
    pub field: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<&'static str>,
}

/// What a package declares it may do: the most the user can grant it. Each scope is granted
/// or revoked on its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities(BTreeMap<ScopeKey, Scope>);

impl Capabilities {
    /// Validates the `capabilities` value of a package's `dupin` manifest. Every problem is
    /// reported, not just the first; any problem rejects the whole declaration.
    pub fn parse(value: &Value) -> Result<Self, Vec<Diagnostic>> {
        let mut parser = Parser::default();
        let mut scopes = BTreeMap::new();
        if let Some(object) = parser.object(value, "capabilities") {
            for (name, value) in object {
                let field = format!("capabilities.{name}");
                let parsed = match name.as_str() {
                    "http" => parser.http(value, &field),
                    "files" => parser.files(value, &field),
                    "exec" => parser.exec(value, &field),
                    "storage" => parser.storage(value, &field),
                    "clipboard" => parser.clipboard(value, &field),
                    "dictionary" => parser.dictionary(value, &field),
                    "calculator" => parser.calculator(value, &field),
                    "snippets" => parser.snippets(value, &field),
                    "calendar" => parser.calendar(value, &field),
                    "windows" => parser.windows(value, &field),
                    "rows" => parser.rows(value, &field),
                    "query" => parser.query(value, &field),
                    _ => {
                        parser.report(
                            "unknown-capability",
                            field,
                            format!("`{name}` isn't a capability"),
                            Some(
                                "Use http, files, exec, storage, clipboard, dictionary, calculator, snippets, calendar, windows, rows or query.",
                            ),
                        );
                        Vec::new()
                    }
                };
                for scope in parsed {
                    scopes.insert(scope.key(), scope);
                }
            }
        }
        if parser.diagnostics.is_empty() {
            Ok(Self(scopes))
        } else {
            Err(parser.diagnostics)
        }
    }

    pub fn get(&self, key: &ScopeKey) -> Option<&Scope> {
        self.0.get(key)
    }

    pub fn scopes(&self) -> impl Iterator<Item = &Scope> {
        self.0.values()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Default)]
struct Parser {
    diagnostics: Vec<Diagnostic>,
}

impl Parser {
    fn report(
        &mut self,
        code: &'static str,
        field: impl Into<String>,
        message: impl Into<String>,
        hint: Option<&'static str>,
    ) {
        self.diagnostics.push(Diagnostic {
            code,
            field: field.into(),
            message: message.into(),
            hint,
        });
    }

    fn wrong_type(&mut self, field: &str, expected: &str, value: &Value) {
        let found = match value {
            Value::Null => "null",
            Value::Bool(_) => "a boolean",
            Value::Number(_) => "a number",
            Value::String(_) => "a string",
            Value::Array(_) => "an array",
            Value::Object(_) => "an object",
        };
        self.report(
            "invalid-type",
            field,
            format!("must be {expected}, not {found}"),
            None,
        );
    }

    fn object<'a>(&mut self, value: &'a Value, field: &str) -> Option<&'a Map<String, Value>> {
        match value {
            Value::Object(object) => Some(object),
            other => {
                self.wrong_type(field, "an object", other);
                None
            }
        }
    }

    /// The object's fields, after reporting any not in `allowed` and any of `required` missing.
    fn fields<'a>(
        &mut self,
        value: &'a Value,
        field: &str,
        allowed: &[&str],
        required: &[&str],
    ) -> Option<&'a Map<String, Value>> {
        let object = self.object(value, field)?;
        for key in object.keys() {
            if !allowed.contains(&key.as_str()) {
                self.report(
                    "unknown-field",
                    format!("{field}.{key}"),
                    format!("`{key}` isn't a field here"),
                    None,
                );
            }
        }
        for key in required {
            if !object.contains_key(*key) {
                self.report(
                    "missing-field",
                    format!("{field}.{key}"),
                    format!("`{key}` is required"),
                    None,
                );
            }
        }
        Some(object)
    }

    fn integer(&mut self, value: &Value, field: &str, range: std::ops::RangeInclusive<u64>) -> Option<u64> {
        match value.as_u64() {
            Some(number) if range.contains(&number) => Some(number),
            Some(_) | None if value.is_number() => {
                self.report(
                    "out-of-range",
                    field,
                    format!("must be a whole number from {} to {}", range.start(), range.end()),
                    None,
                );
                None
            }
            _ => {
                self.wrong_type(field, "a whole number", value);
                None
            }
        }
    }

    fn boolean(&mut self, value: Option<&Value>, field: &str) -> bool {
        match value {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(other) => {
                self.wrong_type(field, "a boolean", other);
                false
            }
        }
    }

    /// A non-empty array of distinct strings, each mapped by `item`.
    fn set<T: Ord>(
        &mut self,
        value: &Value,
        field: &str,
        mut item: impl FnMut(&mut Self, &str, &str) -> Option<T>,
    ) -> Option<BTreeSet<T>> {
        let Value::Array(items) = value else {
            self.wrong_type(field, "an array of strings", value);
            return None;
        };
        if items.is_empty() {
            self.report(
                "empty",
                field,
                "must list at least one value",
                Some("Remove the field to declare nothing."),
            );
            return None;
        }
        let mut set = BTreeSet::new();
        let mut valid = true;
        for (index, value) in items.iter().enumerate() {
            let field = format!("{field}[{index}]");
            let Value::String(text) = value else {
                self.wrong_type(&field, "a string", value);
                valid = false;
                continue;
            };
            match item(self, text, &field) {
                Some(parsed) => {
                    if !set.insert(parsed) {
                        self.report("duplicate", field, format!("`{text}` is listed twice"), None);
                        valid = false;
                    }
                }
                None => valid = false,
            }
        }
        valid.then_some(set)
    }

    fn named<'a>(&mut self, value: &'a Value, field: &str) -> Vec<(String, &'a Value)> {
        let Some(object) = self.object(value, field) else {
            return Vec::new();
        };
        if object.is_empty() {
            self.report(
                "empty",
                field,
                "must declare at least one scope",
                Some("Remove the field to declare nothing."),
            );
        }
        if object.len() > MAX_NAMED {
            self.report(
                "too-many",
                field,
                format!("declares more than {MAX_NAMED} scopes"),
                None,
            );
            return Vec::new();
        }
        object.iter().map(|(name, value)| (name.clone(), value)).collect()
    }

    fn name(&mut self, name: &str, field: &str) -> Option<ScopeName> {
        ScopeName::new(name)
            .map_err(|message| self.report("invalid-name", field, message, None))
            .ok()
    }

    fn http(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        let Value::Array(entries) = value else {
            self.wrong_type(field, "an array", value);
            return Vec::new();
        };
        if entries.is_empty() {
            self.report(
                "empty",
                field,
                "must declare at least one origin",
                Some("Remove the field to declare nothing."),
            );
        }
        if entries.len() > MAX_HTTP {
            self.report(
                "too-many",
                field,
                format!("declares more than {MAX_HTTP} origins"),
                None,
            );
            return Vec::new();
        }
        let mut seen = BTreeSet::new();
        let mut scopes = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let field = format!("{field}[{index}]");
            let Some(object) = self.fields(entry, &field, &["origin", "methods"], &["origin", "methods"])
            else {
                continue;
            };
            let origin = match object.get("origin") {
                Some(Value::String(text)) => Origin::parse(text)
                    .map_err(|message| {
                        self.report(
                            "invalid-origin",
                            format!("{field}.origin"),
                            message,
                            Some("An origin is `https://` and a host, with no path, such as `https://api.github.com`."),
                        )
                    })
                    .ok(),
                Some(other) => {
                    self.wrong_type(&format!("{field}.origin"), "a string", other);
                    None
                }
                None => None,
            };
            let methods = object.get("methods").and_then(|methods| {
                self.set(methods, &format!("{field}.methods"), |parser, text, field| {
                    let method = Method::ALL.into_iter().find(|method| method.as_str() == text);
                    if method.is_none() {
                        parser.report(
                            "invalid-value",
                            field,
                            format!("`{text}` isn't an HTTP method"),
                            Some("Use GET, HEAD, POST, PUT, PATCH or DELETE, in capitals."),
                        );
                    }
                    method
                })
            });
            let (Some(origin), Some(methods)) = (origin, methods) else {
                continue;
            };
            if !seen.insert(origin.clone()) {
                self.report(
                    "duplicate",
                    format!("{field}.origin"),
                    format!("`{origin}` is declared twice"),
                    Some("List every method for an origin in one entry."),
                );
                continue;
            }
            scopes.push(Scope::Http { origin, methods });
        }
        scopes
    }

    fn files(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        let mut scopes = Vec::new();
        for (name, entry) in self.named(value, field) {
            let field = format!("{field}.{name}");
            let name = self.name(&name, &field);
            let Some(object) = self.fields(entry, &field, &["kind", "access"], &["kind", "access"]) else {
                continue;
            };
            let kind = match object.get("kind") {
                Some(Value::String(kind)) if kind == "file" => Some(FilesKind::File),
                Some(Value::String(kind)) if kind == "directory" => Some(FilesKind::Directory),
                Some(other @ Value::String(_)) => {
                    self.report(
                        "invalid-value",
                        format!("{field}.kind"),
                        format!("{other} isn't a kind"),
                        Some("Use \"file\" or \"directory\"."),
                    );
                    None
                }
                Some(other) => {
                    self.wrong_type(&format!("{field}.kind"), "a string", other);
                    None
                }
                None => None,
            };
            let access = object.get("access").and_then(|access| {
                self.set(
                    access,
                    &format!("{field}.access"),
                    |parser, text, field| match text {
                        "read" => Some(FilesAccess::Read),
                        "list" => Some(FilesAccess::List),
                        "write" => {
                            parser.report(
                                "unsupported-access",
                                field,
                                "files can't be written yet",
                                Some("Declare read or list."),
                            );
                            None
                        }
                        _ => {
                            parser.report(
                                "invalid-value",
                                field,
                                format!("`{text}` isn't a files access"),
                                Some("Use read or list."),
                            );
                            None
                        }
                    },
                )
            });
            let (Some(name), Some(kind), Some(access)) = (name, kind, access) else {
                continue;
            };
            if kind == FilesKind::File && access.contains(&FilesAccess::List) {
                self.report(
                    "invalid-value",
                    format!("{field}.access"),
                    "a single file can't be listed",
                    Some("Use \"kind\": \"directory\" to list its contents."),
                );
                continue;
            }
            scopes.push(Scope::Files { name, kind, access });
        }
        scopes
    }

    fn exec(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        let mut scopes = Vec::new();
        for (key, entry) in self.named(value, field) {
            let field = format!("{field}.{key}");
            if let Some(script) = key.strip_prefix(SCRIPT_PREFIX) {
                let name = self.name(script, &field);
                let Some(object) = self.fields(entry, &field, &["output"], &[]) else {
                    continue;
                };
                let output = self.boolean(object.get("output"), &format!("{field}.output"));
                if let Some(name) = name {
                    scopes.push(Scope::Script { name, output });
                }
                continue;
            }
            let name = self.name(&key, &field);
            let Some(object) = self.fields(entry, &field, &["args", "output"], &["args"]) else {
                continue;
            };
            let output = self.boolean(object.get("output"), &format!("{field}.output"));
            let args = object
                .get("args")
                .and_then(|args| self.args(args, &format!("{field}.args")));
            if let (Some(name), Some(args)) = (name, args) {
                scopes.push(Scope::Exec { name, args, output });
            }
        }
        scopes
    }

    fn args(&mut self, value: &Value, field: &str) -> Option<Vec<String>> {
        let Value::Array(items) = value else {
            self.wrong_type(field, "an array of strings", value);
            return None;
        };
        if items.len() > MAX_ARGS {
            self.report(
                "too-many",
                field,
                format!("has more than {MAX_ARGS} arguments"),
                None,
            );
            return None;
        }
        let mut args = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let field = format!("{field}[{index}]");
            let Value::String(arg) = item else {
                self.wrong_type(&field, "a string", item);
                continue;
            };
            if arg.len() > MAX_ARG_BYTES || !crate::displayable(arg) {
                self.report(
                    "invalid-value",
                    field,
                    format!(
                        "must be at most {MAX_ARG_BYTES} bytes with no control or invisible formatting characters"
                    ),
                    None,
                );
                continue;
            }
            if let Err(message) = placeholders(arg) {
                self.report(
                    "invalid-placeholder",
                    field,
                    message,
                    Some("Write an input as `{name}`, with a lowercase name."),
                );
                continue;
            }
            args.push(arg.clone());
        }
        (args.len() == items.len()).then_some(args)
    }

    fn storage(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        let Some(object) = self.fields(
            value,
            field,
            &["quotaBytes", "retentionDays", "sync"],
            &["quotaBytes"],
        ) else {
            return Vec::new();
        };
        let object = object.clone();
        let sync = self.boolean(object.get("sync"), &format!("{field}.sync"));
        let quota = object
            .get("quotaBytes")
            .and_then(|quota| self.integer(quota, &format!("{field}.quotaBytes"), 1..=MAX_QUOTA_BYTES));
        let retention = match object.get("retentionDays") {
            None => Some(None),
            Some(days) => self
                .integer(days, &format!("{field}.retentionDays"), 1..=MAX_RETENTION_DAYS)
                .map(|days| Some(days as u32)),
        };
        match (quota, retention) {
            (Some(quota_bytes), Some(retention_days)) => {
                let mut scopes = vec![Scope::Storage {
                    quota_bytes,
                    retention_days,
                }];
                if sync {
                    scopes.push(Scope::StorageSync);
                }
                scopes
            }
            _ => Vec::new(),
        }
    }

    fn clipboard(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        self.set(value, field, |parser, text, field| match text {
            "read" => Some(ClipboardAccess::Read),
            "write" => Some(ClipboardAccess::Write),
            "watch" => Some(ClipboardAccess::Watch),
            "paste" => Some(ClipboardAccess::Paste),
            _ => {
                parser.report(
                    "invalid-value",
                    field,
                    format!("`{text}` isn't a clipboard access"),
                    Some("Use read, write, watch or paste."),
                );
                None
            }
        })
        .into_iter()
        .flatten()
        .map(|access| Scope::Clipboard { access })
        .collect()
    }

    /// `{ "<kind>": ["<field>", …] }`: the fields of each kind of row the package's filters
    /// and kind actions may receive.
    fn rows(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        let Some(object) = self.object(value, field) else {
            return Vec::new();
        };
        if object.is_empty() {
            self.report(
                "empty",
                field,
                "must declare at least one kind of row",
                Some("Remove the field to declare nothing."),
            );
        }
        let mut scopes = Vec::new();
        for (name, fields) in object {
            let field = format!("{field}.{name}");
            let Some(kind) = RowKind::ALL.into_iter().find(|kind| kind.as_str() == name) else {
                self.report(
                    "invalid-value",
                    field,
                    format!("`{name}` isn't a kind of row"),
                    Some("Use text, url, file or app."),
                );
                continue;
            };
            let fields = self.set(fields, &field, |parser, text, field| {
                let found = kind.fields().iter().copied().find(|known| known.as_str() == text);
                if found.is_none() {
                    let known: Vec<_> = kind.fields().iter().map(|known| known.as_str()).collect();
                    parser.report(
                        "invalid-value",
                        field,
                        format!(
                            "{} rows have no `{text}`; they have {}",
                            kind.as_str(),
                            known.join(", ")
                        ),
                        None,
                    );
                }
                found
            });
            if let Some(fields) = fields {
                scopes.push(Scope::Rows { kind, fields });
            }
        }
        scopes
    }

    /// `["read"]`: what the user types where the package's matchers are composed.
    fn query(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        self.set(value, field, |parser, text, field| {
            (text == "read").then_some(()).or_else(|| {
                parser.report(
                    "invalid-value",
                    field,
                    format!("`{text}` isn't a query access"),
                    Some("Use read."),
                );
                None
            })
        })
        .map(|_| vec![Scope::Query])
        .unwrap_or_default()
    }

    fn dictionary(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        self.set(value, field, |parser, text, field| {
            (text == "lookup").then_some(()).or_else(|| {
                parser.report(
                    "invalid-value",
                    field,
                    format!("`{text}` isn't a dictionary access"),
                    Some("Use lookup."),
                );
                None
            })
        })
        .map(|_| vec![Scope::Dictionary])
        .unwrap_or_default()
    }

    fn calculator(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        self.set(value, field, |parser, text, field| {
            (text == "evaluate").then_some(()).or_else(|| {
                parser.report(
                    "invalid-value",
                    field,
                    format!("`{text}` isn't a calculator access"),
                    Some("Use evaluate."),
                );
                None
            })
        })
        .map(|_| vec![Scope::Calculator])
        .unwrap_or_default()
    }

    fn snippets(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        self.set(value, field, |parser, text, field| {
            (text == "write").then_some(()).or_else(|| {
                parser.report(
                    "invalid-value",
                    field,
                    format!("`{text}` isn't a snippets access"),
                    Some("Use write."),
                );
                None
            })
        })
        .map(|_| vec![Scope::Snippets])
        .unwrap_or_default()
    }

    fn calendar(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        let Some(object) = self.fields(
            value,
            field,
            &["access", "pastDays", "futureDays"],
            &["access", "pastDays", "futureDays"],
        ) else {
            return Vec::new();
        };
        let access = object.get("access").and_then(|access| {
            self.set(access, &format!("{field}.access"), |parser, text, field| {
                (text == "read").then_some(()).or_else(|| {
                    parser.report(
                        "invalid-value",
                        field,
                        format!("`{text}` isn't a calendar access"),
                        Some("Use read; Dupin can't change calendars."),
                    );
                    None
                })
            })
        });
        let mut days = |key: &str| {
            object
                .get(key)
                .and_then(|days| self.integer(days, &format!("{field}.{key}"), 0..=MAX_CALENDAR_DAYS))
                .map(|days| days as u32)
        };
        let (past, future) = (days("pastDays"), days("futureDays"));
        match (access, past, future) {
            (Some(_), Some(past_days), Some(future_days)) => vec![Scope::Calendar {
                past_days,
                future_days,
            }],
            _ => Vec::new(),
        }
    }

    fn windows(&mut self, value: &Value, field: &str) -> Vec<Scope> {
        self.set(value, field, |parser, text, field| match text {
            "read" => Some(WindowsAccess::Read),
            "arrange" => Some(WindowsAccess::Arrange),
            _ => {
                parser.report(
                    "invalid-value",
                    field,
                    format!("`{text}` isn't a windows access"),
                    Some("Use read or arrange."),
                );
                None
            }
        })
        .into_iter()
        .flatten()
        .map(|access| Scope::Windows { access })
        .collect()
    }
}

/// Checks that every brace in an argument template opens a `{name}` input.
fn placeholders(arg: &str) -> Result<(), String> {
    let mut rest = arg;
    while let Some(start) = rest.find(['{', '}']) {
        if rest[start..].starts_with('}') {
            return Err("has a `}` with no matching `{`".into());
        }
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            return Err("has a `{` with no matching `}`".into());
        };
        let name = &after[..end];
        let valid = name.starts_with(|c: char| c.is_ascii_lowercase())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !valid {
            return Err(format!("`{{{name}}}` isn't an input name"));
        }
        rest = &after[end + 1..];
    }
    Ok(())
}
