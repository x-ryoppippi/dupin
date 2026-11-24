//! Every bundled extension, built-in commands' included, follows one override rule: a copy in
//! the extensions folder with its `name` answers in its place, is the bundled package while it
//! is unchanged, loses the bundled grants once edited, and deleting it brings the bundled one
//! back. Each test drives one bundled package through `dupin ext eject`, the folder scans,
//! the broker, command registration and the extension host.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use dupin_core::{CommandId, Config, Provider, Request};
use dupin_extension_bridge::{Base, Bridge, Broker, ConsentState, Local, Services};

use super::packages::{Kind, Listed, Packages};
use super::{Extension, Origin, Scanner, bundled};
use crate::features::extensions::{Port, add_extensions, hand_over, provider};

/// The bundled packages with a test below; a package added to `extensions/` needs one.
const COVERED: &[&str] = &["calendar", "dupin", "emoji"];

const EDITED: &str = "export function search(query: string) { return [{ title: `edited: ${query}` }]; }\n";

struct Override {
    name: &'static str,
    /// The command that answers the package's keyword.
    command: String,
    /// The package answers a built-in command, through its port.
    built_in: bool,
    /// The configuration under which that command answers.
    on: Config,
    /// One that turns the built-in command off, if a setting can.
    off: Option<Config>,
}

impl Override {
    fn extension(name: &'static str, on: Config) -> Self {
        Self {
            name,
            command: format!("extension.{name}"),
            built_in: false,
            on,
            off: None,
        }
    }

    fn built_in(name: &'static str, on: Config, off: Option<Config>) -> Self {
        let command = crate::features::ported_command(name)
            .unwrap_or_else(|| panic!("no built-in command is answered by {name}"));
        Self {
            name,
            command: command.to_owned(),
            built_in: true,
            on,
            off,
        }
    }

    fn command(&self) -> CommandId {
        CommandId::new(self.command.clone()).unwrap()
    }

    /// The extension the command routes to under `config`, given what runs now.
    fn routed(&self, config: &Config, packages: &Packages) -> Option<Arc<Extension>> {
        let mut configured = crate::commands::configured(config).unwrap();
        let added = add_extensions(&mut configured, &packages.active());
        added
            .registered
            .into_iter()
            .find(|(command, ..)| *command == self.command())
            .map(|(_, extension, _)| extension)
    }

    /// What the command answers for `query`, through the port of the built-in it ports or the
    /// searcher of its own, as the launcher hands the extension over.
    fn answer(&self, packages: &Packages, bridge: &Bridge, query: &str) -> Vec<String> {
        let mut configured = crate::commands::configured(&self.on).unwrap();
        let added = add_extensions(&mut configured, &packages.active());
        let ports: HashMap<_, _> = self
            .built_in
            .then(|| (self.command(), Arc::new(Port::default())))
            .into_iter()
            .collect();
        let (command, extension, role) = added
            .registered
            .into_iter()
            .find(|(command, ..)| *command == self.command())
            .expect("the command is registered");
        let searcher = hand_over(&ports, &command, provider(extension, role, bridge.clone()));
        let answering: Arc<dyn Provider> = match ports.get(&command) {
            Some(port) => {
                assert!(searcher.is_none(), "a built-in command keeps its own searcher");
                port.provider().expect("the port holds the extension")
            }
            None => searcher.expect("an extension's command gets a searcher"),
        };
        let mut rows = Vec::new();
        answering
            .search(&Request::detached(query, 10), &mut |batch| rows.extend(batch))
            .unwrap();
        rows.iter().map(|row| row.display_title().to_owned()).collect()
    }

