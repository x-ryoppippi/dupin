//! The macOS side of Dupin: the providers that need the system (installed apps, Spotlight)
//! and the actions that hand results to it (NSWorkspace).

pub mod accessibility;
mod accessibility_window;
pub mod apps;
pub mod bundle;
mod bundle_names;
mod calendar;
pub mod dictionary;
pub mod file_actions;
pub mod file_ops;
pub mod font;
pub mod input_source;
pub mod login_item;
pub mod privacy;
pub mod quick_look;
mod recent;
pub mod selection;
mod settings_search;
pub mod spotlight;
pub mod system_hotkey;
pub mod watcher;
pub mod window;
pub mod workspace;

pub use apps::{AppsProvider, AppsWatcher};
pub use calendar::CalendarProvider;
pub use dictionary::DictionaryProvider;
pub use recent::RecentProvider;
pub use settings_search::SettingsProvider;
pub use spotlight::{SpotlightProvider, Target};
pub use watcher::PathWatcher;
