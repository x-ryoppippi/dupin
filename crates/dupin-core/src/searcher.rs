use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use std::path::PathBuf;
use std::thread;

use crate::{Batch, Item, Page, Provider, Request};

/// What a [`Searcher`] hands back: a batch of new results for one generation, or the end of it.
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    Batch {
        generation: u64,
        items: Vec<Item>,
    },
    /// Rows under a section heading or with details, from [`Provider::search_batches`].
    Presented {
        generation: u64,
        batch: Batch,
    },
    Done {
        generation: u64,
    },
    Failed {
        generation: u64,
    },
    Expired {
        generation: u64,
    },
}

impl Delivery {
    pub(crate) fn generation(&self) -> u64 {
        match self {
            Delivery::Batch { generation, .. }
            | Delivery::Presented { generation, .. }
            | Delivery::Done { generation }
            | Delivery::Failed { generation }
            | Delivery::Expired { generation } => *generation,
        }
    }
}

struct Job {
    query: String,
    folder: Option<PathBuf>,
    page: Option<Page>,
    limit: usize,
    generation: u64,
}

#[derive(Default)]
struct Mailbox {
    job: Option<Job>,
    closed: bool,
}

/// Runs one provider on its own long-lived thread, newest request wins.
///
/// A request supersedes every earlier one: queued ones never start, and a running one sees
/// `Request::is_cancelled` and its later batches are dropped here before they reach `deliver`.
/// `deliver` runs on the worker thread; hand its value to the UI thread from there.
pub struct Searcher {
    jobs: Arc<(Mutex<Mailbox>, Condvar)>,
    current: Arc<AtomicU64>,
}

/// Generation 0 means "nothing current": `cancel` stores it and requests start at 1.
const NONE: u64 = 0;

impl Searcher {
    pub fn spawn(
        name: &str,
        provider: Arc<dyn Provider>,
        deliver: impl Fn(Delivery) + Send + 'static,
    ) -> Searcher {
        let jobs = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let rx = jobs.clone();
        let current = Arc::new(AtomicU64::new(NONE));
        let worker_current = current.clone();
        thread::Builder::new()
            .name(format!("dupin-{name}"))
            .spawn(move || run(rx, &*provider, &worker_current, &deliver))
            .expect("spawn search worker");
        Searcher { jobs, current }
    }

    /// Start a search for `generation` (above 0), superseding any other.
    pub fn search(
        &self,
        generation: u64,
        query: String,
        folder: Option<PathBuf>,
        page: Option<Page>,
        limit: usize,
    ) {
        debug_assert_ne!(generation, NONE);
        self.current.store(generation, Ordering::Release);
        self.jobs.0.lock().unwrap().job = Some(Job {
            query,
            folder,
            page,
            limit,
            generation,
        });
        self.jobs.1.notify_one();
    }

    /// Stop the running search and drop queued ones.
    pub fn cancel(&self) {
        self.current.store(NONE, Ordering::Release);
        self.jobs.0.lock().unwrap().job = None;
    }
}

