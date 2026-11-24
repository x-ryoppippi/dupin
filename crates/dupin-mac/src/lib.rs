//! The macOS side of Dupin: the providers that need the system (installed apps, Spotlight)
//! and the actions that hand results to it (NSWorkspace).

pub mod accessibility;
mod accessibility_window;
pub mod apps;
pub mod bundle;
mod bundle_names;
mod calendar;
pub mod clipboard;
pub mod clock;
pub mod dictionary;
pub mod file_actions;
pub mod file_ops;
mod folder;
pub mod font;
pub mod input_source;
pub mod keychain;
pub mod login_item;
pub mod paste;
pub mod pick;
pub mod privacy;
pub mod quick_look;
mod recent;
pub mod selection;
mod settings_search;
pub mod sharing;
pub mod spotlight;
pub mod system_commands;
pub mod system_hotkey;
pub mod watcher;
pub mod window;
pub mod window_management;
pub mod workspace;

pub use apps::{AppsProvider, AppsWatcher};
pub use calendar::{EventStore, FIXTURE_CLOCK};
pub use dictionary::SystemDictionary;
pub use folder::FolderProvider;
pub use recent::RecentProvider;
pub use settings_search::SettingsProvider;
pub use spotlight::{SpotlightProvider, Target};
pub use watcher::PathWatcher;
