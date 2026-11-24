//! Which loaded extensions run, by origin and by the user's choices, which the capability broker
//! keeps in grants.toml in the data directory.
//!
//! A user extension whose `name` matches a bundled one replaces it. A bundled package is known
//! to the broker as `bundled:<name>`, a name no package.json can have, so grants and choices
//! made for it never pass to a package from the extensions folder that reuses its name. A user
//! copy that is byte for byte the shipped revision is the bundled package; any other copy is a
//! package of its own, with no bundled pre-grants. Extensions from the user's folder run while
//! it is switched on, unless the user turned one off.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use dupin_broker::{Base, Broker, ConsentState, Local, PackageId};

use super::{Extension, Origin};

pub const GRANTS_FILE: &str = "grants.toml";

const BUNDLED_PREFIX: &str = "bundled:";

/// How Settings describes where an extension came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bundled,
    /// The user's copy of a bundled extension, byte for byte the same.
    Copy,
    /// A package in the extensions folder with a bundled extension's name, changed from it: a
    /// third-party package.
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
    /// The user's choice, which the switch shows.
    pub enabled: bool,
    /// Whether its keyword reaches it now; an enabled extension can lose its keyword to
    /// another command.
    pub answering: bool,
    /// Each declared scope, as grant files name it, and where it stands.
    pub scopes: Vec<(String, ConsentState)>,
}

pub struct Packages {
    broker: Broker,
    grants: Option<PathBuf>,
    /// Why the grants couldn't be read or saved.
    pub problem: Option<String>,
    /// `None` while the bundled folder's scan is on its way: until it lands, a user extension
    /// could take a bundled keyword or pass for a package of its own.
    bundled: Option<Vec<Arc<Extension>>>,
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
            bundled: None,
            user: Vec::new(),
        }
    }

    /// Holds back every extension until the next bundled scan lands.
    pub fn await_bundled(&mut self) {
        self.bundled = None;
    }

    /// Takes a folder's latest scan and tells the broker which revision of each package is
    /// active and which revisions shipped with Dupin.
    pub fn set(&mut self, origin: Origin, extensions: Vec<Arc<Extension>>) {
        match origin {
            Origin::Bundled => self.bundled = Some(extensions),
            Origin::User => self.user = extensions,
        }
        for extension in self.bundled.iter().flatten() {
            if let Some(id) = bundled_id(extension) {
                self.broker.set_bundled(id, extension.revision.clone());
            }
        }
        let present: Vec<_> = self
            .present()
            .filter_map(|(origin, extension)| Some((self.package_id(origin, extension)?, extension.clone())))
            .collect();
        for (id, extension) in present {
            let _ = self
                .broker
                .install(id, extension.revision.clone(), extension.capabilities.clone());
        }
    }

    /// Every loaded extension that isn't replaced, bundled ones first, once the bundled ones
    /// are known.
    fn present(&self) -> impl Iterator<Item = (Origin, &Arc<Extension>)> {
        let bundled = self.bundled.as_deref();
        let replaced: HashSet<&str> = self.user.iter().map(|extension| extension.id.as_str()).collect();
        let user = bundled.map(|_| self.user.as_slice()).unwrap_or_default();
        bundled
            .unwrap_or_default()
            .iter()
            .filter(move |extension| !replaced.contains(extension.id.as_str()))
            .map(|extension| (Origin::Bundled, extension))
            .chain(user.iter().map(|extension| (Origin::User, extension)))
    }

    fn shipped(&self, extension: &Extension) -> Option<&Arc<Extension>> {
        self.bundled
            .iter()
            .flatten()
            .find(|bundled| bundled.id == extension.id)
    }

    /// Whether the broker knows this extension as the bundled package.
    fn is_bundled(&self, origin: Origin, extension: &Extension) -> bool {
        match origin {
            Origin::Bundled => true,
            Origin::User => self
                .shipped(extension)
                .is_some_and(|shipped| shipped.revision == extension.revision),
        }
    }

    fn package_id(&self, origin: Origin, extension: &Extension) -> Option<PackageId> {
        if self.is_bundled(origin, extension) {
            bundled_id(extension)
        } else {
            PackageId::new(&extension.id).ok()
        }
    }

    fn enabled(&self, origin: Origin, extension: &Extension) -> bool {
        let Some(id) = self.package_id(origin, extension) else {
            return false;
        };
        if self.is_bundled(origin, extension) {
            self.broker.is_enabled(&id)
        } else {
            !self.broker.is_turned_off(&id)
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

    /// Each present extension, with `answering` telling which ones were registered.
    pub fn listing(&self, answering: impl Fn(&Extension) -> bool) -> Vec<Listed> {
        self.present()
            .map(|(origin, extension)| {
                let kind = match (origin, self.shipped(extension)) {
                    (Origin::Bundled, _) => Kind::Bundled,
                    (Origin::User, Some(_)) if self.is_bundled(origin, extension) => Kind::Copy,
                    (Origin::User, Some(_)) => Kind::Modified,
                    (Origin::User, None) => Kind::User,
                };
                let scopes = self
                    .package_id(origin, extension)
                    .map(|id| {
                        self.broker
                            .consents(&id)
                            .into_iter()
                            .map(|(scope, state)| (scope.key().to_string(), state))
                            .collect()
                    })
                    .unwrap_or_default();
                let enabled = self.enabled(origin, extension);
                Listed {
                    id: extension.id.clone(),
                    title: extension.name.clone(),
                    keyword: extension.keyword.clone(),
                    kind,
                    enabled,
                    answering: enabled && answering(extension),
                    scopes,
                }
            })
            .collect()
    }

    /// Turns the present extension `id` on or off and saves the choice. A choice that can't be
    /// saved isn't made.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        let package = self
            .present()
            .find(|(_, extension)| extension.id == id)
            .and_then(|(origin, extension)| self.package_id(origin, extension))
            .ok_or_else(|| format!("`{id}` isn't loaded"))?;
        match &self.grants {
            Some(path) => self
                .broker
                .change_saved(path, |broker| broker.set_enabled(&package, enabled))
                .map_err(|error| error.to_string()),
            None => self
                .broker
                .set_enabled(&package, enabled)
                .map_err(|error| error.to_string()),
        }
    }
}

