use std::fmt;

/// Why an install, update, removal or lock read failed, precise enough for a person or an agent
/// to act on: a stable kebab-case `code`, the package file or key at fault, and a hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    pub file: Option<String>,
    pub field: Option<String>,
    pub hint: Option<String>,
}

impl Error {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            file: None,
            field: None,
            hint: None,
        }
    }

    pub(crate) fn file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub(crate) fn field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub(crate) fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub(crate) fn io(code: &'static str, what: impl fmt::Display, error: std::io::Error) -> Self {
        Self::new(code, format!("couldn't {what}: {error}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.file, &self.field) {
            (Some(file), Some(field)) => write!(f, "{file}: `{field}` {}", self.message),
            (Some(file), None) => write!(f, "{file}: {}", self.message),
            (None, Some(field)) => write!(f, "`{field}` {}", self.message),
            (None, None) => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for Error {}
