use std::borrow::Cow;
use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

/// Messages and hints can carry text a guest chose, such as what it threw. Dupin shows them as
/// one subtitle line and writes them to its log on every failed query.
const MAX_TEXT_BYTES: usize = 1024;
const MAX_LABEL_BYTES: usize = 256;

/// A problem with an extension package or one of its answers, precise enough for an author (or
/// an agent) to fix without reading Dupin's source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Diagnostic(#[serde(deserialize_with = "Details::bounded")] Box<Details>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Details {
    /// A stable, kebab-case name for the kind of problem.
    pub code: Cow<'static, str>,
    pub message: String,
    /// The package file at fault, relative to the package folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The manifest key or result path at fault, such as `keyword` or `items[2].url`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Deref for Diagnostic {
    type Target = Details;

    fn deref(&self) -> &Details {
        &self.0
    }
}

impl Diagnostic {
    pub fn new(code: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
        Self(Box::new(Details {
            code: code.into(),
            message: one_line(message.into(), MAX_TEXT_BYTES),
            file: None,
            field: None,
            line: None,
            column: None,
            hint: None,
        }))
    }

    pub fn file(mut self, file: impl Into<String>) -> Self {
        self.0.file = Some(one_line(file.into(), MAX_LABEL_BYTES));
        self
    }

    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.0.field = Some(one_line(field.into(), MAX_LABEL_BYTES));
        self
    }

    pub fn at(mut self, line: u32, column: u32) -> Self {
        self.0.line = Some(line);
        self.0.column = Some(column);
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.0.hint = Some(one_line(hint.into(), MAX_TEXT_BYTES));
        self
    }
}

impl Details {
    /// A diagnostic read from elsewhere, such as the extension host process, is held to the
    /// same bounds as one made here.
    fn bounded<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Box<Details>, D::Error> {
        let details = Details::deserialize(deserializer)?;
        let code = if is_code(&details.code) {
            details.code
        } else {
            Cow::Borrowed("invalid-diagnostic")
        };
        let mut diagnostic = Diagnostic::new(code, details.message);
        if let Some(file) = details.file {
            diagnostic = diagnostic.file(file);
        }
        if let Some(field) = details.field {
            diagnostic = diagnostic.field(field);
        }
        diagnostic.0.line = details.line;
        diagnostic.0.column = details.column;
        if let Some(hint) = details.hint {
            diagnostic = diagnostic.hint(hint);
        }
        Ok(diagnostic.0)
    }
}

fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `text` with control characters escaped, cut to at most `max` bytes with an ellipsis.
fn one_line(text: String, max: usize) -> String {
    if text.len() <= max && !text.chars().any(char::is_control) {
        return text;
    }
    const ELLIPSIS: &str = "…";
    let mut line = String::with_capacity(max);
    let mut escaped = String::new();
    for ch in text.chars() {
        escaped.clear();
        if ch.is_control() {
            escaped.extend(ch.escape_default());
        } else {
            escaped.push(ch);
        }
        if line.len() + escaped.len() > max - ELLIPSIS.len() {
            line.push_str(ELLIPSIS);
            return line;
        }
        line.push_str(&escaped);
    }
    line
}

impl Diagnostic {
    /// The diagnostic as a log file may keep it: the kind of problem and its line, never the
    /// text. A guest chooses the message, the hint, a field's name and, through a thrown
    /// error's stack, the file, and may fill any of them with the query, results or clipboard.
    pub fn for_log(&self) -> impl fmt::Display + '_ {
        Logged(self)
    }
}

struct Logged<'a>(&'a Diagnostic);

impl fmt::Display for Logged<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.code)?;
        match (self.0.line, self.0.column) {
            (Some(line), Some(column)) => write!(f, " at line {line}, column {column}"),
            (Some(line), None) => write!(f, " at line {line}"),
            _ => Ok(()),
        }
    }
}

impl fmt::Display for Diagnostic {
    /// One line: where, then what. The hint is left to callers with room for it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            f.write_str(file)?;
            if let Some(line) = self.line {
                write!(f, ":{line}")?;
                if let Some(column) = self.column {
                    write!(f, ":{column}")?;
                }
            }
            f.write_str(": ")?;
        }
        if let Some(field) = &self.field {
            write!(f, "`{field}` ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}
