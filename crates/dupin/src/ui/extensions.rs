use std::collections::HashMap;
use std::sync::Arc;

use dupin_core::CommandId;
use rune::Tx;

use super::{Ui, search::spawn_searcher};

/// The extensions folder's latest catalogue while `extensions = true`, and which of its
/// extensions own a command and searcher now.
#[derive(Default)]
pub(super) struct Extensions {
    watch: Option<crate::extensions::Watch>,
    /// Identifies the current watch, so a scan from a stopped one is ignored.
    epoch: u64,
    catalog: Vec<Arc<crate::extensions::Extension>>,
    scan_problems: Vec<String>,
    registered: HashMap<CommandId, Arc<crate::extensions::Extension>>,
}

impl Ui {
    /// Start or stop reading the extensions folder. Stopping drops their commands at the next
    /// registry install, so no guest runs once the switch is off.
    pub(super) fn watch_extensions(&mut self, enabled: bool) {
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

    pub(super) fn install_extensions(
        &mut self,
        registration: &mut crate::commands::ConfiguredCommands,
    ) -> bool {
        let added = crate::commands::add_extensions(registration, &self.extensions.catalog);
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
            let provider = Arc::new(crate::extensions::ExtensionProvider::new(extension.clone()));
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
        self.prefs
            .update(move |prefs, _| prefs.reflect_extensions(loaded, problems));
        active_changed
    }
}
