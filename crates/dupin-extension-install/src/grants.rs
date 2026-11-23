//! Declared grants: `extension-grants.toml` beside config.toml, which Home Manager writes, is the
//! read-only base; the grants recorded in extensions.lock overlay it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::Error;
use crate::lock::Entry;

pub const GRANTS_FILE: &str = "extension-grants.toml";

/// `[<package name>] grants = ["http:https://api.example.com", …]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Base(BTreeMap<String, Declared>);

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Declared {
    #[serde(default)]
    grants: Vec<String>,
}

impl Base {
    /// The base at `path`, or an empty one when there is none.
    pub fn read(path: &Path) -> Result<Self, Error> {
        match std::fs::read_to_string(path) {
            Ok(source) => toml::from_str(&source).map_err(|error| {
                Error::new("invalid-grants", error.message().trim_end().to_owned()).file(GRANTS_FILE)
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(Error::io(
                "invalid-grants",
                format!("read {}", path.display()),
                error,
            )),
        }
    }

    fn grants(&self, name: &str) -> &[String] {
        self.0.get(name).map_or(&[], |declared| &declared.grants)
    }
}

/// The scopes the broker may allow `name` to use: what the base declares and what the user
/// granted locally, limited to what the installed manifest still requests.
pub fn effective(base: &Base, entry: Option<&Entry>, name: &str, requested: &[String]) -> BTreeSet<String> {
    let requested: BTreeSet<_> = requested.iter().collect();
    base.grants(name)
        .iter()
        .chain(entry.into_iter().flat_map(|entry| &entry.granted))
        .filter(|scope| requested.contains(scope))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_grants_overlay_the_declared_base_within_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(GRANTS_FILE);
        assert_eq!(Base::read(&path).unwrap(), Base::default());
        std::fs::write(
            &path,
            "[weather]\ngrants = [\"http:https://api.example.com\", \"files:~/Notes\"]\n",
        )
        .unwrap();
        let base = Base::read(&path).unwrap();
        let entry = Entry {
            source: "github:a/b".into(),
            rev: None,
            commit: None,
            content_hash: String::new(),
            installed_at: String::new(),
            version: "1".into(),
            keyword: "w".into(),
            requested: vec![],
            granted: vec!["clipboard".into()],
            pending_consent: vec![],
        };
        let requested = ["http:https://api.example.com".to_owned(), "clipboard".to_owned()];
        assert_eq!(
            effective(&base, Some(&entry), "weather", &requested),
            BTreeSet::from(["clipboard".to_owned(), "http:https://api.example.com".to_owned()])
        );
        assert!(effective(&base, None, "other", &requested).is_empty());

        std::fs::write(&path, "[weather]\ngrant = []\n").unwrap();
        assert_eq!(Base::read(&path).unwrap_err().code, "invalid-grants");
    }
}
