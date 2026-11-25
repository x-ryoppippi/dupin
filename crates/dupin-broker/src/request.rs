use std::path::{Component, Path};

use crate::scope::{
    ClipboardAccess, FilesAccess, Method, Origin, RowField, RowKind, Scope, ScopeKey, ScopeName, StorageOp,
    WindowsAccess,
};

/// One operation a host service is about to perform for a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Http {
        url: String,
        method: Method,
    },
    /// `path` is relative to the root the user picked for the scope; empty means the root.
    Files {
        scope: ScopeName,
        access: FilesAccess,
        path: String,
    },
    Exec {
        profile: ScopeName,
    },
    Script {
        name: ScopeName,
        output: bool,
    },
    /// Its quota isn't checked here: only the store can check it and write in one step, so
    /// the service applies the quota the permit's scope names.
    Storage {
        op: StorageOp,
    },
    Clipboard(ClipboardAccess),
    DictionaryLookup,
    CalculatorEvaluate,
    /// The window, in whole days around today, the read covers.
    CalendarRead {
        past_days: u32,
        future_days: u32,
    },
    Windows(WindowsAccess),
    /// Handing the package `field` of rows of `kind` that Dupin's own sources found.
    Rows {
        kind: RowKind,
        field: RowField,
    },
    /// Handing the package what the user typed.
    Query,
}

pub(crate) enum Resolved {
    Scope(ScopeKey),
    Invalid(String),
}

impl Request {
    pub(crate) fn key(&self) -> Resolved {
        Resolved::Scope(match self {
            Self::Http { url, .. } => match Origin::of_url(url) {
                Ok(origin) => ScopeKey::Http(origin),
                Err(message) => return Resolved::Invalid(message),
            },
            Self::Files { scope, path, .. } => {
                if let Err(message) = relative(path) {
                    return Resolved::Invalid(message);
                }
                ScopeKey::Files(scope.clone())
            }
            Self::Exec { profile } => ScopeKey::Exec(profile.clone()),
            Self::Script { name, .. } => ScopeKey::Script(name.clone()),
            Self::Storage { .. } => ScopeKey::Storage,
            Self::Clipboard(access) => ScopeKey::Clipboard(*access),
            Self::DictionaryLookup => ScopeKey::Dictionary,
            Self::CalculatorEvaluate => ScopeKey::Calculator,
            Self::CalendarRead { .. } => ScopeKey::Calendar,
            Self::Windows(access) => ScopeKey::Windows(*access),
            Self::Rows { kind, .. } => ScopeKey::Rows(*kind),
            Self::Query => ScopeKey::Query,
        })
    }

    /// Mutations and runs of native programs need a host-verified user activation.
    pub(crate) fn needs_activation(&self) -> bool {
        match self {
            Self::Http { method, .. } => method.mutates(),
            Self::Exec { .. } | Self::Script { .. } => true,
            Self::Clipboard(access) => *access != ClipboardAccess::Watch,
            Self::Windows(access) => *access == WindowsAccess::Arrange,
            Self::Files { .. }
            | Self::Storage { .. }
            | Self::DictionaryLookup
            | Self::CalculatorEvaluate
            | Self::CalendarRead { .. }
            | Self::Rows { .. }
            | Self::Query => false,
        }
    }

    /// Why the request goes beyond the declared scope its key names, if it does.
    pub(crate) fn exceeds(&self, declared: &Scope) -> Option<String> {
        match (self, declared) {
            (Self::Http { method, .. }, Scope::Http { methods, origin }) => (!methods.contains(method))
                .then(|| format!("{} isn't declared for {origin}", method.as_str())),
            (
                Self::Files { access, .. },
                Scope::Files {
                    access: allowed,
                    name,
                    ..
                },
            ) => (!allowed.contains(access)).then(|| {
                let access = match access {
                    FilesAccess::Read => "reading",
                    FilesAccess::List => "listing",
                };
                format!("{access} isn't declared for “{name}”")
            }),
            (
                Self::Script { output, .. },
                Scope::Script {
                    output: allowed,
                    name,
                },
            ) => (*output && !allowed).then(|| format!("returning the output of “{name}” isn't declared")),
            (
                Self::CalendarRead {
                    past_days,
                    future_days,
                },
                Scope::Calendar {
                    past_days: past,
                    future_days: future,
                },
            ) => (past_days > past || future_days > future)
                .then(|| format!("calendar reads are limited to {past} days back and {future} days ahead")),
            (Self::Rows { field, .. }, Scope::Rows { kind, fields }) => (!fields.contains(field))
                .then(|| format!("the {} of {} rows isn't declared", field.as_str(), kind.as_str())),
            _ => None,
        }
    }
}

/// A lexical check only: the files service still resolves symlinks against the picked root
/// when it opens the path.
fn relative(path: &str) -> Result<(), String> {
    if path.contains('\0') {
        return Err("a path can't contain NUL".into());
    }
    let path = Path::new(path);
    if path
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
    {
        Ok(())
    } else {
        Err("a path must stay inside its scope, with no `..` or leading `/`".into())
    }
}
