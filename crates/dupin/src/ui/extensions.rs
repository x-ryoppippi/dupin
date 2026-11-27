use std::collections::HashMap;
use std::sync::Arc;

use dupin_core::{Anchor, CommandId, ExtensionRequest, Navigated, Page, Provider};
use dupin_extension_bridge::{Bridge, Notice, PackageId, ScopeKey, Trigger};
use dupin_extension_host::ActionEffect;
use dupin_js_runtime::{Acted, PageContext, RunError};
use rune::Tx;

use super::{Busy, Ui, search::spawn_searcher};
use crate::extensions::packages::{Listed, Packages};
use crate::extensions::{ACTION_DEADLINE, Catalog, Extension, Origin, Role, Watch};
use crate::features::extensions::Port;

/// The bundled extensions, always, and the extensions folder while `extensions = true`; which
/// of them run; and which own a command and searcher now.
pub(crate) struct Extensions {
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
    /// Whether the current watch's first scan has landed.
    scanned: bool,
    problems: Vec<String>,
}

impl Folder {
    /// Whether its packages are known: it was scanned, or there is nothing to wait for. The
    /// bundled folder is always read, so it waits until its first start.
    fn settled(&self, always_read: bool) -> bool {
        self.scanned || (self.watch.is_none() && (!always_read || self.epoch > 0))
    }
}

