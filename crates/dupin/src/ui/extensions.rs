use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use dupin_core::{CommandId, ExtensionRequest};
use dupin_extension_bridge::{Bridge, Notice, PackageId, ScopeKey, Trigger};
use dupin_js_runtime::{Acted, RunError};
use rune::Tx;

use super::{Busy, Ui, search::spawn_searcher};
use crate::extensions::packages::{Kind, Packages};
use crate::extensions::{ACTION_DEADLINE, Catalog, Extension, Origin, Role, Watch};
use crate::features::extensions::Port;

/// The bundled extensions, always, and the extensions folder while `extensions = true`; which
/// of them run; and which own a command and searcher now.
pub(super) struct Extensions {
    bundled: Folder,
    user: Folder,
    packages: Packages,
    /// What [`Packages::active`] returned at the last registry install, to tell whether a
    /// change of the user's choices needs another.
    active: Vec<Arc<Extension>>,
    registered: HashMap<CommandId, (Arc<Extension>, Role)>,
    /// The built-in commands bundled extensions answer, which keep their searcher and take
    /// the extension that runs now.
    ports: HashMap<CommandId, Arc<Port>>,
    /// Every guest call's host requests go through it.
    pub(super) bridge: Bridge,
}

#[derive(Default)]
struct Folder {
    watch: Option<Watch>,
    /// Identifies the current watch, so a scan from a stopped one is ignored.
    epoch: u64,
    problems: Vec<String>,
}

impl Extensions {
    pub(super) fn new(bridge: Bridge, ports: HashMap<CommandId, Arc<Port>>) -> Self {
        Self {
            bundled: Folder::default(),
            user: Folder::default(),
            packages: Packages::new(bridge.clone()),
            active: Vec::new(),
            registered: HashMap::new(),
            ports,
            bridge,
        }
    }

    /// Whether `current` lists `package`'s catalogue: its keyword, or the main search it
    /// answers too.
    fn lists_catalogue_of(&self, current: &CommandId, package: &PackageId) -> bool {
        lists_catalogue_of(
            self.registered
                .iter()
                .map(|(command, (extension, role))| (command, &extension.package, *role)),
            current,
            package,
        )
    }

    fn folder(&mut self, origin: Origin) -> &mut Folder {
        match origin {
            Origin::Bundled => &mut self.bundled,
            Origin::User => &mut self.user,
        }
    }
}

impl Ui {
    /// Start or stop reading the extensions folder, and read the bundled extensions on the first
    /// call. Stopping drops the folder's commands at the next registry install, so none of its
    /// guests runs once the switch is off.
    pub(crate) fn watch_extensions(&mut self, enabled: bool) {
        if self.extensions.bundled.epoch == 0 {
            self.start_extensions(Origin::Bundled, crate::extensions::bundled::directory(), None);
        }
        if enabled == self.extensions.user.watch.is_some() {
            return;
        }
        if enabled {
            let missing = "Couldn't find the configuration folder.";
            self.start_extensions(Origin::User, crate::extensions::directory(), Some(missing));
        } else {
            self.stop_extensions(Origin::User);
        }
    }

    fn stop_extensions(&mut self, origin: Origin) {
        let folder = self.extensions.folder(origin);
        folder.epoch += 1;
        folder.problems.clear();
        folder.watch = None;
        self.extensions.packages.set(origin, Vec::new());
        self.install_declarations();
    }

    fn start_extensions(
        &mut self,
        origin: Origin,
        directory: Option<std::path::PathBuf>,
        missing: Option<&str>,
    ) {
        self.stop_extensions(origin);
        let Some(directory) = directory else {
            self.extensions
                .folder(origin)
                .problems
                .extend(missing.map(str::to_owned));
            return;
        };
        let (me, epoch) = (self.me.clone(), self.extensions.folder(origin).epoch);
        match crate::extensions::watch(directory, origin, move |catalog| {
            me.update(move |s, tx| s.extensions_scanned(origin, epoch, catalog, tx));
        }) {
            Ok(watch) => {
                self.extensions.folder(origin).watch = Some(watch);
                if origin == Origin::Bundled {
                    self.extensions.packages.await_bundled();
                }
            }
            Err(error) => {
                let problem = format!("Couldn't start the extension scanner: {error}");
                crate::log::warn(&problem);
                self.extensions.folder(origin).problems.push(problem);
            }
        }
    }

    fn extensions_scanned(&mut self, origin: Origin, epoch: u64, catalog: Catalog, tx: &mut Tx) {
        let folder = self.extensions.folder(origin);
        if epoch != folder.epoch || folder.watch.is_none() {
            return;
        }
        folder.problems = catalog.problems;
        self.extensions.packages.set(origin, catalog.extensions);
        self.install_declarations();
        let config = self.configuration.values.borrow().clone();
        self.apply_config(&config, tx);
    }

