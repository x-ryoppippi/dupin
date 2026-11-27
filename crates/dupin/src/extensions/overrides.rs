//! Every bundled extension, built-in commands' included, follows one override rule: a copy in
//! the extensions folder with its `name` answers in its place, is the bundled package while it
//! is unchanged, loses the bundled grants once edited, and deleting it, or turning the
//! extensions folder off, brings the bundled one back. One that ships off answers only once it
//! is turned on, and then follows the same rule. The test takes each package in
//! `extensions/` through `dupin ext eject`, the folder scans, the launcher's registry install
//! with its ports and searchers, Settings' listing, the broker and the extension host.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dupin_core::{CommandId, Config, Provider};
use dupin_extension_bridge::services::{self, Services};
use dupin_extension_bridge::{
    Bridge, ClipboardAccess, ConsentState, Failure, OsAccess, OsPermission, PackageId, Request, Scope,
    StorageOp, Trigger, WindowsAccess,
};
use dupin_host_services::calendar;
use dupin_storage::{Limits, MemoryKey, NewRecord, Storage};
use serde_json::{Value, json};

use super::packages::{Kind, Listed};
use super::{Origin, Scanner, bundled};
use crate::commands::ConfiguredCommands;
use crate::features::extensions::Port;
use crate::ui::Extensions;

/// Something each package works on: the calculator only ever sees a calculation.
const QUERY: &str = "1+1 override";
const EDITED: &str = "export function search(query: string) { return [{ title: `edited: ${query}` }]; }\n";
const EDITED_AGAIN: &str = "export function search() { return [{ title: 'another copy' }]; }\n";
/// The keyword an edited copy's `package.json` asks for.
const EDITED_KEYWORD: &str = "overridden";

/// The case for each package in `extensions/`; one without a case fails the test.
fn case(name: &str) -> Option<Override> {
    Some(match name {
        "calendar" => Override::built_in(
            "calendar",
            Config::parse("[calendar]\nenabled = true").unwrap(),
            Some(Config::default()),
            Some(Use::Search),
        ),
        "clipboard-history" => Override::built_in(
            "clipboard-history",
            Config::parse("[clipboard]\nenabled = true").unwrap(),
            Some(Config::default()),
            Some(Use::Effect(Request::Clipboard(ClipboardAccess::Paste))),
        ),
        "calculator" => Override::built_in(
            "calculator",
            Config::default(),
            Some(Config::parse("calculator = false").unwrap()),
            Some(Use::Search),
        ),
        "dictionary" => Override::built_in("dictionary", Config::default(), None, Some(Use::Search)),
        "dupin" => Override::extension("dupin", None),
        "emoji" => Override::extension(
            "emoji",
            Some(Use::Effect(Request::Clipboard(ClipboardAccess::Paste))),
        ),
        // Writing the user's stored snippets is the most a copy could reach, so it is the use
        // an edited copy must be allowed again; pasting follows the same grants.
        "snippets" => Override::built_in(
            "snippets",
            Config::default(),
            None,
            Some(Use::Effect(Request::SnippetsWrite)),
        ),
        // Its files and exec scopes are bound to paths, so only its storage ships granted.
        "repo" => Override::extension(
            "repo",
            Some(Use::Effect(Request::Storage { op: StorageOp::Read })),
        )
        .ships_off(),
        "web-search" => Override::extension("web-search", None),
        "proc" => Override::signals("proc"),
        "port" => Override::signals("port"),
        "agent" => Override::signals("agent"),
        "window-management" => Override::extension(
            "window-management",
            Some(Use::Effect(Request::Windows(WindowsAccess::Arrange))),
        )
        .ships_off(),
        _ => return None,
    })
}

#[test]
fn every_bundled_package_follows_the_override_rule() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let names: Vec<_> = bundled::packages(&bundled::directory().unwrap())
        .into_iter()
        .map(|(_, name, _)| name)
        .collect();
    assert!(!names.is_empty());
    for name in names {
        let case = case(&name).unwrap_or_else(|| panic!("{name} has no override case"));
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| case.run())).is_err() {
            panic!("{name} doesn't follow the override rule");
        }
    }
}

