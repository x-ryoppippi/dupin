//! The `context` capability: what Dupin noted, as the panel opened, about where the user was:
//! the app in front, and the file its focused window shows.
//!
//! Both are read once per opening, never while the user types. The document comes from the
//! Accessibility API, and only while macOS already trusts Dupin with it: the service asks
//! nothing of macOS, and without that trust, or for an app that names no file, there is none.

use serde::{Deserialize, Serialize};

use crate::service::bounded;

const MAX_NAME_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct App {
    pub name: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    pub pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// An absolute path.
    pub path: String,
}

/// Where the context comes from: what `dupin-mac` noted as the panel opened, [`Fixture`] in
/// tests.
pub trait Context: Send + Sync {
    /// The app in front when the panel opened, or none while Dupin itself was.
    fn frontmost(&self) -> Option<App>;
    /// The file the front app's focused window showed then, if Dupin could tell.
    fn document(&self) -> Option<Document>;
}

pub struct Service<C> {
    backend: C,
}

impl<C: Context> Service<C> {
    pub fn new(backend: C) -> Self {
        Self { backend }
    }

    pub fn frontmost(&self) -> Option<App> {
        self.backend.frontmost().map(|app| App {
            name: bounded(app.name, MAX_NAME_BYTES),
            bundle_id: app.bundle_id.filter(|id| id.len() <= MAX_NAME_BYTES),
            pid: app.pid,
        })
    }

    /// The document, when its path is absolute, within the size limit and free of control
    /// characters, so a guest never receives a path it would have to second-guess.
    pub fn document(&self) -> Option<Document> {
        self.backend.document().filter(|document| {
            document.path.starts_with('/')
                && document.path.len() <= MAX_PATH_BYTES
                && !document.path.chars().any(char::is_control)
        })
    }
}

/// A fixed app and document.
#[derive(Clone, Debug, Default)]
pub struct Fixture {
    pub frontmost: Option<App>,
    pub document: Option<Document>,
}

impl Context for Fixture {
    fn frontmost(&self) -> Option<App> {
        self.frontmost.clone()
    }

    fn document(&self) -> Option<Document> {
        self.document.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(document: &str) -> Service<Fixture> {
        Service::new(Fixture {
            frontmost: Some(App {
                name: "x".repeat(MAX_NAME_BYTES + 10),
                bundle_id: Some("b".repeat(MAX_NAME_BYTES + 1)),
                pid: 42,
            }),
            document: Some(Document {
                path: document.into(),
            }),
        })
    }

    #[test]
    fn the_app_is_bounded_and_an_overlong_bundle_id_is_dropped() {
        let app = service("/a").frontmost().unwrap();
        assert_eq!(app.name.len(), MAX_NAME_BYTES);
        assert_eq!((app.bundle_id, app.pid), (None, 42));
    }

    #[test]
    fn only_an_absolute_path_of_one_line_within_the_limit_is_handed_over() {
        assert_eq!(
            service("/Users/me/notes.md").document(),
            Some(Document {
                path: "/Users/me/notes.md".into()
            })
        );
        for refused in [
            "notes.md".to_owned(),
            "file:///Users/me/notes.md".to_owned(),
            "/Users/me/a\nb".to_owned(),
            format!("/{}", "a".repeat(MAX_PATH_BYTES)),
        ] {
            assert_eq!(service(&refused).document(), None, "{refused:?}");
        }
    }
}