    pub(crate) fn install_extensions(
        &mut self,
        registration: &mut crate::commands::ConfiguredCommands,
    ) -> bool {
        let active = self.extensions.packages.active();
        let added = crate::features::extensions::add_extensions(registration, &active);
        self.extensions.active = active;
        let registered: HashMap<_, _> = added
            .registered
            .into_iter()
            .map(|(id, extension, role)| (id, (extension, role)))
            .collect();
        let current = self.session.command_id();
        let answers = |registered: &HashMap<CommandId, (Arc<Extension>, Role)>| {
            registered
                .get(current)
                .map(|(extension, _)| Arc::as_ptr(extension))
        };
        let active_changed = answers(&registered) != answers(&self.extensions.registered)
            && (self.extensions.registered.contains_key(current)
                || self.extensions.ports.contains_key(current));
        let ports = &self.extensions.ports;
        self.extensions.registered.retain(|id, (extension, _)| {
            let keep = registered
                .get(id)
                .is_some_and(|(next, _)| Arc::ptr_eq(next, extension));
            if !keep {
                match ports.get(id) {
                    Some(port) => port.set(None),
                    None => {
                        self.searchers.remove(id);
                        self.deadlines.remove(id);
                    }
                }
            }
            keep
        });
        for (id, (extension, role)) in registered {
            if self.extensions.registered.contains_key(&id) {
                continue;
            }
            let provider = crate::features::extensions::provider(
                extension.clone(),
                role,
                self.extensions.bridge.clone(),
            );
            match self.extensions.ports.get(&id) {
                Some(port) => port.set(Some(provider)),
                None => {
                    let searcher = spawn_searcher(self.me.clone(), id.clone(), provider);
                    self.searchers.insert(id.clone(), searcher);
                }
            }
            self.extensions.registered.insert(id, (extension, role));
        }
        let routed: Vec<_> = self.extensions.registered.values().cloned().collect();
        // A feature's own switch turns its bundled extension on and off, so Settings lists only
        // a changed copy of one, which runs as third-party code.
        let listing = self
            .extensions
            .packages
            .listing(|extension| {
                routed
                    .iter()
                    .any(|(routed, role)| *role == Role::Keyword && std::ptr::eq(&**routed, extension))
            })
            .into_iter()
            .filter(|listed| {
                !(matches!(listed.kind, Kind::Bundled | Kind::Copy)
                    && listed
                        .package
                        .as_str()
                        .strip_prefix("bundled:")
                        .and_then(crate::features::ported_command)
                        .is_some())
            })
            .collect();
        let problems: Vec<_> = self
            .extensions
            .bundled
            .problems
            .iter()
            .chain(&self.extensions.user.problems)
            .cloned()
            .chain(added.problems)
            .collect();
        let bridge = self.extensions.bridge.clone();
        self.prefs.update(move |prefs, _| {
            prefs.reflect_extensions(listing, problems);
            prefs.reflect_extension_access(bridge);
        });
        active_changed
    }

    /// Watches the clipboard for each present package that declares it; [`Packages::set`]
    /// has already handed the broker every declaration, which never grants anything.
    fn install_declarations(&mut self) {
        let bridge = self.extensions.bridge.clone();
        for (_, extension) in self.extensions.packages.present() {
            let id = &extension.package;
            let watches = extension
                .capabilities
                .get(&ScopeKey::Clipboard(
                    dupin_extension_bridge::ClipboardAccess::Watch,
                ))
                .is_some();
            if !watches {
                bridge.unwatch_clipboard(id);
                continue;
            }
            let (watcher, bridge_for_watch) = (Arc::downgrade(&extension), bridge.clone());
            let id_for_watch = id.clone();
            let me = self.me.clone();
            bridge.watch_clipboard(id, move |event| {
                let Some(extension) = watcher.upgrade() else {
                    return;
                };
                let call = bridge_for_watch.call(&id_for_watch, Trigger::Background);
                let deadline = Instant::now() + crate::extensions::QUERY_DEADLINE;
                match extension.event("clipboard", &event, deadline, &call.services) {
                    // Only a search that lists this catalogue runs again: any other would run
                    // its package's `search` on this package's events.
                    Ok(handled) if handled.invalidated => {
                        let id = id_for_watch.clone();
                        me.update(move |ui, tx| {
                            let current = ui.session.command_id().clone();
                            if ui.shown && ui.extensions.lists_catalogue_of(&current, &id) {
                                let effects = ui.session.refresh();
                                ui.apply(effects, tx);
                            }
                        });
                    }
                    Ok(_) | Err(RunError::Cancelled) => {}
                    Err(RunError::Failed(diagnostic)) => {
                        crate::log::warn(&format!("extension {}: {diagnostic}", extension.id));
                    }
                }
            });
        }
    }