/// How a package uses what it declares.
enum Use {
    /// Its search reads through a host service.
    Search,
    /// Dupin performs the request for it when its row's action is chosen, as pasting does.
    Effect(Request),
}

struct Override {
    name: &'static str,
    /// The command that answers the package's keyword.
    command: CommandId,
    /// The package answers a built-in command, through its port.
    built_in: bool,
    /// The configuration under which that command answers.
    on: Config,
    /// One that turns the built-in command off, if a setting can.
    off: Option<Config>,
    uses: Option<Use>,
    /// It waits for the user to turn it on in Settings → Extensions.
    ships_off: bool,
}

impl Override {
    fn extension(name: &'static str, uses: Option<Use>) -> Self {
        Self {
            name,
            command: CommandId::new(format!("extension.{name}")).unwrap(),
            built_in: false,
            on: Config::default(),
            off: None,
            uses,
            ships_off: false,
        }
    }

    /// A package that ships off and only runs programs, each of which the user allows before
    /// it runs, bundled or not, so it holds nothing as it ships.
    fn signals(name: &'static str) -> Self {
        Self::extension(name, None).ships_off()
    }

    fn ships_off(self) -> Self {
        Self {
            ships_off: true,
            ..self
        }
    }

    fn built_in(name: &'static str, on: Config, off: Option<Config>, uses: Option<Use>) -> Self {
        let command = crate::features::ported_command(name)
            .unwrap_or_else(|| panic!("no built-in command is answered by {name}"));
        Self {
            name,
            command: CommandId::new(command).unwrap(),
            built_in: true,
            on,
            off,
            uses,
            ships_off: false,
        }
    }

    fn run(&self) {
        let shipped = bundled::directory().unwrap();
        let grants = tempfile::tempdir().unwrap();
        let storage_root = grants.path().join("storage");
        let storage = Arc::new(Storage::new(&storage_root, MemoryKey::random()));
        let reached = Arc::new(AtomicUsize::new(0));
        let bridge = Bridge::open(
            &grants.path().join("base.toml"),
            Box::new(super::grants::Grants::new(Some(storage.clone()))),
            services(reached.clone()),
        );
        bridge.open_grants();
        for permission in OsPermission::ALL {
            bridge.set_os_access(permission, OsAccess::Allowed);
        }
        let ports: HashMap<_, _> = self
            .built_in
            .then(|| (self.command.clone(), Arc::new(Port::default())))
            .into_iter()
            .collect();
        let mut launcher = Launcher {
            extensions: Extensions::new(bridge.clone(), ports.clone()),
            ports,
            searchers: HashMap::new(),
            listing: Vec::new(),
        };
        let seals = bundled::Seals::shipped();
        let mut bundled_scan = Scanner {
            integrity: Some(Box::new(move |folder: &Path| seals.check(folder))),
            bundled: true,
            ..Scanner::default()
        };
        let catalog = bundled_scan.scan(&shipped);
        assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
        launcher.extensions.scanned(Origin::Bundled, catalog);
        let user = tempfile::tempdir().unwrap();
        let mut user_scan = Scanner::default();
        let mut rescan = |launcher: &mut Launcher| {
            let catalog = user_scan.scan(user.path());
            assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
            launcher.extensions.scanned(Origin::User, catalog);
        };
        rescan(&mut launcher);
        // The bridge holds the grants as the running launcher does, so ejecting asks it.
        let running = |command| super::grants::answer(&bridge, &command);
        let eject = |launcher: crate::extension_install::Launcher<'_>| {
            crate::extension_install::eject_into(&shipped, user.path(), Some(&storage), self.name, launcher)
        };
        let may_use = |launcher: &Launcher, package: &PackageId| match &self.uses {
            None => true,
            Some(Use::Search) => {
                reached.store(0, Ordering::SeqCst);
                let _ = launcher.search(&self.command);
                reached.load(Ordering::SeqCst) > 0
            }
            Some(Use::Effect(request)) => bridge.authorize(package, request, Trigger::Activation).is_ok(),
        };

