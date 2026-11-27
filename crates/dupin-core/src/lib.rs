//! The part of Dupin that doesn't know about any UI framework or operating system: what a
//! result is, how providers hand results over, fuzzy matching, and the launcher session that
//! turns keys and query edits into searches and actions.

pub mod catalogue;
pub mod command;
pub mod config;
pub mod detail;
pub mod file_name;
pub mod file_query;
pub mod file_rank;
pub mod filter;
pub mod form;
pub mod fuzzy;
pub mod history;
pub mod item;
pub mod kind;
pub mod provider;
pub mod script;
pub mod searcher;
pub mod session;
pub mod snippets;

pub use command::{
    CommandError, CommandId, CommandRegistry, CommandSpec, KeywordEntry, KeywordHolder, KeywordTarget,
    QueryEligibility, QueryPresentation,
};
pub use config::{Backdrop, Config, DisplaySelection, InputSource, Position, Rows};
pub use detail::Detail;
pub use history::History;
pub(crate) use item::ActionDisposition;
pub use item::{
    Accessory, Action, AppCommand, ExtensionRequest, HostImage, HostImageSource, Icon, Item, ItemAction,
    Page, Payload, RowState, ShareTarget, SystemCommand, View, ago_ms,
};
pub use provider::{Batch, Cancellation, Provider, ProviderError, Request};
pub use searcher::{Delivery, Searcher};
pub use session::{Anchor, Effect, Intent, Navigated, Session};

#[cfg(test)]
mod concurrent_tests;
pub mod snapshots;
