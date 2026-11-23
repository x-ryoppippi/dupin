use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::{Error, Store};

/// Applies a store's limits on a background thread every `interval`, so records age out while
/// nothing is being written. Stops when dropped or when the store is gone.
pub struct Sweeper {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Sweeper {
    pub fn spawn(
        store: &Arc<Store>,
        interval: Duration,
        on_error: impl Fn(&Error) + Send + 'static,
    ) -> std::io::Result<Self> {
        let store: Weak<Store> = Arc::downgrade(store);
        let (stop, stopped) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("storage-sweeper".into())
            .spawn(move || {
                while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(interval) {
                    let Some(store) = store.upgrade() else {
                        break;
                    };
                    if let Err(error) = store.sweep() {
                        on_error(&error);
                    }
                }
            })?;
        Ok(Self {
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

impl Drop for Sweeper {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
