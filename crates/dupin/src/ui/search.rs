use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
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

    /// Hand `delivery` to the session; returns whether it changed the results.
    fn deliver(&mut self, origin: CommandId, delivery: Delivery, sent: Instant, tx: &mut Tx) -> bool {
        let delivered = self.session.deliver_from(origin.clone(), delivery);
        if delivered {
            if self.entering && !self.session.items().is_empty() {
                self.entering = false;
                tx.animated();
            }
            crate::latency::delivered(&origin, sent, |generation| {
                self.session.pending(&origin, generation)
            });
            if self.session.results() == dupin_core::session::Results::Done {
                let row = |origin: &CommandId, item: &dupin_core::Item| self.row_package(origin, item);
                let extension = |id: &str| {
                    self.extension_package(id)
                        .map(|package| package.as_str().to_owned())
                };
                crate::latency::results_done(
                    &self.session,
                    crate::latency::Packages {
                        row: &row,
                        extension: &extension,
                    },
                );
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
            // The list keeps its offset when its rows change, so one scrolled for the results
            // these replace would hide the selected row. Revealing it slows the frame that
            // draws the results, so the first row of a list at its top asks for none.
            if self.session.selected() != 0 || self.list_offset.get() > 0.0 {
                self.reveal_selection(tx);
            }
            self.sync_forms(tx);
            self.fetch_icons();
        }
        delivered
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

    /// In a headless run, waits for every search and extension action still running to answer
    /// and hands over what each delivers, so the frame shows the answer to the keys before it
    /// rather than whatever came back in the time a script left, or in the time a busy machine
    /// let it run. What bounds each there is what runs it: a guest stops at its deadline in the
    /// processor time its replaying host counts, and native providers answer from the scratch
    /// home.
    fn settle_headless(&mut self, tx: &mut Tx) -> bool {
        let Some(delivered) = headless_deliveries() else {
            return false;
        };
        let delivered = delivered
            .1
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut changed = false;
        while let Ok(handed) = delivered.try_recv() {
            changed |= self.take_handed(handed, tx);
        }
        while self.headless_acting > 0
            || self
                .deadlines
                .iter()
                .any(|(id, (generation, _))| self.session.pending(id, *generation))
        {
            match delivered.recv() {
                Ok(handed) => changed |= self.take_handed(handed, tx),
                Err(_) => break,
            }
        }
        changed
    }

    /// Hands over what headless searches and actions delivered while no frame waited for them.
    fn take_headless_deliveries(&mut self, tx: &mut Tx) {
        let Some(delivered) = headless_deliveries() else {
            return;
        };
        let delivered = delivered
            .1
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while let Ok(handed) = delivered.try_recv() {
            self.take_handed(handed, tx);
        }
    }

    fn take_handed(&mut self, handed: Handed, tx: &mut Tx) -> bool {
        match handed {
            Handed::Delivered(origin, delivery, sent) => self.deliver(origin, *delivery, sent, tx),
            Handed::Acted(acted) => {
                self.headless_acting -= 1;
                acted(self, tx);
                true
            }
        }
    }

    /// Carries out on the UI what an action running on another thread with `me` returns, once
    /// it does: in a headless run through the frame that waits for it, as a search's answer is.
    pub(super) fn on_acted(&mut self) -> Acting {
        if headless_deliveries().is_some() {
            self.headless_acting += 1;
        }
        Acting(Some(self.me.clone()))
    }

    pub(super) fn poll_searches(&mut self, clock: f64, tx: &mut Tx) -> bool {
        let mut changed = self.settle_headless(tx);
        let due = take_due(&mut self.deadlines, clock);
        for (id, generation) in due {
            if self.session.pending(&id, generation) {
                if let Some(worker) = self.searchers.get(&id) {
                    worker.cancel();
                }
                changed |= self.deliver(id, Delivery::Expired { generation }, Instant::now(), tx);
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
                // Drawn in the colour of the row's text, which only the view knows.
                dupin_core::Icon::Symbol(_) => continue,
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
        if let Some(delivered) = headless_deliveries() {
            let _ = delivered
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .send(Handed::Delivered(origin, Box::new(d), sent));
            me.update(|s, tx| s.take_headless_deliveries(tx));
            return;
        }
        me.update(move |s, tx| {
            s.poll(tx.clock(), tx);
            s.deliver(origin, d, sent, tx);
        });
    })
}

/// What a search or an action running on another thread hands the UI.
enum Handed {
    Delivered(CommandId, Box<Delivery>, Instant),
    Acted(Acted),
}

type Acted = Box<dyn FnOnce(&mut Ui, &mut Tx) + Send>;

type Deliveries = (Mutex<mpsc::Sender<Handed>>, Mutex<mpsc::Receiver<Handed>>);

/// Where an action that [`Ui::on_acted`] counted hands its result; dropped without one, as by
/// a thread that panicked, it hands over nothing, so no frame waits for it.
#[must_use]
pub(super) struct Acting(Option<AppHandle<Ui>>);

impl Acting {
    pub(super) fn acted(mut self, acted: impl FnOnce(&mut Ui, &mut Tx) + Send + 'static) {
        self.hand(Box::new(acted));
    }

    fn hand(&mut self, acted: Acted) {
        let Some(me) = self.0.take() else { return };
        let Some(delivered) = headless_deliveries() else {
            return me.update(acted);
        };
        let _ = delivered
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(Handed::Acted(acted));
        me.update(|s, tx| s.take_headless_deliveries(tx));
    }
}

impl Drop for Acting {
    fn drop(&mut self) {
        self.hand(Box::new(|_, _| {}));
    }
}

/// Where a headless run's searches and actions deliver, which a frame can wait on from the UI thread while
/// an update it queued couldn't be run until that frame ends. None with a window.
fn headless_deliveries() -> Option<&'static Deliveries> {
    static CHANNEL: OnceLock<Deliveries> = OnceLock::new();
    rune::tooling::is_headless().then(|| {
        CHANNEL.get_or_init(|| {
            let (sender, receiver) = mpsc::channel();
            (Mutex::new(sender), Mutex::new(receiver))
        })
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