    /// Called from the bridge's listener: refresh what shows a package's access, and register
    /// the extensions again if turning one on or off changed which run. An answer takes its
    /// question off the panel but never searches again, since granting runs nothing the
    /// extension asked for before.
    pub(crate) fn extension_access_changed(&mut self, notice: Notice, tx: &mut Tx) {
        if let Notice::Changed(id) = &notice {
            // A grant changes what the catalogue may read even when no authority was lost, so
            // it is asked for again from the start on its next use.
            for (extension, _) in self.extensions.registered.values() {
                if extension.package == *id {
                    extension.forget_catalogue();
                }
            }
            let bridge = &self.extensions.bridge;
            let commands: Vec<_> = self
                .extensions
                .registered
                .iter()
                .filter(|(_, (extension, _))| extension.package == *id)
                .map(|(command, _)| command.clone())
                .collect();
            for command in commands {
                self.session.forget_rows(&command, |row| {
                    crate::extensions::is_answered_consent_row(row, bridge, id)
                });
            }
            let active = self.extensions.packages.active();
            let unchanged = active.len() == self.extensions.active.len()
                && active
                    .iter()
                    .zip(&self.extensions.active)
                    .all(|(now, before)| Arc::ptr_eq(now, before));
            if !unchanged {
                let config = self.configuration.values.borrow().clone();
                self.apply_config(&config, tx);
            }
        }
        self.prefs.update(|prefs, _| prefs.refresh_permissions());
    }

    /// The bridge every extension's host services go through.
    pub(crate) fn extension_bridge(&self) -> Bridge {
        self.extensions.bridge.clone()
    }

    /// The registered extensions whose catalogues answer the main search, in the order they
    /// registered, which is where web searches come from.
    pub(crate) fn web_search_sources(&self) -> Vec<Arc<Extension>> {
        self.extensions
            .active
            .iter()
            .filter(|extension| {
                self.extensions.registered.values().any(|(registered, role)| {
                    *role == Role::DefaultSearch && Arc::ptr_eq(registered, extension)
                })
            })
            .cloned()
            .collect()
    }

    fn registered_extension(&self, id: &str) -> Option<Arc<Extension>> {
        self.extensions
            .registered
            .values()
            .find(|(extension, _)| extension.id == id)
            .map(|(extension, _)| extension.clone())
    }

    /// Runs a row's extension action, which the user chose: the only calls that carry an
    /// activation. Searching again afterwards shows a catalogue the action invalidated.
    pub(super) fn run_extension_request(
        &mut self,
        extension: String,
        request: ExtensionRequest,
        tx: &mut Tx,
    ) {
        let Some(extension) = self.registered_extension(&extension) else {
            return;
        };
        let (action, data) = match request {
            ExtensionRequest::Run { action, data } => (action, data),
            ExtensionRequest::Paste { text } => return self.run_paste(&extension, text, tx),
            request => return self.answer_extension_row(None, extension, request, tx),
        };
        let bridge = self.extensions.bridge.clone();
        let me = self.me.clone();
        let busy = Busy::new(self.running.clone());
        std::thread::spawn(move || {
            let call = bridge.call(&extension.package, Trigger::Activation);
            let data = data.unwrap_or(serde_json::Value::Null);
            let result = extension.act(&action, &data, Instant::now() + ACTION_DEADLINE, &call.services);
            let stale = !bridge.is_current(&extension.package, call.generation);
            let name = extension.name.clone();
            me.update(move |ui, tx| {
                let notice = match result {
                    _ if stale => Some(format!(
                        "{name}'s access changed while it ran, so its result was dropped."
                    )),
                    Ok(Acted {
                        effect: Some(dupin_extension_host::ActionEffect::Toast { text }),
                        ..
                    }) => {
                        ui.timed_notice(text, tx);
                        None
                    }
                    Ok(Acted { effect: None, .. }) | Err(RunError::Cancelled) => None,
                    Err(RunError::Failed(diagnostic)) => {
                        crate::log::warn(&format!("extension {name}: {diagnostic}"));
                        Some(format!("{name} failed: {}", diagnostic.message))
                    }
                };
                if let Some(notice) = notice {
                    ui.show_notice(notice, tx);
                }
                // A refused request may have left a consent row to show.
                let effects = ui.session.refresh();
                ui.apply(effects, tx);
            });
            drop(busy);
        });
    }

