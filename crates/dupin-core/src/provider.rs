use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Item, Page};

/// A source of candidates. Apps, files and (later) extensions all take this shape.
///
/// `search` runs on a worker thread owned by a [`Searcher`](crate::Searcher), one search at a
/// time, so a provider may keep thread-affine state (a run loop) on it. It hands results over
/// in batches as they arrive and should return soon after `request.is_cancelled()` turns true.
pub trait Provider: Send + Sync + 'static {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError>;
}

pub struct Request {
    /// Raw field text after keyword routing: whitespace may be part of the provider's syntax.
    pub query: String,
    /// The folder being browsed, for a command that lists one.
    pub folder: Option<PathBuf>,
    /// The view of the command being searched, if one was pushed.
    pub page: Option<Page>,
    /// The most candidates the provider should emit in total, before the display limit.
    pub limit: usize,
    generation: u64,
    current: Arc<AtomicU64>,
}

impl Request {
    pub(crate) fn new(
        query: String,
        folder: Option<PathBuf>,
        page: Option<Page>,
        limit: usize,
        generation: u64,
        current: Arc<AtomicU64>,
    ) -> Self {
        Request {
            query,
            folder,
            page,
            limit,
            generation,
            current,
        }
    }

    /// A request for tests and direct calls, never cancelled.
    pub fn detached(query: impl Into<String>, limit: usize) -> Self {
        Request::new(query.into(), None, None, limit, 1, Arc::new(AtomicU64::new(1)))
    }

    /// A request for a pushed view, never cancelled.
    pub fn detached_page(query: impl Into<String>, page: Page, limit: usize) -> Self {
        Request::new(
            query.into(),
            None,
            Some(page),
            limit,
            1,
            Arc::new(AtomicU64::new(1)),
        )
    }

    /// A newer search or a cancel has superseded this one.
    pub fn is_cancelled(&self) -> bool {
        self.current.load(Ordering::Acquire) != self.generation
    }

    /// The same check as [`Request::is_cancelled`], for code that must own it, such as an
    /// engine's interrupt handler.
    pub fn cancellation(&self) -> Cancellation {
        Cancellation {
            generation: self.generation,
            current: self.current.clone(),
        }
    }
}

#[derive(Clone)]
pub struct Cancellation {
    generation: u64,
    current: Arc<AtomicU64>,
}

impl Cancellation {
    pub fn is_cancelled(&self) -> bool {
        self.current.load(Ordering::Acquire) != self.generation
    }
}

#[derive(Debug)]
pub struct ProviderError;
