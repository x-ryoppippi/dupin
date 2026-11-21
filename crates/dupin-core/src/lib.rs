//! The part of Dupin that doesn't know about any UI framework or operating system: what a
//! result is, how providers hand results over, fuzzy matching, and the launcher session that
//! turns keys and query edits into searches and actions.

pub mod config;
pub mod fuzzy;
pub mod history;
pub mod item;
pub mod provider;
pub mod searcher;
pub mod session;

pub use config::{Config, InputSource, Position};
pub use history::History;
pub use item::{Action, Item, ItemKind};
pub use provider::{Provider, Request};
pub use searcher::{Delivery, Searcher};
pub use session::{Effect, Intent, Mode, Session};
