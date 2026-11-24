//! Which loaded extensions run, by origin and by the user's choices, which the capability broker
//! keeps in grants.toml in the data directory.
//!
//! A user extension whose `name` matches a bundled one replaces it. A bundled package is
//! registered with the broker at its own revision, so the broker turns it on and pre-grants its
//! scopes exactly while the active copy has that revision; a changed copy has another revision
//! and gets neither. Extensions from the user's folder run while it is switched on, unless the
//! user turned one off.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use dupin_broker::{Base, Broker, ConsentState, Local, PackageId};

use super::{Extension, Origin};

pub const GRANTS_FILE: &str = "grants.toml";

/// How Settings describes where an extension came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bundled,
    /// The user's copy of a bundled extension, byte for byte the same.
    Copy,
    /// The user's copy of a bundled extension, changed: a third-party package.
    Modified,
    User,
}

/// One extension as Settings lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub title: String,
    pub keyword: String,
    pub kind: Kind,
    pub enabled: bool,
    /// Each declared scope, as grant files name it, and where it stands.
    pub scopes: Vec<(String, ConsentState)>,
}

pub struct Packages {
    broker: Broker,
    grants: Option<PathBuf>,
    /// Why the grants couldn't be read or saved.
    pub problem: Option<String>,
    bundled: Vec<Arc<Extension>>,
    user: Vec<Arc<Extension>>,
}

impl Packages {
    /// Reads the user's choices from `grants`. A file that exists but can't be read keeps
    /// every extension off rather than guessing what it held.
    pub fn load(grants: Option<PathBuf>) -> Self {
        let (local, problem) = match grants.as_deref().map(std::fs::read_to_string) {
            None => (Local::default(), None),
            Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => (Local::default(), None),
            Some(Ok(source)) => match Local::parse(&source) {
                Ok(local) => (local, None),
                Err(error) => (Local::unreadable(), Some(error.to_string())),
            },
            Some(Err(error)) => (
                Local::unreadable(),
                Some(format!("Couldn't read the extension grants: {error}")),
            ),
        };
        let problem = problem.map(|problem| format!("{problem}. Extensions stay off until it's fixed."));
        Self {
            broker: Broker::new(Base::default(), local),
            grants,
            problem,
            bundled: Vec::new(),
            user: Vec::new(),
        }
    }

    /// Takes a folder's latest scan and tells the broker which revision of each package is
    /// active and which revisions shipped with Dupin.
    pub fn set(&mut self, origin: Origin, extensions: Vec<Arc<Extension>>) {
        match origin {
            Origin::Bundled => self.bundled = extensions,
            Origin::User => self.user = extensions,
        }
        for extension in &self.bundled {
            if let Ok(id) = PackageId::new(&extension.id) {
                self.broker.set_bundled(id, extension.revision.clone());
            }
        }
        let present: Vec<_> = self.present().map(|(_, extension)| extension.clone()).collect();
        for extension in present {
            if let Ok(id) = PackageId::new(&extension.id) {
                let _ = self
                    .broker
                    .install(id, extension.revision.clone(), extension.capabilities.clone());
            }
        }
    }

    /// Every loaded extension that isn't replaced, bundled ones first.
    fn present(&self) -> impl Iterator<Item = (Origin, &Arc<Extension>)> {
        let replaced: HashSet<&str> = self.user.iter().map(|extension| extension.id.as_str()).collect();
        self.bundled
            .iter()
            .filter(move |extension| !replaced.contains(extension.id.as_str()))
            .map(|extension| (Origin::Bundled, extension))
            .chain(self.user.iter().map(|extension| (Origin::User, extension)))
    }

    fn enabled(&self, origin: Origin, extension: &Extension) -> bool {
        let Ok(id) = PackageId::new(&extension.id) else {
            return false;
        };
        match origin {
            Origin::Bundled => self.broker.is_enabled(&id),
            Origin::User => !self.broker.is_turned_off(&id),
        }
    }

    /// The extensions to register, bundled ones first, so a user extension never takes a
    /// bundled one's keyword.
    pub fn active(&self) -> Vec<Arc<Extension>> {
        self.present()
            .filter(|(origin, extension)| self.enabled(*origin, extension))
            .map(|(_, extension)| extension.clone())
            .collect()
    }

    pub fn listing(&self) -> Vec<Listed> {
        self.present()
            .map(|(origin, extension)| {
                let bundled = self.bundled.iter().find(|bundled| bundled.id == extension.id);
                let kind = match (origin, bundled) {
                    (Origin::Bundled, _) => Kind::Bundled,
                    (Origin::User, Some(bundled)) if bundled.revision == extension.revision => Kind::Copy,
                    (Origin::User, Some(_)) => Kind::Modified,
                    (Origin::User, None) => Kind::User,
                };
                let scopes = PackageId::new(&extension.id)
                    .map(|id| {
                        self.broker
                            .consents(&id)
                            .into_iter()
                            .map(|(scope, state)| (scope.key().to_string(), state))
                            .collect()
                    })
                    .unwrap_or_default();
                Listed {
                    id: extension.id.clone(),
                    title: extension.name.clone(),
                    keyword: extension.keyword.clone(),
                    kind,
                    enabled: self.enabled(origin, extension),
                    scopes,
                }
            })
            .collect()
    }

