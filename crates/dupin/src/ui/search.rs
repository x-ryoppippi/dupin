use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

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
                crate::latency::results_done(self.session.origins());
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
            crate::latency::search_started(&command, generation);
            searcher.search(generation, query, folder, page, limit);
            self.deadlines.insert(command, (generation, due));
            tx.wake_at(due);
        }
    }

    pub(super) fn cancel_search(&mut self) {
        self.action_screen = None;
        self.searchers.values().for_each(Searcher::cancel);
        self.deadlines.clear();
    }

    /// In a headless run, waits for every search still running to answer and hands over what
    /// it delivers, so the frame shows the answer to the keys before it rather than whatever
    /// came back in the time a script left. A search that outlasts its own deadline is left
    /// for that deadline to expire.
    fn settle_headless(&mut self, clock: f64) -> bool {
        let Some(delivered) = headless_deliveries() else {
            return false;
        };
        let delivered = delivered
            .1
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut changed = false;
        while let Ok((origin, delivery, sent)) = delivered.try_recv() {
            self.deliver(origin, delivery, sent);
            changed = true;
        }
        let started = Instant::now();
        loop {
            let Some(due) = self
                .deadlines
                .iter()
                .filter(|(id, (generation, _))| self.session.pending(id, *generation))
                .map(|(_, (_, due))| *due)
                .max_by(f64::total_cmp)
            else {
                return changed;
            };
            let left = Duration::from_secs_f64((due - clock).max(0.0)).saturating_sub(started.elapsed());
            match delivered.recv_timeout(left) {
                Ok((origin, delivery, sent)) => {
                    self.deliver(origin, delivery, sent);
                    changed = true;
                }
                Err(_) => return changed,
            }
        }
    }

    /// Hands over what headless searches delivered while no frame waited for them.
    fn take_headless_deliveries(&mut self) {
        let Some(delivered) = headless_deliveries() else {
            return;
        };
        let delivered = delivered
            .1
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while let Ok((origin, delivery, sent)) = delivered.try_recv() {
            self.deliver(origin, delivery, sent);
        }
    }

    pub(super) fn poll_searches(&mut self, clock: f64, tx: &mut Tx) -> bool {
        let mut changed = self.settle_headless(clock);
        let due = take_due(&mut self.deadlines, clock);
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
                    crate::latency::results_done(self.session.origins());
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

    /// Ask for the icons of the rows on screen. Handles live in `icons`, which the cache
    /// needs to keep them, and which is trimmed to the current rows when it grows too big.
    pub(super) fn fetch_icons(&mut self) {
        let Some(cache) = &self.icon_cache else { return };
        if self.icons.len() > ICON_CAP {
            let current: std::collections::HashSet<_> = self
                .session
                .items()
                .iter()
                .filter_map(|item| item.icon.as_ref())
                .collect();
            self.icons.retain(|path, _| current.contains(path));
        }
        for icon in self.session.items().iter().filter_map(|item| item.icon.as_ref()) {
            if !self.icons.contains_key(icon) {
                let image = match icon {
                    dupin_core::Icon::File(path) => cache.app_icon(path, ICON_SIZE),
                    dupin_core::Icon::Image(path) => cache.file(path, ICON_SIZE, 2.0),
                };
                self.icons.insert(icon.clone(), image);
            }
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
                .send((origin, d, sent));
            me.update(|s, _| s.take_headless_deliveries());
            return;
        }
        me.update(move |s, tx| {
            s.poll(tx.clock(), tx);
            s.deliver(origin, d, sent);
        });
    })
}

type Delivered = (CommandId, Delivery, Instant);
type Deliveries = (Mutex<mpsc::Sender<Delivered>>, Mutex<mpsc::Receiver<Delivered>>);

/// Where a headless run's searches deliver, which a frame can wait on from the UI thread while
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