impl Drop for Searcher {
    fn drop(&mut self) {
        self.cancel();
        self.jobs.0.lock().unwrap().closed = true;
        self.jobs.1.notify_one();
    }
}
fn run(
    rx: Arc<(Mutex<Mailbox>, Condvar)>,
    provider: &dyn Provider,
    current: &Arc<AtomicU64>,
    deliver: &dyn Fn(Delivery),
) {
    loop {
        let job = {
            let mut mailbox = rx.0.lock().unwrap();
            while mailbox.job.is_none() && !mailbox.closed {
                mailbox = rx.1.wait(mailbox).unwrap();
            }
            if mailbox.closed {
                return;
            }
            mailbox.job.take().unwrap()
        };
        let is_current = |g: u64| current.load(Ordering::Acquire) == g;
        if !is_current(job.generation) {
            continue;
        }
        let request = Request::new(
            job.query,
            job.folder,
            job.page,
            job.limit,
            job.generation,
            current.clone(),
        );
        let generation = job.generation;
        let mut remaining = job.limit;
        let outcome = provider.search_batches(&request, &mut |mut batch| {
            batch.items.truncate(remaining);
            remaining -= batch.items.len();
            if (!batch.items.is_empty() || batch.empty_text.is_some()) && is_current(generation) {
                deliver(if batch.is_plain() {
                    Delivery::Batch {
                        generation,
                        items: batch.items,
                    }
                } else {
                    Delivery::Presented { generation, batch }
                });
            }
        });
        if is_current(generation) {
            deliver(if outcome.is_ok() {
                Delivery::Done { generation }
            } else {
                Delivery::Failed { generation }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::time::Duration;

    struct Echo {
        gate: Mutex<Option<Receiver<()>>>,
    }

    impl Provider for Echo {
        fn search(
            &self,
            request: &Request,
            emit: &mut dyn FnMut(Vec<Item>),
        ) -> Result<(), crate::ProviderError> {
            if request.query == "slow"
                && let Some(gate) = self.gate.lock().unwrap().take()
            {
                let _ = gate.recv();
            }
            let path = PathBuf::from(format!("/{}", request.query));
            emit(vec![Item::path(path, request.query.clone(), vec![])]);
            Ok(())
        }
    }

    #[test]
    fn a_newer_search_drops_the_running_ones_results() {
        let (open_gate, gate) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        let provider = Arc::new(Echo {
            gate: Mutex::new(Some(gate)),
        });
        let searcher = Searcher::spawn("test", provider, move |d| tx.send(d).unwrap());
        searcher.search(1, "slow".into(), None, None, 10);
        searcher.search(2, "fast".into(), None, None, 10);
        open_gate.send(()).unwrap();
        let mut got = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(d) => {
                    let done = matches!(d, Delivery::Done { generation: 2 });
                    got.push(d);
                    if done {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => panic!("no answer"),
                Err(e) => panic!("{e}"),
            }
        }
        assert!(got.iter().all(|d| d.generation() == 2), "{got:?}");
        assert!(matches!(&got[0], Delivery::Batch { items, .. } if items[0].display_title() == "fast"));
    }

    struct Sections;

    impl Provider for Sections {
        fn search(
            &self,
            request: &Request,
            emit: &mut dyn FnMut(Vec<Item>),
        ) -> Result<(), crate::ProviderError> {
            self.search_batches(request, &mut |batch| emit(batch.items))
        }

        fn search_batches(
            &self,
            _: &Request,
            emit: &mut dyn FnMut(Batch),
        ) -> Result<(), crate::ProviderError> {
            let row = |name: &str| Item::path(PathBuf::from(format!("/{name}")), name.into(), vec![]);
            emit(Batch::section("Today", vec![row("a"), row("b")]));
            emit(vec![row("c")].into());
            emit(Batch::section("Later", vec![row("d")]));
            Ok(())
        }
    }

    #[test]
    fn sections_reach_the_session_within_the_output_budget() {
        let (tx, rx) = mpsc::channel();
        let searcher = Searcher::spawn("test", Arc::new(Sections), move |d| tx.send(d).unwrap());
        searcher.search(1, String::new(), None, None, 3);
        let got: Vec<_> = (0..3)
            .map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert!(matches!(
            &got[0],
            Delivery::Presented { batch, .. } if batch.section.as_deref() == Some("Today") && batch.items.len() == 2
        ));
        assert!(matches!(&got[1], Delivery::Batch { items, .. } if items.len() == 1));
        assert_eq!(got[2], Delivery::Done { generation: 1 });
    }

    struct Nothing;

    impl Provider for Nothing {
        fn search(
            &self,
            request: &Request,
            emit: &mut dyn FnMut(Vec<Item>),
        ) -> Result<(), crate::ProviderError> {
            self.search_batches(request, &mut |batch| emit(batch.items))
        }

        fn search_batches(
            &self,
            _: &Request,
            emit: &mut dyn FnMut(Batch),
        ) -> Result<(), crate::ProviderError> {
            emit(Batch::default());
            emit(Batch {
                empty_text: Some("Nothing here".into()),
                ..Batch::default()
            });
            Ok(())
        }
    }

    #[test]
    fn what_to_show_for_no_rows_reaches_the_session_without_rows() {
        let (tx, rx) = mpsc::channel();
        let searcher = Searcher::spawn("test", Arc::new(Nothing), move |d| tx.send(d).unwrap());
        searcher.search(1, String::new(), None, None, 10);
        let got: Vec<_> = (0..2)
            .map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        assert!(matches!(
            &got[0],
            Delivery::Presented { batch, .. }
                if batch.items.is_empty() && batch.empty_text.as_deref() == Some("Nothing here")
        ));
        assert_eq!(got[1], Delivery::Done { generation: 1 });
    }

    #[test]
    fn cancel_silences_the_running_search() {
        let (open_gate, gate) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        let provider = Arc::new(Echo {
            gate: Mutex::new(Some(gate)),
        });
        let searcher = Searcher::spawn("test", provider, move |d| tx.send(d).unwrap());
        searcher.search(1, "slow".into(), None, None, 10);
        searcher.cancel();
        open_gate.send(()).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