        launcher.install(&self.on);
        if self.ships_off {
            let off = self.listed(&launcher);
            assert!(!off.registered, "it doesn't answer until turned on");
            assert!(launcher.answering(&self.command).is_none());
            assert!(!bridge.is_enabled(&off.package));
            if let Some(Use::Effect(request)) = &self.uses {
                assert!(
                    bridge
                        .authorize(&off.package, request, Trigger::Activation)
                        .is_err(),
                    "it can use nothing while off"
                );
            }
            bridge.set_enabled(&off.package, true).unwrap();
            launcher.install(&self.on);
        }
        let original = self.listed(&launcher);
        assert_eq!(original.kind, Kind::Bundled);
        assert_eq!(original.package.as_str(), format!("bundled:{}", self.name));
        assert_eq!(original.built_in, self.built_in);
        assert!(original.registered, "Settings says it answers");
        let shipped_answer = launcher.search(&self.command).expect("the bundled one answers");
        let shipped_scopes = scopes(&bridge, &original);
        assert!(
            may_use(&launcher, &original.package),
            "it holds the bundled grants"
        );
        if let Some(off) = &self.off {
            launcher.install(off);
            assert!(
                !self.listed(&launcher).registered,
                "Settings says the built-in's setting is off"
            );
            assert!(launcher.answering(&self.command).is_none());
            launcher.install(&self.on);
        }

        let copy = eject(&running).unwrap();
        rescan(&mut launcher);
        launcher.install(&self.on);
        let unchanged = self.listed(&launcher);
        assert_eq!(
            (unchanged.kind, &unchanged.package),
            (Kind::Copy, &original.package),
            "an unchanged copy is the bundled package"
        );
        assert_eq!(scopes(&bridge, &unchanged), shipped_scopes);
        assert_eq!(launcher.search(&self.command), Some(shipped_answer.clone()));
        assert!(may_use(&launcher, &unchanged.package));

        // The same code under another revision: the bundled copy's calls are its own, the
        // changed copy's identical ones are not.
        let main = copy.join(entry(&copy));
        let code = std::fs::read_to_string(&main).unwrap();
        std::fs::write(&main, format!("{code}\n// changed\n")).unwrap();
        ask_for_keyword(&copy, EDITED_KEYWORD);
        rescan(&mut launcher);
        let configured = launcher.install(&self.on);
        let modified = self.listed(&launcher);
        assert_eq!(modified.kind, Kind::Modified);
        assert_eq!(modified.package.as_str(), self.name, "it has grants of its own");
        assert_eq!(modified.built_in, self.built_in);
        assert!(modified.registered);
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
        if self.built_in {
            assert_eq!(
                configured.registry.keyword(EDITED_KEYWORD),
                None,
                "a built-in copy's own keyword is ignored"
            );
            // A built-in in the main search has no keyword, and its package declares none.
            let expected = (crate::features::ported_role(self.name) == super::Role::Keyword)
                .then_some(Some(&self.command));
            assert_eq!(
                original
                    .keyword
                    .as_deref()
                    .map(|keyword| configured.registry.keyword(keyword)),
                expected,
                "it declares the keyword the built-in answers, and only then"
            );
        } else {
            assert_eq!(configured.registry.keyword(EDITED_KEYWORD), Some(&self.command));
        }
        if self.uses.is_some() {
            assert!(
                !may_use(&launcher, &modified.package),
                "not until it is turned on"
            );
            bridge.set_enabled(&modified.package, true).unwrap();
            assert!(!may_use(&launcher, &modified.package), "not without consent");
            // Each scope is asked for when it is first needed: what is typed before what is
            // worked out with it.
            loop {
                let pending = bridge.pending(Some(&modified.package));
                assert!(!pending.is_empty(), "it asks for consent");
                for request in pending {
                    bridge
                        .allow(&modified.package, &request.revision, &request.scope, None)
                        .unwrap();
                }
                if may_use(&launcher, &modified.package) {
                    break;
                }
            }
        } else {
            bridge.set_enabled(&modified.package, true).unwrap();
        }
        let held = scopes(&bridge, &modified);
        store_data(&storage, self.name);

