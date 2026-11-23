//! The part of Dupin that doesn't know about any UI framework or operating system: what a
//! result is, how providers hand results over, fuzzy matching, and the launcher session that
//! turns keys and query edits into searches and actions.

pub mod clipboard;
pub mod command;
pub mod config;
pub mod file_query;
pub mod file_rank;
pub mod fuzzy;
pub mod history;
pub mod item;
pub mod provider;
pub mod searcher;
pub mod session;

pub use command::{
    CommandError, CommandId, CommandRegistry, CommandSpec, QueryEligibility, QueryPresentation,
};
pub use config::{Backdrop, Config, InputSource, Position};
pub use history::History;
pub(crate) use item::ActionDisposition;
pub use item::{Action, Item, ItemAction, View};
pub use provider::{Provider, ProviderError, Request};
pub use searcher::{Delivery, Searcher};
pub use session::{Effect, Intent, Session};

#[cfg(test)]
mod concurrent_tests;
pub mod snapshots;