fn bundled_id(extension: &Extension) -> Option<PackageId> {
    PackageId::new(format!("{BUNDLED_PREFIX}{}", extension.id)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::tests::extension;
    use dupin_broker::{Capabilities, GrantSource, Revision};

    fn package(name: &str, keyword: &str, revision: &str) -> Arc<Extension> {
        let mut extension = extension(name, keyword);
        extension.revision = Revision::new(revision).unwrap();
        extension.capabilities =
            Capabilities::parse(&serde_json::json!({ "clipboard": ["write"], "files": { "notes": { "kind": "directory", "access": ["read"] } } }))
                .unwrap();
        Arc::new(extension)
    }

    fn ids(extensions: &[Arc<Extension>]) -> Vec<&str> {
        extensions.iter().map(|extension| extension.id.as_str()).collect()
    }

    fn listing(packages: &Packages) -> Vec<Listed> {
        packages.listing(|_| true)
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
        let about = listed(&listing(&packages), "about").clone();
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
        let copy = listed(&listing(&packages), "about").clone();
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
        let modified = listed(&listing(&packages), "about").clone();
        assert_eq!((modified.kind, modified.enabled), (Kind::Modified, true));
        assert!(
            modified
                .scopes
                .iter()
                .all(|(_, state)| *state == ConsentState::Undecided),
            "{:?}",
            modified.scopes
        );
        assert_eq!(listed(&listing(&packages), "words").kind, Kind::User);

        packages.set(Origin::User, Vec::new());
        let restored = listed(&listing(&packages), "about").clone();
        assert_eq!(restored, about);
    }

    #[test]
    fn a_package_reusing_a_bundled_name_gets_none_of_the_choices_made_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let grants = dir.path().join(GRANTS_FILE);
        let mut packages = Packages::load(Some(grants.clone()));
        packages.set(Origin::Bundled, vec![package("About", "about", "blake3:shipped")]);
        packages.set_enabled("about", false).unwrap();
        packages.set(
            Origin::User,
            vec![package("About", "about", "blake3:third-party")],
        );
        let replaced = listed(&listing(&packages), "about").clone();
        assert_eq!((replaced.kind, replaced.enabled), (Kind::Modified, true));

        packages.set_enabled("about", false).unwrap();
        packages.set(Origin::User, Vec::new());
        packages.set_enabled("about", true).unwrap();
        packages.set(
            Origin::User,
            vec![package("About", "about", "blake3:third-party")],
        );
        assert!(!listed(&listing(&packages), "about").enabled);

        let saved = std::fs::read_to_string(&grants).unwrap();
        assert!(saved.contains("\"bundled:about\""), "{saved}");
    }

    #[test]
    fn user_extensions_wait_for_the_bundled_scan() {
        let mut packages = Packages::load(None);
        packages.set(Origin::User, vec![package("Help", "help", "blake3:user")]);
        assert!(packages.active().is_empty());
        assert!(listing(&packages).is_empty());

        packages.set(Origin::Bundled, vec![package("Dupin", "help", "blake3:shipped")]);
        assert_eq!(ids(&packages.active()), ["dupin", "help"]);

        packages.await_bundled();
        assert!(packages.active().is_empty());
        packages.set(Origin::Bundled, Vec::new());
        assert_eq!(ids(&packages.active()), ["help"]);
    }

    #[test]
    fn the_listing_says_which_enabled_extensions_lost_their_keyword() {
        let mut packages = Packages::load(None);
        packages.set(Origin::Bundled, vec![package("Dupin", "help", "blake3:shipped")]);
        packages.set(Origin::User, vec![package("Help", "help", "blake3:user")]);
        let listing = packages.listing(|extension| extension.id == "dupin");
        let help = listed(&listing, "help");
        assert_eq!((help.enabled, help.answering), (true, false));
        assert!(listed(&listing, "dupin").answering);
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
        assert!(listing(&restarted).iter().all(|listed| !listed.enabled));
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

    #[test]
    fn a_choice_that_cannot_be_saved_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut packages = Packages::load(Some(dir.path().join(GRANTS_FILE)));
        packages.set(Origin::Bundled, vec![package("About", "about", "blake3:shipped")]);
        std::fs::create_dir(dir.path().join(format!("{GRANTS_FILE}.tmp"))).unwrap();
        assert!(packages.set_enabled("about", false).is_err());
        assert_eq!(ids(&packages.active()), ["about"]);
        assert!(listed(&listing(&packages), "about").enabled);
    }
}
