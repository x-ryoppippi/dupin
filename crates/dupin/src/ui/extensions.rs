use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use dupin_core::{CommandId, ExtensionRequest};
use dupin_extension_bridge::{Bridge, Notice, ScopeKey, Trigger};
use dupin_js_runtime::RunError;
use rune::Tx;

use super::{Busy, Ui, search::spawn_searcher};
use crate::extensions::{ACTION_DEADLINE, Extension};

/// The extensions folder's latest catalogue while `extensions = true`, and which of its
/// extensions own a command and searcher now.
pub(super) struct Extensions {
    watch: Option<crate::extensions::Watch>,
    /// Identifies the current watch, so a scan from a stopped one is ignored.
    epoch: u64,
    catalog: Vec<Arc<crate::extensions::Extension>>,
    scan_problems: Vec<String>,
    registered: HashMap<CommandId, Arc<crate::extensions::Extension>>,
    /// Every guest call's host requests go through it.
    pub(super) bridge: Bridge,
}

impl Extensions {
    pub(super) fn new(bridge: Bridge) -> Self {
        Self {
            watch: None,
            epoch: 0,
            catalog: Vec::new(),
            scan_problems: Vec::new(),
            registered: HashMap::new(),
            bridge,
        }
    }
}

impl Ui {
    /// Start or stop reading the extensions folder. Stopping drops their commands at the next
    /// registry install, so no guest runs once the switch is off.
    pub(crate) fn watch_extensions(&mut self, enabled: bool) {
        if enabled == self.extensions.watch.is_some() {
            return;
        }
        self.extensions.epoch += 1;
        self.extensions.catalog.clear();
        self.extensions.scan_problems.clear();
        self.extensions.watch = None;
        if !enabled {
            return;
        }
        let Some(directory) = crate::extensions::directory() else {
            self.extensions
                .scan_problems
                .push("Couldn't find the configuration folder.".into());
            return;
        };
        let (me, epoch) = (self.me.clone(), self.extensions.epoch);
        match crate::extensions::watch(directory, move |catalog| {
            me.update(move |s, tx| s.extensions_scanned(epoch, catalog, tx));
        }) {
            Ok(watch) => self.extensions.watch = Some(watch),
            Err(error) => {
                let problem = format!("Couldn't start the extension scanner: {error}");
                crate::log::warn(&problem);
                self.extensions.scan_problems.push(problem);
            }
        }
    }

    fn extensions_scanned(&mut self, epoch: u64, catalog: crate::extensions::Catalog, tx: &mut Tx) {
        if epoch != self.extensions.epoch || self.extensions.watch.is_none() {
            return;
        }
        self.extensions.catalog = catalog.extensions;
        self.extensions.scan_problems = catalog.problems;
        self.install_declarations();
        let config = self.configuration.values.borrow().clone();
        self.apply_config(&config, tx);
    }

    pub(crate) fn install_extensions(
        &mut self,
        registration: &mut crate::commands::ConfiguredCommands,
    ) -> bool {
        let added = crate::features::extensions::add_extensions(registration, &self.extensions.catalog);
        let registered: HashMap<_, _> = added.registered.into_iter().collect();
        let active = self.session.command_id();
        let active_changed = registered.get(active).is_some_and(|next| {
            self.extensions
                .registered
                .get(active)
                .is_some_and(|current| !Arc::ptr_eq(next, current))
        });
        self.extensions.registered.retain(|id, extension| {
            let keep = registered
                .get(id)
                .is_some_and(|next| Arc::ptr_eq(next, extension));
            if !keep {
                self.searchers.remove(id);
                self.deadlines.remove(id);
            }
            keep
        });
        for (id, extension) in registered {
            if self.extensions.registered.contains_key(&id) {
                continue;
            }
            let provider =
                crate::features::extensions::provider(extension.clone(), self.extensions.bridge.clone());
            let searcher = spawn_searcher(self.me.clone(), id.clone(), provider);
            self.searchers.insert(id.clone(), searcher);
            self.extensions.registered.insert(id, extension);
        }
        let mut loaded: Vec<_> = self
            .extensions
            .registered
            .values()
            .map(|extension| format!("{} ({})", extension.name, extension.keyword))
            .collect();
        loaded.sort();
        let problems: Vec<_> = self
            .extensions
            .scan_problems
            .iter()
            .cloned()
            .chain(added.problems)
            .collect();
        let installed: Vec<_> = self
            .extensions
            .catalog
            .iter()
            .map(|extension| crate::prefs::InstalledExtension {
                package: extension.package.clone(),
                name: extension.name.clone(),
                keyword: extension.keyword.clone(),
            })
            .collect();
        let bridge = self.extensions.bridge.clone();
        self.prefs.update(move |prefs, _| {
            prefs.reflect_extensions(loaded, problems);
            prefs.reflect_extension_access(bridge, installed);
        });
        active_changed
    }

    /// Hands each loaded package's declaration to the broker, which never grants anything for
    /// it, and watches the clipboard for those that declare it.
    fn install_declarations(&mut self) {
        let bridge = self.extensions.bridge.clone();
        for extension in &self.extensions.catalog {
            let id = &extension.package;
            bridge.install(id, extension.revision.clone(), extension.capabilities.clone());
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
            let (watcher, bridge_for_watch) = (Arc::downgrade(extension), bridge.clone());
            let id_for_watch = id.clone();
            bridge.watch_clipboard(id, move |event| {
                let Some(extension) = watcher.upgrade() else {
                    return;
                };
                let call = bridge_for_watch.call(&id_for_watch, Trigger::Background);
                let deadline = Instant::now() + crate::extensions::QUERY_DEADLINE;
                if let Err(RunError::Failed(diagnostic)) =
                    extension.event("clipboard", &event, deadline, &call.services)
                {
                    crate::log::warn(&format!("extension {}: {diagnostic}", extension.id));
                }
            });
        }
    }

    /// Called from the bridge's listener: refresh what shows a package's access.
    pub(crate) fn extension_access_changed(&mut self, notice: Notice, tx: &mut Tx) {
        let (Notice::ConsentNeeded(id) | Notice::Changed(id)) = &notice;
        let active = self.session.command_id().clone();
        let showing = self
            .extensions
            .registered
            .get(&active)
            .is_some_and(|extension| extension.package == *id);
        if showing && matches!(notice, Notice::Changed(_)) {
            let effects = self.session.refresh();
            self.apply(effects, tx);
        }
        self.prefs.update(|prefs, _| prefs.refresh_permissions());
    }

    fn registered_extension(&self, id: &str) -> Option<Arc<Extension>> {
        self.extensions
            .registered
            .values()
            .find(|extension| extension.id == id)
            .cloned()
    }

    /// Runs a row's extension action, which the user chose: the only calls that carry an
    /// activation.
    pub(super) fn run_extension_request(
        &mut self,
        extension: String,
        request: ExtensionRequest,
        tx: &mut Tx,
    ) {
        let Some(extension) = self.registered_extension(&extension) else {
            return;
        };
        let ExtensionRequest::Run { action, data } = request else {
            self.answer_extension_row(None, extension, request, tx);
            return;
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
                    Ok(Some(dupin_extension_host::ActionEffect::Toast { text })) => {
                        ui.timed_notice(text, tx);
                        None
                    }
                    Ok(None) | Err(RunError::Cancelled) => None,
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
            ExtensionRequest::Run { .. } => return,
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