impl Extensions {
    pub(crate) fn new(bridge: Bridge, ports: HashMap<CommandId, Arc<Port>>) -> Self {
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

    /// Build every present package's catalogue rows again on next use, as after the kinds
    /// their text and URLs resolve through changed.
    pub(super) fn forget_catalogues(&self) {
        for (_, extension) in self.packages.present() {
            extension.forget_catalogue();
        }
    }

    /// The package that answers `command`, if an extension does.
    pub(super) fn package(&self, command: &CommandId) -> Option<&str> {
        self.registered
            .get(command)
            .map(|(extension, _)| extension.package.as_str())
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

    /// Whether every folder Dupin reads has been scanned, so a command no extension answers
    /// now won't appear without a change.
    pub(super) fn settled(&self) -> bool {
        self.bundled.settled(true) && self.user.settled(false)
    }

    /// Takes a folder's latest scan.
    pub(crate) fn scanned(&mut self, origin: Origin, catalog: Catalog) {
        self.folder(origin).scanned = true;
        self.folder(origin).problems = catalog.problems;
        self.packages.set(origin, catalog.extensions);
    }

    /// Forgets a folder's packages and stops reading it, as turning the extensions folder off
    /// does, so a bundled extension a copy there replaced answers again.
    pub(crate) fn stop(&mut self, origin: Origin) {
        let folder = self.folder(origin);
        folder.epoch += 1;
        folder.scanned = false;
        folder.problems.clear();
        folder.watch = None;
        self.packages.set(origin, Vec::new());
    }

    /// Registers the extensions that run now in `registration`, hands each command's extension
    /// to its built-in's port or back for a searcher, and takes it back from a command it no
    /// longer answers. `current` is the command on screen.
    pub(crate) fn install(
        &mut self,
        registration: &mut crate::commands::ConfiguredCommands,
        current: &CommandId,
    ) -> Installed {
        let active = self.packages.active();
        let added = crate::features::extensions::add_extensions(registration, &active);
        self.active = active;
        let registered: HashMap<_, _> = added
            .registered
            .into_iter()
            .map(|(id, extension, role)| (id, (extension, role)))
            .collect();
        let answers = |registered: &HashMap<CommandId, (Arc<Extension>, Role)>| {
            registered
                .get(current)
                .map(|(extension, _)| Arc::as_ptr(extension))
        };
        let current_changed = answers(&registered) != answers(&self.registered)
            && (self.registered.contains_key(current) || self.ports.contains_key(current));
        let ports = &self.ports;
        let mut dropped = Vec::new();
        self.registered.retain(|id, (extension, _)| {
            let keep = registered
                .get(id)
                .is_some_and(|(next, _)| Arc::ptr_eq(next, extension));
            if !keep {
                match ports.get(id) {
                    Some(port) => port.set(None),
                    None => dropped.push(id.clone()),
                }
            }
            keep
        });
        let mut searchers = Vec::new();
        for (id, (extension, role)) in registered {
            if self.registered.contains_key(&id) {
                continue;
            }
            let provider =
                crate::features::extensions::provider(extension.clone(), role, self.bridge.clone());
            if let Some(provider) = crate::features::extensions::hand_over(&self.ports, &id, provider) {
                searchers.push((id.clone(), provider));
            }
            self.registered.insert(id, (extension, role));
        }
        let routed: Vec<_> = self.registered.values().cloned().collect();
        let listing = self.packages.listing(|extension| {
            routed.iter().any(|(routed, role)| {
                matches!(role, Role::Keyword | Role::MainSearch) && std::ptr::eq(&**routed, extension)
            })
        });
        let problems = self
            .bundled
            .problems
            .iter()
            .chain(&self.user.problems)
            .cloned()
            .chain(added.problems)
            .collect();
        Installed {
            dropped,
            searchers,
            listing,
            problems,
            current_changed,
        }
    }
}

/// What a registry install changed, for the launcher to carry out on its searchers and show in
/// Settings.
pub(crate) struct Installed {
    /// Commands whose extension no longer answers them, so their searcher goes.
    pub dropped: Vec<CommandId>,
    /// Commands that need a searcher of their own, over the extension that answers them now.
    pub searchers: Vec<(CommandId, Arc<dyn Provider>)>,
    /// Every present extension, as Settings → Extensions lists it.
    pub listing: Vec<Listed>,
    pub problems: Vec<String>,
    /// Whether another extension, or none, now answers the command on screen.
    pub current_changed: bool,
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
        self.extensions.stop(origin);
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
        self.extensions.scanned(origin, catalog);
        self.install_declarations();
        let config = self.configuration.values.borrow().clone();
        self.apply_config(&config, tx);
    }

    pub(crate) fn install_extensions(
        &mut self,
        registration: &mut crate::commands::ConfiguredCommands,
    ) -> bool {
        let current = self.session.command_id().clone();
        let installed = self.extensions.install(registration, &current);
        self.composition
            .guests()
            .set_extensions(self.extensions.active.clone());
        for id in &installed.dropped {
            self.searchers.remove(id);
            self.deadlines.remove(id);
        }
        for (id, provider) in installed.searchers {
            let searcher = spawn_searcher(self.me.clone(), id.clone(), provider);
            self.searchers.insert(id, searcher);
        }
        let (listing, problems) = (installed.listing, installed.problems);
        let bridge = self.extensions.bridge.clone();
        self.prefs.update(move |prefs, _| {
            prefs.reflect_extensions(listing, problems);
            prefs.reflect_extension_access(bridge);
        });
        installed.current_changed
    }

    /// Watches the clipboard for each present package that declares it; [`Packages::set`]
    /// has already handed the broker every declaration, which never grants anything.
    pub(super) fn install_declarations(&mut self) {
        let me = std::sync::Mutex::new(self.me.clone());
        crate::extensions::on_catalogue_refreshed(move |id| {
            let id = id.clone();
            me.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .update(move |ui, tx| ui.search_again_if_listed(&id, false, tx));
        });
        let bridge = self.extensions.bridge.clone();
        let config = self.configuration.values.borrow().clone();
        for (_, extension) in self.extensions.packages.present() {
            let id = &extension.package;
            let watches = extension
                .capabilities
                .get(&ScopeKey::Clipboard(
                    dupin_extension_bridge::ClipboardAccess::Watch,
                ))
                .is_some();
            let feature_off = crate::features::ported_enabled(&extension.id, &config) == Some(false);
            if !watches || feature_off {
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
                let deadline = crate::extensions::deadline_in(crate::extensions::QUERY_DEADLINE);
                match extension.event("clipboard", &event, deadline, &call.services) {
                    // Only a search that lists this catalogue runs again: any other would run
                    // its package's `search` on this package's events.
                    Ok(handled) if handled.invalidated => {
                        let id = id_for_watch.clone();
                        me.update(move |ui, tx| ui.search_again_if_listed(&id, true, tx));
                    }
                    Ok(_) | Err(RunError::Cancelled) => {}
                    Err(RunError::Failed(diagnostic)) => {
                        crate::log::warn(&format!("extension {}: {}", extension.id, diagnostic.for_log()));
                    }
                }
            });
        }
    }