    /// Turns a package on or off and saves the choice.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        let id = PackageId::new(id)?;
        self.broker
            .set_enabled(&id, enabled)
            .map_err(|error| error.to_string())?;
        let Some(path) = &self.grants else {
            return Ok(());
        };
        self.broker.save(path).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::tests::{extension, module};
    use dupin_broker::{Capabilities, GrantSource, Revision};

    fn package(name: &str, keyword: &str, revision: &str) -> Arc<Extension> {
        let mut extension = extension(name, keyword, module("{}"));
        extension.revision = Revision::new(revision).unwrap();
        extension.capabilities =
            Capabilities::parse(&serde_json::json!({ "clipboard": ["write"], "files": { "notes": { "kind": "directory", "access": ["read"] } } }))
                .unwrap();
        Arc::new(extension)
    }

    fn ids(extensions: &[Arc<Extension>]) -> Vec<&str> {
        extensions.iter().map(|extension| extension.id.as_str()).collect()
    }

    fn listed<'a>(listing: &'a [Listed], id: &str) -> &'a Listed {
        listing.iter().find(|listed| listed.id == id).unwrap()
    }

    #[test]
    fn bundled_extensions_run_with_their_grants_until_a_changed_copy_replaces_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut packages = Packages::load(Some(dir.path().join(GRANTS_FILE)));
        packages.set(Origin::Bundled, vec![package("About", "about", "blake3:shipped")]);
        assert_eq!(ids(&packages.active()), ["about"]);
        let about = listed(&packages.listing(), "about").clone();
        assert_eq!((about.kind, about.enabled), (Kind::Bundled, true));
        assert_eq!(
            about.scopes,
            [
                ("files:notes".to_owned(), ConsentState::Undecided),
                (
                    "clipboard:write".to_owned(),
                    ConsentState::Granted(GrantSource::Bundled)
                ),
            ]
        );

        packages.set(Origin::User, vec![package("About", "about", "blake3:shipped")]);
        let copy = listed(&packages.listing(), "about").clone();
        assert_eq!((copy.kind, copy.enabled), (Kind::Copy, true));
        assert_eq!(copy.scopes, about.scopes);

        packages.set(
            Origin::User,
            vec![
                package("About", "about", "blake3:edited"),
                package("Words", "wc", "blake3:words"),
            ],
        );
        assert_eq!(ids(&packages.active()), ["about", "words"]);
        let modified = listed(&packages.listing(), "about").clone();
        assert_eq!((modified.kind, modified.enabled), (Kind::Modified, true));
        assert!(
            modified
                .scopes
                .iter()
                .all(|(_, state)| *state == ConsentState::Undecided),
            "{:?}",
            modified.scopes
        );
        assert_eq!(listed(&packages.listing(), "words").kind, Kind::User);

        packages.set(Origin::User, Vec::new());
        let restored = listed(&packages.listing(), "about").clone();
        assert_eq!(restored, about);
    }

    #[test]
    fn turning_an_extension_off_is_saved_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let grants = dir.path().join(GRANTS_FILE);
        let mut packages = Packages::load(Some(grants.clone()));
        packages.set(Origin::Bundled, vec![package("About", "about", "blake3:shipped")]);
        packages.set(Origin::User, vec![package("Words", "wc", "blake3:words")]);
        packages.set_enabled("about", false).unwrap();
        packages.set_enabled("words", false).unwrap();
        assert!(packages.active().is_empty());

        let mut restarted = Packages::load(Some(grants.clone()));
        restarted.set(Origin::Bundled, vec![package("About", "about", "blake3:shipped")]);
        restarted.set(Origin::User, vec![package("Words", "wc", "blake3:words")]);
        assert!(restarted.active().is_empty());
        assert!(restarted.listing().iter().all(|listed| !listed.enabled));
        restarted.set_enabled("about", true).unwrap();
        assert_eq!(ids(&restarted.active()), ["about"]);

        std::fs::write(&grants, "not toml").unwrap();
        let mut unreadable = Packages::load(Some(grants));
        unreadable.set(Origin::Bundled, vec![package("About", "about", "blake3:shipped")]);
        unreadable.set(Origin::User, vec![package("Words", "wc", "blake3:words")]);
        assert!(unreadable.active().is_empty());
        assert!(unreadable.problem.is_some());
        assert!(unreadable.set_enabled("about", true).is_err());
    }
}
