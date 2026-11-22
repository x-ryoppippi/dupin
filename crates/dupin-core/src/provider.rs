use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Item;

/// A source of candidates. Apps, files and (later) extensions all take this shape.
///
/// `search` runs on a worker thread owned by a [`Searcher`](crate::Searcher), one search at a
/// time, so a provider may keep thread-affine state (a run loop) on it. It hands results over
/// in batches as they arrive and should return soon after `request.is_cancelled()` turns true.
pub trait Provider: Send + Sync + 'static {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError>;
}

pub struct Request {
    pub query: String,
    /// The most candidates the provider should emit in total, before the display limit.
    pub limit: usize,
    generation: u64,
    current: Arc<AtomicU64>,
}

impl Request {
    pub(crate) fn new(query: String, limit: usize, generation: u64, current: Arc<AtomicU64>) -> Self {
        Request {
            query,
            limit,
            generation,
            current,
        }
    }

    /// A request for tests and direct calls, never cancelled.
    pub fn detached(query: impl Into<String>, limit: usize) -> Self {
        Request::new(query.into(), limit, 1, Arc::new(AtomicU64::new(1)))
    }

    /// A newer search or a cancel has superseded this one.
    pub fn is_cancelled(&self) -> bool {
        self.current.load(Ordering::Acquire) != self.generation
    }
}

#[derive(Debug)]
pub struct ProviderError;
