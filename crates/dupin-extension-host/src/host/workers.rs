use std::io;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send>;

/// Threads that run host requests, each kept for `idle` after its last, so a request a guest
/// makes while the user types doesn't wait for a thread to start.
///
/// A job is handed to an idle worker under the pool's lock, and a worker leaves the pool under
/// the same lock, so a job is either sent before the worker last looks for one or never sent
/// to it at all.
pub(super) struct Workers {
    pool: Arc<Mutex<Pool>>,
    idle: Duration,
}

#[derive(Default)]
struct Pool {
    idle: Vec<(u64, mpsc::Sender<Job>)>,
    next: u64,
}

impl Default for Workers {
    fn default() -> Self {
        Self::new(super::IDLE)
    }
}

impl Workers {
    fn new(idle: Duration) -> Self {
        Self {
            pool: Arc::default(),
            idle,
        }
    }

    pub(super) fn run(&self, job: Job) -> io::Result<()> {
        let mut pool = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        let job = match pool.idle.pop() {
            Some((_, worker)) => match worker.send(job) {
                Ok(()) => return Ok(()),
                Err(mpsc::SendError(job)) => job,
            },
            None => job,
        };
        let id = pool.next;
        pool.next += 1;
        drop(pool);
        let (worker, jobs) = mpsc::channel::<Job>();
        let pool = Arc::downgrade(&self.pool);
        let idle = self.idle;
        std::thread::Builder::new()
            .name("extension-host-request".into())
            .spawn(move || {
                let mut job = job;
                loop {
                    job();
                    let Some(strong) = pool.upgrade() else { return };
                    strong
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .idle
                        .push((id, worker.clone()));
                    drop(strong);
                    job = match jobs.recv_timeout(idle) {
                        Ok(next) => next,
                        Err(RecvTimeoutError::Timeout) => {
                            let Some(strong) = pool.upgrade() else { return };
                            match retire(&strong, id, &jobs) {
                                Some(next) => next,
                                None => return,
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    };
                }
            })
            .map(drop)
    }

    #[cfg(test)]
    fn idle(&self) -> usize {
        self.pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .idle
            .len()
    }
}

/// Takes worker `id` out of the pool, unless a job reached it first, which it then returns.
fn retire(pool: &Mutex<Pool>, id: u64, jobs: &mpsc::Receiver<Job>) -> Option<Job> {
    let mut pool = pool.lock().unwrap_or_else(PoisonError::into_inner);
    match jobs.try_recv() {
        Ok(job) => Some(job),
        Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
            pool.idle.retain(|(worker, _)| *worker != id);
            None
        }
    }
}

#[cfg(test)]
mod tests;