        std::fs::write(&main, EDITED).unwrap();
        rescan(&mut launcher);
        launcher.install(&self.on);
        let edited = || Some(vec![format!("edited: {QUERY}")]);
        assert_eq!(
            launcher.search(&self.command),
            edited(),
            "the user's copy answers, edited"
        );
        if let Some(off) = &self.off {
            launcher.install(off);
            assert!(
                launcher.answering(&self.command).is_none(),
                "the built-in's own setting still turns the copy off"
            );
            launcher.install(&self.on);
            assert_eq!(launcher.search(&self.command), edited());
        }

        bridge.set_enabled(&modified.package, false).unwrap();
        launcher.install(&self.on);
        assert!(
            launcher.answering(&self.command).is_none(),
            "turning the copy off doesn't bring the bundled one back"
        );
        assert!(!self.listed(&launcher).registered);
        bridge.set_enabled(&modified.package, true).unwrap();
        launcher.install(&self.on);
        assert_eq!(launcher.search(&self.command), edited());

        launcher.extensions.stop(Origin::User);
        launcher.install(&self.on);
        self.restored(&launcher, &bridge, &original, &shipped_scopes, &shipped_answer);
        assert!(may_use(&launcher, &original.package));
        rescan(&mut launcher);
        launcher.install(&self.on);
        assert_eq!(launcher.search(&self.command), edited(), "the folder is on again");

        std::fs::remove_dir_all(&copy).unwrap();
        rescan(&mut launcher);
        launcher.install(&self.on);
        self.restored(&launcher, &bridge, &original, &shipped_scopes, &shipped_answer);
        assert!(may_use(&launcher, &original.package));

        // Copied by hand rather than ejected, another revision is the earlier copy edited.
        let pasted = bundled::eject(&shipped, user.path(), self.name).unwrap();
        std::fs::write(pasted.join(entry(&pasted)), EDITED_AGAIN).unwrap();
        rescan(&mut launcher);
        launcher.install(&self.on);
        let pasted_listed = self.listed(&launcher);
        assert_eq!(pasted_listed.kind, Kind::Modified);
        assert!(
            bridge.is_enabled(&pasted_listed.package),
            "a copy pasted by hand keeps the earlier copy's switch"
        );
        assert_eq!(
            scopes(&bridge, &pasted_listed),
            held,
            "and the grants it doesn't widen"
        );
        std::fs::remove_dir_all(&pasted).unwrap();
        rescan(&mut launcher);
        launcher.install(&self.on);

        let refused = eject(&|_| Err("Dupin is not running".into())).unwrap_err();
        assert_eq!(refused.code, "grants-unsaved");
        assert!(
            user.path().join(self.name).symlink_metadata().is_err(),
            "a copy that can't start afresh is removed"
        );
        assert!(
            storage_root.join(namespace(self.name)).exists(),
            "nothing it stored is deleted before its grants are forgotten"
        );

        let copy = eject(&running).unwrap();
        assert!(
            !storage_root.join(namespace(self.name)).exists(),
            "what the earlier copy stored is deleted"
        );

        std::fs::write(copy.join(entry(&copy)), EDITED_AGAIN).unwrap();
        rescan(&mut launcher);
        launcher.install(&self.on);
        let another = self.listed(&launcher);
        assert_eq!(another.kind, Kind::Modified);
        assert!(
            !bridge.is_enabled(&another.package),
            "a copy ejected again waits to be turned on"
        );
        assert!(
            scopes(&bridge, &another)
                .iter()
                .all(|(_, state)| *state == ConsentState::Undecided),
            "nothing allowed for the earlier copy passes to it"
        );
        // A keyword's copy answers before it may use anything; one in the main search sees
        // nothing typed there until it is turned on and allowed `query`.
        let answer = if crate::features::ported_role(self.name) == super::Role::MainSearch {
            Vec::new()
        } else {
            vec!["another copy".to_owned()]
        };
        assert_eq!(launcher.search(&self.command), Some(answer));
    }

    /// The bundled package answers as it did before it was ejected, with its grants.
    fn restored(
        &self,
        launcher: &Launcher,
        bridge: &Bridge,
        original: &Listed,
        shipped_scopes: &[(String, ConsentState)],
        shipped_answer: &[String],
    ) {
        assert_eq!(&self.listed(launcher), original);
        assert_eq!(scopes(bridge, original), shipped_scopes, "its grants are back");
        assert_eq!(
            launcher.search(&self.command).as_deref(),
            Some(shipped_answer),
            "the bundled one answers again"
        );
    }

    /// The package as Settings → Extensions lists it, which it always does.
    fn listed(&self, launcher: &Launcher) -> Listed {
        launcher
            .listing
            .iter()
            .find(|listed| listed.package.as_str().trim_start_matches("bundled:") == self.name)
            .cloned()
            .unwrap_or_else(|| panic!("Settings doesn't list {}", self.name))
    }
}