    fn run(&self) {
        dupin_extension_host::skip_inside_a_sandbox!();
        let shipped = bundled::directory().unwrap();
        let bridge = Bridge::new(
            Broker::new(Base::default(), Local::default()),
            None,
            Services::none(),
        );
        let mut packages = Packages::new(bridge.clone());
        let seals = bundled::Seals::shipped();
        let mut bundled_scan = Scanner {
            integrity: Some(Box::new(move |folder: &Path| seals.check(folder))),
            bundled: true,
            ..Scanner::default()
        };
        let catalog = bundled_scan.scan(&shipped);
        assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
        packages.set(Origin::Bundled, catalog.extensions);
        let user = tempfile::tempdir().unwrap();
        let mut user_scan = Scanner::default();
        let mut rescan = |packages: &mut Packages| {
            let catalog = user_scan.scan(user.path());
            assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
            packages.set(Origin::User, catalog.extensions);
        };
        rescan(&mut packages);

        let original = self.listed(&packages);
        assert_eq!(original.kind, Kind::Bundled);
        assert_eq!(original.package.as_str(), format!("bundled:{}", self.name));
        assert_eq!(original.built_in, self.built_in);
        let shipped_extension = self.routed(&self.on, &packages).expect("the bundled one answers");
        let shipped_scopes = scopes(&bridge, &original);

        let copy = bundled::eject(&shipped, user.path(), self.name).unwrap();
        rescan(&mut packages);
        let unchanged = self.listed(&packages);
        assert_eq!(
            (unchanged.kind, &unchanged.package),
            (Kind::Copy, &original.package),
            "an unchanged copy is the bundled package"
        );
        assert_eq!(scopes(&bridge, &unchanged), shipped_scopes);
        let routed = self.routed(&self.on, &packages).unwrap();
        assert!(Arc::ptr_eq(&routed, &shipped_extension));

        std::fs::write(copy.join(main(&copy)), EDITED).unwrap();
        rescan(&mut packages);
        let modified = self.listed(&packages);
        assert_eq!(modified.kind, Kind::Modified);
        assert_eq!(modified.package.as_str(), self.name, "it has grants of its own");
        assert_eq!(modified.built_in, self.built_in);
        assert!(!bridge.is_enabled(&modified.package), "it waits to be turned on");
        let edited_scopes = scopes(&bridge, &modified);
        assert_eq!(
            edited_scopes.iter().map(|(key, _)| key).collect::<Vec<_>>(),
            shipped_scopes.iter().map(|(key, _)| key).collect::<Vec<_>>(),
            "it declares what it declared"
        );
        assert!(
            edited_scopes
                .iter()
                .all(|(_, state)| *state == ConsentState::Undecided),
            "none of the bundled grants pass to it: {edited_scopes:?}"
        );
        let routed = self.routed(&self.on, &packages).expect("the copy answers");
        assert_eq!(routed.root, copy, "the user's copy answers, not the bundled one");
        assert_eq!(
            self.answer(&packages, &bridge, "override"),
            ["edited: override"],
            "the edit takes effect"
        );
        if let Some(off) = &self.off {
            assert!(
                self.routed(off, &packages).is_none(),
                "the built-in's own setting still turns the copy off"
            );
        }
        bridge.set_enabled(&modified.package, false).unwrap();
        assert!(
            self.routed(&self.on, &packages).is_none(),
            "turning the copy off doesn't bring the bundled one back"
        );
        bridge.set_enabled(&modified.package, true).unwrap();

        std::fs::remove_dir_all(&copy).unwrap();
        rescan(&mut packages);
        assert_eq!(self.listed(&packages), original);
        assert_eq!(scopes(&bridge, &original), shipped_scopes, "its grants are back");
        let routed = self.routed(&self.on, &packages).unwrap();
        assert!(Arc::ptr_eq(&routed, &shipped_extension));
    }

    /// The package as Settings → Extensions lists it, which it always does.
    fn listed(&self, packages: &Packages) -> Listed {
        packages
            .listing(|_| true)
            .into_iter()
            .find(|listed| listed.package.as_str().trim_start_matches("bundled:") == self.name)
            .unwrap_or_else(|| panic!("Settings doesn't list {}", self.name))
    }
}

fn main(package: &Path) -> String {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(package.join("package.json")).unwrap()).unwrap();
    manifest["dupin"]["main"].as_str().unwrap_or("main.js").to_owned()
}

fn scopes(bridge: &Bridge, listed: &Listed) -> Vec<(String, ConsentState)> {
    bridge
        .scopes(&listed.package)
        .into_iter()
        .map(|entry| (entry.scope.key().to_string(), entry.state))
        .collect()
}

#[test]
fn every_bundled_package_has_an_override_test() {
    let mut names: Vec<_> = bundled::packages(&bundled::directory().unwrap())
        .into_iter()
        .map(|(_, name, _)| name)
        .collect();
    names.sort();
    assert_eq!(names, COVERED);
}

#[test]
fn calendar() {
    let on = Config::parse("[calendar]\nenabled = true").unwrap();
    Override::built_in("calendar", on, Some(Config::default())).run();
}

#[test]
fn dupin() {
    Override::extension("dupin", Config::default()).run();
}

#[test]
fn emoji() {
    Override::extension("emoji", Config::default()).run();
}
