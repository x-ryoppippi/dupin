use std::collections::HashMap;
use std::sync::Arc;

use dupin_core::CommandId;
use rune::Tx;

use super::{Ui, search::spawn_searcher};
use crate::extensions::packages::{GRANTS_FILE, Packages};
use crate::extensions::{Catalog, Extension, Origin, Watch};

/// The bundled extensions, always, and the extensions folder while `extensions = true`; which
/// of them run; and which own a command and searcher now.
pub(super) struct Extensions {
    bundled: Folder,
    user: Folder,
    packages: Packages,
    registered: HashMap<CommandId, Arc<Extension>>,
}

#[derive(Default)]
struct Folder {
    watch: Option<Watch>,
    /// Identifies the current watch, so a scan from a stopped one is ignored.
    epoch: u64,
    problems: Vec<String>,
}

impl Extensions {
    pub(super) fn load() -> Self {
        Self {
            bundled: Folder::default(),
            user: Folder::default(),
            packages: Packages::load(crate::store::data_dir().map(|dir| dir.join(GRANTS_FILE))),
            registered: HashMap::new(),
        }
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
            Ok(watch) => self.extensions.folder(origin).watch = Some(watch),
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
        let config = self.configuration.values.borrow().clone();
        self.apply_config(&config, tx);
    }

    /// Settings' switch for one extension.
    pub(crate) fn set_extension_enabled(&mut self, id: String, enabled: bool, tx: &mut Tx) {
        let packages = &mut self.extensions.packages;
        packages.problem = packages.set_enabled(&id, enabled).err().map(|problem| {
            let problem = format!(
                "Couldn't turn {id} {}: {problem}",
                if enabled { "on" } else { "off" }
            );
            crate::log::warn(&problem);
            problem
        });
        let config = self.configuration.values.borrow().clone();
        self.apply_config(&config, tx);
    }

    pub(crate) fn install_extensions(
        &mut self,
        registration: &mut crate::commands::ConfiguredCommands,
    ) -> bool {
        let active = self.extensions.packages.active();
        let added = crate::features::extensions::add_extensions(registration, &active);
        let registered: HashMap<_, _> = added.registered.into_iter().collect();
        let current = self.session.command_id();
        let active_changed = registered.get(current).is_some_and(|next| {
            self.extensions
                .registered
                .get(current)
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
            let provider = crate::features::extensions::provider(extension.clone());
            let searcher = spawn_searcher(self.me.clone(), id.clone(), provider);
            self.searchers.insert(id.clone(), searcher);
            self.extensions.registered.insert(id, extension);
        }
        let listing = self.extensions.packages.listing();
        let problems: Vec<_> = self
            .extensions
            .packages
            .problem
            .iter()
            .chain(&self.extensions.bundled.problems)
            .chain(&self.extensions.user.problems)
            .cloned()
            .chain(added.problems)
            .collect();
        self.prefs
            .update(move |prefs, _| prefs.reflect_extensions(listing, problems));
        active_changed
    }
}