/// The launcher's side of a registry install: the built-ins' ports, the searchers it spawns
/// and drops, and what Settings lists.
struct Launcher {
    extensions: Extensions,
    ports: HashMap<CommandId, Arc<Port>>,
    searchers: HashMap<CommandId, Arc<dyn Provider>>,
    listing: Vec<Listed>,
}

impl Launcher {
    fn install(&mut self, config: &Config) -> ConfiguredCommands {
        let mut configured = crate::commands::configured(config).unwrap();
        let apps = CommandId::new(crate::features::apps::APPS).unwrap();
        let installed = self.extensions.install(&mut configured, &apps);
        for id in &installed.dropped {
            self.searchers.remove(id);
        }
        self.searchers.extend(installed.searchers);
        self.listing = installed.listing;
        configured
    }

    /// What answers `command` now: the extension in its built-in's port, or its searcher's.
    fn answering(&self, command: &CommandId) -> Option<Arc<dyn Provider>> {
        match self.ports.get(command) {
            Some(port) => port.provider(),
            None => self.searchers.get(command).cloned(),
        }
    }

    /// The titles `command` answers for [`QUERY`], if anything answers it and its search ran.
    fn search(&self, command: &CommandId) -> Option<Vec<String>> {
        let mut rows = Vec::new();
        self.answering(command)?
            .search(&dupin_core::Request::detached(QUERY, 10), &mut |batch| {
                rows.extend(batch)
            })
            .ok()?;
        Some(rows.iter().map(|row| row.display_title().to_owned()).collect())
    }
}

fn entry(package: &Path) -> String {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(package.join("package.json")).unwrap()).unwrap();
    manifest["dupin"]["main"].as_str().unwrap_or("main.js").to_owned()
}

fn ask_for_keyword(package: &Path, keyword: &str) {
    let path = package.join("package.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["dupin"]["keyword"] = keyword.into();
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

fn namespace(name: &str) -> String {
    dupin_extension_bridge::storage::namespace(
        &PackageId::new(name).unwrap(),
        dupin_extension_bridge::Origin::User,
    )
}

fn store_data(storage: &Storage, name: &str) {
    storage
        .open(&namespace(name), Limits::default())
        .unwrap()
        .put(NewRecord {
            data: json!({ "stored by": "an earlier copy" }),
            blobs: Vec::new(),
        })
        .unwrap();
}

fn scopes(bridge: &Bridge, listed: &Listed) -> Vec<(String, ConsentState)> {
    bridge
        .scopes(&listed.package)
        .into_iter()
        .map(|entry| (entry.scope.key().to_string(), entry.state))
        .collect()
}

/// Host services that answer every read with nothing and count the reads that reach them,
/// which only the broker's permission lets through.
fn services(reached: Arc<AtomicUsize>) -> Services {
    Services {
        dictionary: Box::new(Counted(reached.clone())),
        calculator: Box::new(Counted(reached.clone())),
        calendar: Box::new(Counted(reached)),
        ..Services::none()
    }
}

struct Counted(Arc<AtomicUsize>);

impl services::Calendar for Counted {
    fn window(&self, _from: i64, _to: i64) -> Result<(u32, u32), Failure> {
        Ok((0, 0))
    }

    fn events(
        &self,
        _from: i64,
        _to: i64,
        _query: Option<&str>,
        _limit: usize,
        _order: calendar::Order,
        _scope: &Scope,
    ) -> Result<Value, Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Array(Vec::new()))
    }
}

impl services::Calculator for Counted {
    fn evaluate(
        &self,
        _expression: &str,
        _cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Value, Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Null)
    }
}

impl services::Dictionary for Counted {
    fn lookup(&self, _word: &str) -> Result<Value, Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Array(Vec::new()))
    }
}
