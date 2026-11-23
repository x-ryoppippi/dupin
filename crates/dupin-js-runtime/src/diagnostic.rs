use std::fmt;
use std::ops::Deref;

use serde::Serialize;

/// A problem with an extension package or one of its answers, precise enough for an author (or
/// an agent) to fix without reading Dupin's source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Diagnostic(Box<Details>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Details {
    /// A stable, kebab-case name for the kind of problem.
    pub code: &'static str,
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
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self(Box::new(Details {
            code,
            message: message.into(),
            file: None,
            field: None,
            line: None,
            column: None,
            hint: None,
        }))
    }

    pub fn file(mut self, file: impl Into<String>) -> Self {
        self.0.file = Some(file.into());
        self
    }

    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.0.field = Some(field.into());
        self
    }

    pub fn at(mut self, line: u32, column: u32) -> Self {
        self.0.line = Some(line);
        self.0.column = Some(column);
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.0.hint = Some(hint.into());
        self
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
