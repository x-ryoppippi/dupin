use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use dupin_core::{CommandId, Delivery, Page, Provider, Searcher};
use rune::{AppHandle, ImageCache, Tx, Waker};

use super::Ui;

/// Icons kept decoded: enough for a full file result list and the apps around it.
const ICON_CAP: usize = 512;

pub const ICON_SIZE: f32 = 32.0;

impl Ui {
    pub fn setup(s: &mut Ui, waker: Waker) {
        s.icon_cache = Some(ImageCache::new(waker));
    }

    fn deliver(&mut self, origin: CommandId, delivery: Delivery, sent: Instant) {
        if self.session.deliver_from(origin.clone(), delivery) {
            crate::latency::delivered(&origin, sent, |generation| {
                self.session.pending(&origin, generation)
            });
            if self.session.results() == dupin_core::session::Results::Done {
                crate::latency::results_done(&self.session, &|origin, item| self.row_package(origin, item));
                if !self.session.command().private_results {
                    crate::trace::results_done(&self.query.text, self.session.items().len());
                }
            }
            if self
                .deadlines
                .get(&origin)
                .is_some_and(|(g, _)| !self.session.pending(&origin, *g))
            {
                self.deadlines.remove(&origin);
            }
            if self
                .action_screen
                .as_ref()
                .is_some_and(|screen| !screen.is_current(&self.session))
            {
                self.action_screen = None;
            }
            self.fetch_icons();
        }
    }

    pub(super) fn refresh_default(&mut self, tx: &mut Tx) {
        if self.session.command_id().as_str() == crate::features::apps::APPS {
            let effects = self.session.refresh();
            self.apply(effects, tx);
        }
    }

    pub(super) fn start_search(
        &mut self,
        command: CommandId,
        generation: u64,
        query: String,
        folder: Option<PathBuf>,
        page: Option<Page>,
        tx: &mut Tx,
    ) {
        self.action_screen = None;
        let spec = self.session.provider_spec(&command).expect("registered provider");
        let limit = spec.candidate_limit();
        let due = tx.clock() + spec.deadline.as_secs_f64();
        self.deadlines.retain(|id, (g, _)| {
            let keep = self.session.pending(id, *g);
            if !keep && let Some(worker) = self.searchers.get(id) {
                worker.cancel();
            }
            keep
        });
        for (id, worker) in &self.searchers {
            if !self.session.pending(id, generation) {
                worker.cancel();
            }
        }
        if let Some(searcher) = self.searchers.get(&command) {
            crate::latency::search_started(&command, generation, self.extensions.package(&command));
            searcher.search(generation, query, folder, page, limit);
            self.deadlines.insert(command, (generation, due));
            tx.wake_at(due);
        }
    }

    /// The package whose catalogue a web alias row came from; Dupin's own provider shows it,
    /// so no package answers the command itself.
    pub(super) fn row_package(&self, origin: &CommandId, item: &dupin_core::Item) -> Option<String> {
        if origin.as_str() != crate::features::web::WEB_KEYWORDS {
            return None;
        }
        let web = self.web.read().ok()?;
        web.package_of(&item.id)
            .map(|package| package.as_str().to_owned())
    }

    pub(super) fn cancel_search(&mut self) {
        self.action_screen = None;
        self.searchers.values().for_each(Searcher::cancel);
        self.deadlines.clear();
    }

    pub(super) fn poll_searches(&mut self, clock: f64, tx: &mut Tx) -> bool {
        let due = take_due(&mut self.deadlines, clock);
        let mut changed = false;
        for (id, generation) in due {
            if self.session.pending(&id, generation) {
                if let Some(worker) = self.searchers.get(&id) {
                    worker.cancel();
                }
                let delivered = self
                    .session
                    .deliver_from(id.clone(), Delivery::Expired { generation });
                changed |= delivered;
                if delivered {
                    crate::latency::delivered(&id, Instant::now(), |generation| {
                        self.session.pending(&id, generation)
                    });
                }
                if delivered && self.session.results() == dupin_core::session::Results::Done {
                    crate::latency::results_done(&self.session, &|origin, item| {
                        self.row_package(origin, item)
                    });
                    if !self.session.command().private_results {
                        crate::trace::results_done(&self.query.text, self.session.items().len());
                    }
                }
            }
        }
        if let Some(next) = self
            .deadlines
            .values()
            .map(|(_, due)| *due)
            .min_by(f64::total_cmp)
        {
            tx.wake_at(next);
        }
        changed
    }