    /// Answers a consent row: Allow, Deny, Request Access, Turn On or Open Settings. Answering
    /// runs nothing the extension asked for before.
    pub(super) fn answer_extension_row(
        &mut self,
        row: Option<String>,
        extension: Arc<Extension>,
        request: ExtensionRequest,
        tx: &mut Tx,
    ) {
        let id = extension.package.clone();
        let bridge = self.extensions.bridge.clone();
        let pending = |key: &str| {
            bridge
                .pending(Some(&id))
                .into_iter()
                .find(|request| request.key().to_string() == key)
        };
        let outcome = match request {
            ExtensionRequest::Run { .. } | ExtensionRequest::Paste { .. } => return,
            ExtensionRequest::Allow { scope, revision } => match pending(&scope) {
                Some(request) if request.revision.as_str() == revision => bridge
                    .allow(&id, &request.revision, &request.scope, None)
                    .map(|()| format!("Allowed {} to {}.", extension.name, request.description))
                    .map_err(|error| error.to_string()),
                _ => Err("That request changed; check Settings → Extensions.".into()),
            },
            ExtensionRequest::Deny { scope } => match scope.parse::<ScopeKey>() {
                Ok(key) => bridge
                    .revoke(&id, &key)
                    .map(|()| "Denied. Allow it again in Settings → Extensions.".to_owned())
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            },
            ExtensionRequest::RequestAccess { scope } => {
                match pending(&scope).and_then(|request| request.permission) {
                    Some(dupin_extension_bridge::OsPermission::Calendars) => {
                        self.calendar_access.request();
                        Ok(String::new())
                    }
                    Some(dupin_extension_bridge::OsPermission::Pasteboard) => {
                        self.request_clipboard_access();
                        Ok(String::new())
                    }
                    Some(dupin_extension_bridge::OsPermission::Accessibility) => {
                        crate::extensions::access::request_accessibility();
                        Ok(String::new())
                    }
                    None => Ok(String::new()),
                }
            }
            ExtensionRequest::TurnOn => bridge
                .set_enabled(&id, true)
                .map(|()| format!("Turned on {}.", extension.name))
                .map_err(|error| error.to_string()),
            ExtensionRequest::Review => {
                self.prefs.update(|prefs, _| prefs.show_extensions());
                self.prepare_settings();
                self.hide(tx);
                return;
            }
        };
        if let Some(row) = row {
            self.session.forget(&row);
        }
        match outcome {
            Ok(text) if text.is_empty() => {}
            Ok(text) => self.timed_notice(text, tx),
            Err(error) => self.show_notice(error, tx),
        }
    }

    /// Answers a consuming consent row by its result key.
    pub(super) fn answer_extension_consent(
        &mut self,
        row: String,
        extension: String,
        request: ExtensionRequest,
        tx: &mut Tx,
    ) {
        if let Some(extension) = self.registered_extension(&extension) {
            self.answer_extension_row(Some(row), extension, request, tx);
        }
    }
}

/// Whether `current` lists `package`'s catalogue, given which extension and role each command
/// routes to: its keyword does, and so does the main search while it answers there.
fn lists_catalogue_of<'a>(
    registered: impl IntoIterator<Item = (&'a CommandId, &'a PackageId, Role)>,
    current: &CommandId,
    package: &PackageId,
) -> bool {
    let main = current.as_str() == crate::features::apps::APPS;
    registered.into_iter().any(|(command, routed, role)| {
        routed == package
            && match role {
                Role::Keyword => command == current,
                Role::DefaultSearch => main,
            }
    })
}

#[cfg(test)]
#[test]
fn only_a_search_that_lists_the_catalogue_runs_again_for_it() {
    let command = |id: &str| CommandId::new(id).unwrap();
    let (clips, web) = (PackageId::new("clips").unwrap(), PackageId::new("web").unwrap());
    let (clips_keyword, web_keyword, apps) = (
        command("extension.clips"),
        command("extension.web"),
        command(crate::features::apps::APPS),
    );
    let clips_search = command("extension-search.clips");
    let keywords_only = [
        (&clips_keyword, &clips, Role::Keyword),
        (&web_keyword, &web, Role::Keyword),
    ];
    assert!(lists_catalogue_of(keywords_only, &clips_keyword, &clips));
    assert!(
        !lists_catalogue_of(keywords_only, &web_keyword, &clips),
        "another extension's keyword search never runs on this one's events"
    );
    assert!(!lists_catalogue_of(keywords_only, &apps, &clips));
    let in_main_search = [
        (&clips_keyword, &clips, Role::Keyword),
        (&clips_search, &clips, Role::DefaultSearch),
        (&web_keyword, &web, Role::Keyword),
    ];
    assert!(lists_catalogue_of(in_main_search, &apps, &clips));
    assert!(!lists_catalogue_of(in_main_search, &apps, &web));
}
