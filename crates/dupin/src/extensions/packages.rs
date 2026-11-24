//! Which loaded extensions run, by origin and by the user's choices, which the broker bridge
//! keeps in grants.toml in the data directory.
//!
//! A user extension whose `name` matches a bundled one replaces it. A bundled package is known
//! to the broker as `bundled:<name>`, so grants and choices made for it never pass to a package
//! from the extensions folder that reuses its name. A user copy that is byte for byte the
//! shipped revision is the bundled package; any other copy is a package of its own, with no
//! bundled pre-grants. A package runs unless the user or the declared base turned it off; one
//! never turned on runs on what the user types but can use nothing.

use std::collections::HashSet;
use std::sync::Arc;

use dupin_extension_bridge::{Bridge, PackageId};

use super::{Extension, Origin};

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
    pub package: PackageId,
    pub title: String,
    pub keyword: String,
    pub kind: Kind,
    /// Whether its keyword reaches it now; one that runs can lose its keyword to another
    /// command.
    pub registered: bool,
    /// Its catalogue answers the main search too.
    pub main_search: bool,
}

pub struct Packages {
    bridge: Bridge,
    /// `None` while the bundled folder's scan is on its way: until it lands, a user extension
    /// could take a bundled keyword or pass for a package of its own.
    bundled: Option<Vec<Arc<Extension>>>,
    user: Vec<Arc<Extension>>,
}

impl Packages {
    pub fn new(bridge: Bridge) -> Self {
        Self {
            bridge,
            bundled: None,
            user: Vec::new(),
        }
    }

    /// Holds back every extension until the next bundled scan lands.
    pub fn await_bundled(&mut self) {
        self.bundled = None;
    }

    /// Takes a folder's latest scan and tells the broker which revisions shipped with Dupin and
    /// which revision of each package is present.
    pub fn set(&mut self, origin: Origin, extensions: Vec<Arc<Extension>>) {
        match origin {
            Origin::Bundled => self.bundled = Some(extensions),
            Origin::User => self.user = extensions,
        }
        for extension in self.bundled.iter().flatten() {
            self.bridge
                .set_bundled(&extension.package, extension.revision.clone());
        }
        let fixture = super::fixtures::headless();
        for (kind, extension) in self.present() {
            let install = if kind == Kind::Bundled {
                Bridge::install_bundled
            } else {
                Bridge::install
            };
            install(
                &self.bridge,
                &extension.package,
                extension.revision.clone(),
                extension.capabilities.clone(),
            );
            if let Some(fixture) = fixture {
                fixture.grant(&self.bridge, &extension);
            }
        }
    }

    /// Every loaded extension that isn't replaced, bundled ones first, once the bundled ones
    /// are known. A user copy identical to the shipped package is the shipped package.
    pub fn present(&self) -> Vec<(Kind, Arc<Extension>)> {
        let Some(bundled) = self.bundled.as_deref() else {
            return Vec::new();
        };
        let replaced: HashSet<&str> = self.user.iter().map(|extension| extension.id.as_str()).collect();
        let shipped = |extension: &Extension| bundled.iter().find(|shipped| shipped.id == extension.id);
        bundled
            .iter()
            .filter(|extension| !replaced.contains(extension.id.as_str()))
            .map(|extension| (Kind::Bundled, extension.clone()))
            .chain(self.user.iter().map(|extension| match shipped(extension) {
                Some(shipped) if shipped.revision == extension.revision => (Kind::Copy, shipped.clone()),
                Some(_) => (Kind::Modified, extension.clone()),
                None => (Kind::User, extension.clone()),
            }))
            .collect()
    }

    /// The extensions to register, bundled ones first, so a user extension never takes a
    /// bundled one's keyword.
    pub fn active(&self) -> Vec<Arc<Extension>> {
        self.present()
            .into_iter()
            .filter(|(_, extension)| !self.bridge.is_turned_off(&extension.package))
            .map(|(_, extension)| extension)
            .collect()
    }