    /// Ask for the icons of the rows on screen or near it, and the selected row's preview.
    /// Handles live in `icons`, which the cache needs to keep them, and which is trimmed to
    /// those rows when it grows too big. Returns whether it asked for anything.
    pub(super) fn fetch_icons(&mut self) -> bool {
        let Some(cache) = &self.icon_cache else {
            return false;
        };
        let items = self.session.items();
        let wanted = crate::view::wanted_rows(self, self.visible_rows.clone());
        if self.icons.len() > ICON_CAP {
            let current: std::collections::HashSet<_> = wanted
                .iter()
                .filter_map(|&index| items[index].icon.as_ref())
                .collect();
            self.icons.retain(|path, _| current.contains(path));
        }
        let mut asked = false;
        let mut jobs = Vec::new();
        let selected = self.session.selected();
        if let Some(image) = self
            .session
            .detail(selected)
            .and_then(|detail| detail.image.as_ref())
            .filter(|image| !self.thumbnails.has_preview(image))
            && let Some(package) = self.extension_package(&image.extension)
        {
            jobs.push(super::thumbnails::Job {
                image: image.clone(),
                size: super::thumbnails::Size::Preview,
                package,
            });
        }
        for index in wanted {
            let Some(icon) = items[index].icon.as_ref() else {
                continue;
            };
            if self.icons.contains_key(icon) {
                continue;
            }
            let image = match icon {
                dupin_core::Icon::File(path) => cache.app_icon(path, ICON_SIZE),
                dupin_core::Icon::Image(path) => cache.file(path, ICON_SIZE, 2.0),
                dupin_core::Icon::Host(image) => {
                    if let Some(package) = self.extension_package(&image.extension) {
                        jobs.push(super::thumbnails::Job {
                            image: image.clone(),
                            size: super::thumbnails::Size::Row,
                            package,
                        });
                    }
                    continue;
                }
            };
            self.icons.insert(icon.clone(), image);
            asked = true;
        }
        self.thumbnails.want(jobs, &self.extensions.bridge, &self.me);
        asked
    }

    /// The rows the list shows now changed, by scrolling or a new answer.
    pub(crate) fn rows_in_view(&mut self, rows: std::ops::Range<usize>) -> bool {
        if self.visible_rows == rows {
            return false;
        }
        self.visible_rows = rows;
        self.fetch_icons()
    }

    /// A thumbnail the worker made, or failed to.
    pub(super) fn thumbnail_made(
        &mut self,
        image: dupin_core::HostImage,
        size: super::thumbnails::Size,
        png: Option<Vec<u8>>,
    ) {
        let (Some(png), Some(cache)) = (png, &self.icon_cache) else {
            self.thumbnails.failed(image, size);
            return;
        };
        let decoded = cache.load(png);
        match size {
            super::thumbnails::Size::Row => {
                self.icons.insert(dupin_core::Icon::Host(image), decoded);
            }
            super::thumbnails::Size::Preview => self.thumbnails.keep_preview(image, decoded),
        }
    }
}

pub(super) fn spawn_searcher(me: AppHandle<Ui>, origin: CommandId, provider: Arc<dyn Provider>) -> Searcher {
    let name = origin.as_str().replace('.', "-");
    Searcher::spawn(&name, provider, move |d| {
        let origin = origin.clone();
        let sent = Instant::now();
        me.update(move |s, tx| {
            s.poll(tx.clock(), tx);
            s.deliver(origin, d, sent);
        });
    })
}

fn take_due(deadlines: &mut HashMap<CommandId, (u64, f64)>, clock: f64) -> Vec<(CommandId, u64)> {
    let due: Vec<_> = deadlines
        .iter()
        .filter(|(_, (_, deadline))| clock >= *deadline)
        .map(|(id, (generation, _))| (id.clone(), *generation))
        .collect();
    for (id, _) in &due {
        deadlines.remove(id);
    }
    due
}

#[cfg(test)]
#[test]
fn controlled_clock_expires_only_latest_deadline_once() {
    let id = CommandId::new("sample.search").unwrap();
    let mut deadlines = HashMap::new();
    deadlines.insert(id.clone(), (1, 10.0));
    deadlines.insert(id.clone(), (2, 20.0));
    assert!(take_due(&mut deadlines, 10.0).is_empty());
    assert_eq!(take_due(&mut deadlines, 20.0), [(id.clone(), 2)]);
    assert!(take_due(&mut deadlines, 21.0).is_empty());
    deadlines.insert(id.clone(), (3, 30.0));
    deadlines.remove(&id);
    assert!(take_due(&mut deadlines, 30.0).is_empty());
}
