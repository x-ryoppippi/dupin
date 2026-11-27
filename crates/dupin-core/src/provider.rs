use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Accessory, Detail, Item, Page, form::Form};

/// A source of candidates. Apps, files and (later) extensions all take this shape.
///
/// `search` runs on a worker thread owned by a [`Searcher`](crate::Searcher), one search at a
/// time, so a provider may keep thread-affine state (a run loop) on it. It hands results over
/// in batches as they arrive and should return soon after `request.is_cancelled()` turns true.
pub trait Provider: Send + Sync + 'static {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError>;

    /// `search` for a provider that groups its rows under headings or shows a row as a
    /// [`Detail`]. The host searches through this; the default hands over plain rows.
    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        self.search(request, &mut |items| emit(items.into()))
    }
}

/// Rows handed over together, with how the host presents them beyond each row's own view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    pub items: Vec<Item>,
    /// The heading over these rows. The host starts a heading wherever the section changes
    /// between rows, so a provider keeps each section's rows together.
    pub section: Option<String>,
    /// Shown in place of a row's own view, by item ID. The view remains the plain-text form
    /// for places that don't render details.
    pub details: HashMap<String, Arc<Detail>>,
    /// Show a row's detail only while it is selected, so a long list stays a list.
    pub details_when_selected: bool,
    /// What rows show at their end, by item ID.
    pub accessories: HashMap<String, Vec<Accessory>>,
    /// What the search shows when it finds nothing, which a batch without rows may carry.
    pub empty_text: Option<String>,
    /// A form the batch's one row stands for, which the page shows in place of a list.
    pub form: Option<Arc<Form>>,
}

impl Batch {
    pub fn section(title: impl Into<String>, items: Vec<Item>) -> Self {
        Self {
            items,
            section: Some(title.into()),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_detail(mut self, id: impl Into<String>, detail: Detail) -> Self {
        self.details.insert(id.into(), Arc::new(detail));
        self
    }

    pub(crate) fn is_plain(&self) -> bool {
        self.section.is_none()
            && self.details.is_empty()
            && self.accessories.is_empty()
            && self.empty_text.is_none()
            && self.form.is_none()
    }
}

impl From<Vec<Item>> for Batch {
    fn from(items: Vec<Item>) -> Self {
        Self {
            items,
            ..Self::default()
        }
    }
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