    /// Each present extension, with `registered` telling which ones own their keyword.
    pub fn listing(&self, registered: impl Fn(&Extension) -> bool) -> Vec<Listed> {
        self.present()
            .into_iter()
            .map(|(kind, extension)| Listed {
                package: extension.package.clone(),
                title: extension.name.clone(),
                keyword: extension.keyword.clone(),
                kind,
                registered: registered(&extension),
                main_search: extension.default_search,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::tests::extension;
    use dupin_extension_bridge::{
        Base, Broker, Capabilities, ConsentState, GrantSource, Local, Revision, SaveError, Services,
    };

    const GRANTS_FILE: &str = "grants.toml";

    fn bridge(grants: Option<&std::path::Path>) -> Bridge {
        match grants {
            Some(local) => Bridge::open(&local.with_file_name("base.toml"), local, Services::none()),
            None => Bridge::new(
                Broker::new(Base::default(), Local::default()),
                None,
                Services::none(),
            ),
        }
    }

    fn package(origin: Origin, name: &str, keyword: &str, revision: &str) -> Arc<Extension> {
        let mut extension = extension(name, keyword);
        if origin == Origin::Bundled {
            extension.package = PackageId::new(format!("bundled:{}", extension.id)).unwrap();
        }
        extension.revision = Revision::new(revision).unwrap();
        extension.capabilities = Capabilities::parse(&serde_json::json!({
            "clipboard": ["write"],
            "files": { "notes": { "kind": "directory", "access": ["read"] } },
        }))
        .unwrap();
        Arc::new(extension)
    }

    fn ids(extensions: &[Arc<Extension>]) -> Vec<&str> {
        extensions.iter().map(|extension| extension.id.as_str()).collect()
    }

    fn listed(packages: &Packages, id: &str) -> Listed {
        packages
            .listing(|_| true)
            .into_iter()
            .find(|listed| listed.title.to_lowercase() == id)
            .unwrap()
    }

    fn scopes(bridge: &Bridge, listed: &Listed) -> Vec<(String, ConsentState)> {
        bridge
            .scopes(&listed.package)
            .into_iter()
            .map(|entry| (entry.scope.key().to_string(), entry.state))
            .collect()
    }

    #[test]
    fn bundled_extensions_run_with_their_grants_until_a_changed_copy_replaces_them() {
        let bridge = bridge(None);
        let mut packages = Packages::new(bridge.clone());
        packages.set(
            Origin::Bundled,
            vec![package(Origin::Bundled, "About", "about", "blake3:shipped")],
        );
        packages.set(Origin::User, Vec::new());
        assert_eq!(ids(&packages.active()), ["about"]);
        let about = listed(&packages, "about");
        assert_eq!(about.kind, Kind::Bundled);
        assert!(bridge.is_enabled(&about.package));
        let shipped_scopes = scopes(&bridge, &about);
        assert_eq!(
            shipped_scopes,
            [
                ("files:notes".to_owned(), ConsentState::Undecided),
                (
                    "clipboard:write".to_owned(),
                    ConsentState::Granted(GrantSource::Bundled)
                ),
            ]
        );

        packages.set(
            Origin::User,
            vec![package(Origin::User, "About", "about", "blake3:shipped")],
        );
        let copy = listed(&packages, "about");
        assert_eq!((copy.kind, &copy.package), (Kind::Copy, &about.package));
        assert_eq!(scopes(&bridge, &copy), shipped_scopes);

        packages.set(
            Origin::User,
            vec![
                package(Origin::User, "About", "about", "blake3:edited"),
                package(Origin::User, "Words", "wc", "blake3:words"),
            ],
        );
        assert_eq!(ids(&packages.active()), ["about", "words"]);
        let modified = listed(&packages, "about");
        assert_eq!(modified.kind, Kind::Modified);
        assert_ne!(modified.package, about.package);
        assert!(!bridge.is_enabled(&modified.package));
        assert!(
            scopes(&bridge, &modified)
                .iter()
                .all(|(_, state)| *state == ConsentState::Undecided)
        );
        assert_eq!(listed(&packages, "words").kind, Kind::User);

        packages.set(Origin::User, Vec::new());
        assert_eq!(listed(&packages, "about"), about);
        assert_eq!(scopes(&bridge, &about), shipped_scopes);
    }

    #[test]
    fn a_package_reusing_a_bundled_name_gets_none_of_the_choices_made_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let grants = dir.path().join(GRANTS_FILE);
        let bridge = bridge(Some(&grants));
        let mut packages = Packages::new(bridge.clone());
        packages.set(
            Origin::Bundled,
            vec![package(Origin::Bundled, "About", "about", "blake3:shipped")],
        );
        let shipped = listed(&packages, "about").package;
        bridge.set_enabled(&shipped, false).unwrap();
        assert!(packages.active().is_empty());

        packages.set(
            Origin::User,
            vec![package(Origin::User, "About", "about", "blake3:third-party")],
        );
        let replaced = listed(&packages, "about");
        assert_eq!(replaced.kind, Kind::Modified);
        assert_eq!(ids(&packages.active()), ["about"]);

        bridge.set_enabled(&replaced.package, false).unwrap();
        packages.set(Origin::User, Vec::new());
        bridge.set_enabled(&shipped, true).unwrap();
        packages.set(
            Origin::User,
            vec![package(Origin::User, "About", "about", "blake3:third-party")],
        );
        assert!(packages.active().is_empty());

        let saved = std::fs::read_to_string(&grants).unwrap();
        assert!(saved.contains("\"bundled:about\""), "{saved}");
    }

    #[test]
    fn user_extensions_wait_for_the_bundled_scan() {
        let mut packages = Packages::new(bridge(None));
        packages.set(
            Origin::User,
            vec![package(Origin::User, "Help", "help", "blake3:user")],
        );
        assert!(packages.active().is_empty());
        assert!(packages.listing(|_| true).is_empty());

        packages.set(
            Origin::Bundled,
            vec![package(Origin::Bundled, "Dupin", "help", "blake3:shipped")],
        );
        assert_eq!(ids(&packages.active()), ["dupin", "help"]);

        packages.await_bundled();
        assert!(packages.active().is_empty());
        packages.set(Origin::Bundled, Vec::new());
        assert_eq!(ids(&packages.active()), ["help"]);
    }

    #[test]
    fn a_user_extension_runs_until_turned_off_and_the_choice_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let grants = dir.path().join(GRANTS_FILE);
        let load = || {
            let bridge = bridge(Some(&grants));
            let mut packages = Packages::new(bridge.clone());
            packages.set(
                Origin::Bundled,
                vec![package(Origin::Bundled, "About", "about", "blake3:shipped")],
            );
            packages.set(
                Origin::User,
                vec![package(Origin::User, "Words", "wc", "blake3:words")],
            );
            (bridge, packages)
        };
        let (bridge, packages) = load();
        let words = listed(&packages, "words").package;
        assert_eq!(ids(&packages.active()), ["about", "words"]);
        assert!(!bridge.is_enabled(&words), "it runs but can use nothing yet");
        bridge
            .set_enabled(&listed(&packages, "about").package, false)
            .unwrap();
        bridge.set_enabled(&words, false).unwrap();
        assert!(packages.active().is_empty());

        let (bridge, restarted) = load();
        assert!(restarted.active().is_empty());
        bridge
            .set_enabled(&listed(&restarted, "about").package, true)
            .unwrap();
        assert_eq!(ids(&restarted.active()), ["about"]);

        std::fs::write(&grants, "not toml").unwrap();
        let (bridge, unreadable) = load();
        assert!(unreadable.active().is_empty());
        assert!(!bridge.problems().is_empty());
        assert!(matches!(
            bridge.set_enabled(&listed(&unreadable, "about").package, true),
            Err(SaveError::Refused(_))
        ));
    }

    #[test]
    fn a_choice_that_cannot_be_saved_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = bridge(Some(&dir.path().join(GRANTS_FILE)));
        let mut packages = Packages::new(bridge.clone());
        packages.set(
            Origin::Bundled,
            vec![package(Origin::Bundled, "About", "about", "blake3:shipped")],
        );
        std::fs::create_dir(dir.path().join(format!("{GRANTS_FILE}.tmp"))).unwrap();
        assert!(
            bridge
                .set_enabled(&listed(&packages, "about").package, false)
                .is_err()
        );
        assert_eq!(ids(&packages.active()), ["about"]);
    }
}