    /// Searches again when the panel shows the package's catalogue, after something other than
    /// typing changed it. Only a search that lists the catalogue runs again: any other would run
    /// its package's `search` for this package's change. With `over_typing` false, as for a
    /// refresh nobody asked for, a search already running is left alone: it reads the catalogue
    /// itself, and replacing it with the same query would only delay what was typed.
    fn search_again_if_listed(&mut self, id: &PackageId, over_typing: bool, tx: &mut Tx) {
        let current = self.session.command_id().clone();
        let searching = matches!(
            self.session.results(),
            dupin_core::session::Results::Searching { .. }
        );
        if self.on_screen() && (over_typing || !searching) && self.extensions.lists_catalogue_of(&current, id)
        {
            let effects = self.session.refresh();
            self.apply(effects, tx);
        }
    }

    /// Called from the bridge's listener: refresh what shows a package's access, and register
    /// the extensions again if turning one on or off changed which run. An answer takes its
    /// question off the panel but never searches again, since granting runs nothing the
    /// extension asked for before.
    pub(crate) fn extension_access_changed(&mut self, notice: Notice, tx: &mut Tx) {
        if let Notice::Changed(id) = &notice {
            self.resume_bound_row(id, tx);
            // Images refused before may be drawn now.
            self.thumbnails.retry_failed();
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

    /// The values `package`'s preferences hold changed, so its catalogue is listed again with
    /// them on its next use.
    pub(crate) fn preferences_changed(&mut self, package: &str) {
        for (extension, _) in self.extensions.registered.values() {
            if extension.id == package {
                extension.forget_catalogue();
            }
        }
    }

    pub(super) fn registered_extension(&self, id: &str) -> Option<Arc<Extension>> {
        self.extensions
            .registered
            .values()
            .find(|(extension, _)| extension.id == id)
            .map(|(extension, _)| extension.clone())
    }

    /// The package of the extension whose rows name images by `id`, whose storage alone they
    /// are read from.
    pub(super) fn extension_package(&self, id: &str) -> Option<PackageId> {
        self.extensions
            .registered
            .values()
            .find(|(extension, _)| extension.id == id)
            .map(|(extension, _)| extension.package.clone())
    }

    /// Runs a row's extension action, which the user chose: the only calls that carry an
    /// activation. Searching again afterwards shows a catalogue the action invalidated.
    pub(super) fn run_extension_request(
        &mut self,
        extension: String,
        request: ExtensionRequest,
        tx: &mut Tx,
    ) {
        let (action, data) = match request {
            ExtensionRequest::Run { action, data } => (action, data),
            ExtensionRequest::Paste { text, template } => {
                let Some(extension) = self.registered_extension(&extension) else {
                    return;
                };
                return self.run_paste(&extension, text, template, tx);
            }
            ExtensionRequest::Kind { action, subject } => {
                return self.run_kind_action(&extension, action, &subject, tx);
            }
            request => {
                let Some(extension) = self.registered_extension(&extension) else {
                    return;
                };
                return self.answer_extension_row(None, extension, request, tx);
            }
        };
        self.run_extension_action(extension, action, data.unwrap_or_default());
    }

    /// Runs the kind action `action` that `package` provides on a row Dupin's own sources
    /// found, handing the package only the one field its manifest names for it, and only once
    /// the user lets it see that field.
    fn run_kind_action(
        &mut self,
        package: &str,
        action: String,
        subject: &dupin_core::kind::Subject,
        tx: &mut Tx,
    ) {
        let Some(extension) = self.registered_extension(package) else {
            return self.show_notice(
                format!("No extension called {package} runs now, so its action didn't run."),
                tx,
            );
        };
        let kind = crate::composition::row_kind(subject);
        let Some(&field) = extension.provides.kinds.get(&(kind, action.clone())) else {
            return self.show_notice(
                format!(
                    "{} provides no {} action called {action}.",
                    extension.name,
                    kind.as_str()
                ),
                tx,
            );
        };
        let Some(value) = crate::composition::row_field(subject, field) else {
            return self.show_notice(
                format!(
                    "{}'s action didn't run: the {} isn't text it can be handed.",
                    extension.name,
                    field.as_str()
                ),
                tx,
            );
        };
        let bridge = &self.extensions.bridge;
        let request = dupin_extension_bridge::Request::Rows { kind, field };
        match bridge.authorize(&extension.package, &request, Trigger::Activation) {
            Ok(permit) if bridge.validate(&permit) => {
                let mut data = serde_json::Map::new();
                data.insert(field.as_str().to_owned(), value.into());
                self.run_extension_action(extension.id.clone(), action, data.into());
            }
            Ok(_) => self.show_notice(
                format!("{}'s access changed, so its action didn't run.", extension.name),
                tx,
            ),
            Err(refused) => {
                self.show_notice(format!("{}'s action didn't run: {refused}", extension.name), tx)
            }
        }
    }

    /// What `ctx.query` and `ctx.state` hand an action of `extension` chosen now: those of
    /// the page on screen when it is one of the package's own, and nothing otherwise, so a
    /// package never sees what is typed in the main search.
    fn page_context(&self, extension: &Extension) -> PageContext {
        let own = self.extensions.package(self.session.command_id()) == Some(extension.package.as_str());
        match self.session.page() {
            Some(page) if own => PageContext {
                query: self.session.query().to_owned(),
                state: page.context.clone().unwrap_or_default(),
            },
            _ => PageContext::default(),
        }
    }

    /// Runs `extension`'s `actions[action](data)`, which the user chose, or a form's submit,
    /// and carries out the effect it returns on the page it was chosen on.
    pub(super) fn run_extension_action(
        &mut self,
        extension: String,
        action: String,
        data: serde_json::Value,
    ) {
        self.start_extension_action(extension, action, data, None);
    }

    /// Runs the action of the row `item` a shortcut chose with the panel hidden, which shows
    /// the panel only for what the action has to say or opens, or for the consent it needs.
    pub(super) fn run_bound_extension_action(
        &mut self,
        extension: String,
        action: String,
        data: serde_json::Value,
        item: dupin_core::Item,
    ) {
        self.start_extension_action(extension, action, data, Some(item));
    }

    fn start_extension_action(
        &mut self,
        extension: String,
        action: String,
        data: serde_json::Value,
        bound: Option<dupin_core::Item>,
    ) {
        let Some(extension) = self.registered_extension(&extension) else {
            let anchor = self.session.anchor();
            self.form_answered(&anchor);
            return;
        };
        let anchor = self.session.anchor();
        let page = self.page_context(&extension);
        let bridge = self.extensions.bridge.clone();
        let acting = self.on_acted();
        let busy = Busy::new(self.running.clone());
        std::thread::spawn(move || {
            let call = bridge.call(&extension.package, Trigger::Activation);
            let result = extension.act(
                &action,
                &data,
                &page,
                crate::extensions::deadline_in(ACTION_DEADLINE),
                &call.services,
            );
            let stale = !bridge.is_current(&extension.package, call.generation);
            acting.acted(move |ui, tx| {
                ui.form_answered(&anchor);
                if stale {
                    ui.show_notice(
                        format!(
                            "{}'s access changed while it ran, so its result was dropped.",
                            extension.name
                        ),
                        tx,
                    );
                    let effects = ui.session.refresh();
                    ui.apply(effects, tx);
                } else {
                    ui.extension_acted(&extension, &anchor, result, bound, tx);
                }
            });
            drop(busy);
        });
    }

    /// Carries out what an action returned, on the page it was chosen on while that page is
    /// still the one on screen; a toast shows either way, since the action ran.
    fn extension_acted(
        &mut self,
        extension: &Extension,
        anchor: &Anchor,
        result: Result<Acted, RunError>,
        bound: Option<dupin_core::Item>,
        tx: &mut Tx,
    ) {
        // A shortcut's row refused for want of consent goes back to where shortcuts decide,
        // which now shows the consent and holds the row until it is granted.
        if let Some(item) = bound.as_ref()
            && let Some(registered) = self.registered_extension(&extension.id)
            && (!self
                .extensions
                .bridge
                .pending(Some(&extension.package))
                .is_empty()
                || self.extensions.bridge.asked_while_off(&extension.package))
        {
            return self.run_bound_row(registered, item.clone(), tx);
        }
        let bound = bound.is_some();
        let name = &extension.name;
        let effect = match result {
            Ok(acted) => acted.effect,
            Err(RunError::Cancelled) => None,
            Err(RunError::Failed(diagnostic)) => {
                crate::log::warn(&format!("extension {name}: {}", diagnostic.for_log()));
                self.show_notice(format!("{name} failed: {}", diagnostic.message), tx);
                None
            }
        };
        if let Some(text) = effect.as_ref().and_then(ActionEffect::toast) {
            if bound && !self.shown {
                self.show_notice(text.to_owned(), tx);
            } else {
                self.timed_notice(text.to_owned(), tx);
            }
        }
        // What the action opens needs the panel, which a shortcut's action ran without.
        let reveals = bound
            && matches!(
                effect,
                Some(ActionEffect::Push { .. } | ActionEffect::Enter { .. })
            );
        let navigated = match effect {
            None | Some(ActionEffect::Toast { .. }) => {
                // A refused request may have left a consent row to show.
                Navigated::Done(self.session.refresh())
            }
            Some(effect @ (ActionEffect::Hide | ActionEffect::Paste)) => {
                self.close_after_copy(extension, effect == ActionEffect::Paste, tx);
                return;
            }
            Some(ActionEffect::Push {
                view, title, state, ..
            }) => self.session.open_page(
                anchor,
                Page {
                    view,
                    title: title.unwrap_or_else(|| name.clone()),
                    context: state,
                },
            ),
            Some(ActionEffect::Pop { .. }) => self.session.close_page(anchor),
            Some(ActionEffect::Refresh { .. }) => self.session.refresh_page(anchor),
            Some(ActionEffect::ReplaceState { state, .. }) => self.session.replace_page_state(anchor, state),
            Some(ActionEffect::Enter { command, .. }) if !extension.may_enter(&command) => {
                self.show_notice(
                    format!("{name} failed: only a package that ships with Dupin may enter another command"),
                    tx,
                );
                Navigated::Done(Vec::new())
            }
            Some(ActionEffect::Enter { command, query }) => {
                match crate::features::aliases::command(&command, &self.session.command_ids()) {
                    Ok(id) => self.session.enter_command_from(anchor, &id, &query),
                    Err(problem) => {
                        self.show_notice(format!("{name} failed: its `enter` {problem}"), tx);
                        Navigated::Done(Vec::new())
                    }
                }
            }
            Some(ActionEffect::FieldErrors { errors }) => match self.show_field_errors(anchor, errors, tx) {
                Navigated::NoPage => {
                    self.show_notice(
                        format!("{name} failed: it returned field errors, which only a form's submit may"),
                        tx,
                    );
                    Navigated::Done(Vec::new())
                }
                navigated => navigated,
            },
        };
        match navigated {
            Navigated::Done(effects) => {
                self.apply(effects, tx);
                if reveals {
                    self.show(tx);
                }
            }
            Navigated::Stale | Navigated::NoPage => {}
            Navigated::TooDeep => self.show_notice(
                format!("{name} failed: it opened more views than Dupin keeps open; close one first"),
                tx,
            ),
        }
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
        // A refusal leaves nothing for a shortcut's held row to wait for.
        if matches!(request, ExtensionRequest::Deny { .. }) {
            self.drop_bound_row(&id);
        }
        let outcome = match request {
            ExtensionRequest::Run { .. } | ExtensionRequest::Paste { .. } | ExtensionRequest::Kind { .. } => {
                return;
            }
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
                // It answers the main search with `search` alone, never a catalogue.
                Role::MainSearch => false,
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
