use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use dupin_core::{CommandId, ExtensionRun};
use dupin_js_runtime::{Acted, RunError};
use rune::Tx;

use super::{Busy, Ui, search::spawn_searcher};
use crate::extensions::{Extension, QUERY_DEADLINE, Role};

/// The extensions folder's latest catalogue while `extensions = true`, and which of its
/// extensions own a command and searcher now.
#[derive(Default)]
pub(super) struct Extensions {
    watch: Option<crate::extensions::Watch>,
    /// Identifies the current watch, so a scan from a stopped one is ignored.
    epoch: u64,
    catalog: Vec<Arc<crate::extensions::Extension>>,
    scan_problems: Vec<String>,
    registered: HashMap<CommandId, (Arc<Extension>, Role)>,
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
        let config = self.configuration.values.borrow().clone();
        self.apply_config(&config, tx);
    }

    pub(crate) fn install_extensions(
        &mut self,
        registration: &mut crate::commands::ConfiguredCommands,
    ) -> bool {
        let added = crate::features::extensions::add_extensions(registration, &self.extensions.catalog);
        let registered: HashMap<_, _> = added
            .registered
            .into_iter()
            .map(|(id, extension, role)| (id, (extension, role)))
            .collect();
        let active = self.session.command_id();
        let active_changed = registered.get(active).is_some_and(|(next, _)| {
            self.extensions
                .registered
                .get(active)
                .is_some_and(|(current, _)| !Arc::ptr_eq(next, current))
        });
        self.extensions.registered.retain(|id, (extension, _)| {
            let keep = registered
                .get(id)
                .is_some_and(|(next, _)| Arc::ptr_eq(next, extension));
            if !keep {
                self.searchers.remove(id);
                self.deadlines.remove(id);
            }
            keep
        });
        for (id, (extension, role)) in registered {
            if self.extensions.registered.contains_key(&id) {
                continue;
            }
            let provider = crate::features::extensions::provider(extension.clone(), role);
            let searcher = spawn_searcher(self.me.clone(), id.clone(), provider);
            self.searchers.insert(id.clone(), searcher);
            self.extensions.registered.insert(id, (extension, role));
        }
        let mut loaded: Vec<_> = self
            .extensions
            .registered
            .values()
            .filter(|(_, role)| *role == Role::Keyword)
            .map(|(extension, _)| {
                let main = if extension.default_search {
                    ", in the main search"
                } else {
                    ""
                };
                format!("{} ({}{main})", extension.name, extension.keyword)
            })
            .collect();
        loaded.sort();
        let problems: Vec<_> = self
            .extensions
            .scan_problems
            .iter()
            .cloned()
            .chain(added.problems)
            .collect();
        self.prefs
            .update(move |prefs, _| prefs.reflect_extensions(loaded, problems));
        active_changed
    }
}

impl Ui {
    /// Runs the handler a row's action named, off the main thread, and searches again when it
    /// invalidated the extension's catalogue so the change shows at once.
    pub(super) fn run_extension_action(&mut self, run: ExtensionRun) {
        let extension = self
            .extensions
            .registered
            .values()
            .find(|(extension, _)| extension.id == run.extension)
            .map(|(extension, _)| extension.clone());
        let Some(extension) = extension else {
            crate::log::warn(&format!("extension {} is no longer loaded", run.extension));
            return;
        };
        let me = self.me.clone();
        let busy = Busy::new(self.running.clone());
        let started = std::thread::Builder::new()
            .name("extension-action".into())
            .spawn(move || {
                let outcome =
                    extension.run_action(&run.handler, run.data.as_ref(), Instant::now() + QUERY_DEADLINE);
                drop(busy);
                me.update(move |ui, tx| ui.extension_acted(&extension, outcome, tx));
            });
        if let Err(error) = started {
            crate::log::warn(&format!("couldn't start the extension action: {error}"));
        }
    }

    fn extension_acted(&mut self, extension: &Extension, outcome: Result<Acted, RunError>, tx: &mut Tx) {
        match outcome {
            Ok(acted) if acted.invalidated => {
                let effects = self.session.refresh();
                self.apply(effects, tx);
            }
            Ok(_) | Err(RunError::Cancelled) => {}
            Err(RunError::Failed(diagnostic)) => {
                crate::log::warn(&format!("extension {}: {diagnostic}", extension.id));
                self.show_notice(format!("{} failed: {diagnostic}", extension.name), tx);
            }
        }
    }
}
