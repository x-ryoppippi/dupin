//! The Linux side of Dupin: the providers that need the system (installed apps, from desktop
//! entries) and the actions that hand results to it (desktop-entry launch and `xdg-open`).
#![cfg(target_os = "linux")]

pub mod accessibility;
pub mod apps;
pub mod bundle;
pub mod clipboard;
pub mod clock;
mod desktop_entry;
pub mod dirs;
pub mod file_actions;
pub mod file_ops;
pub mod input_source;
pub mod keychain;
pub mod login_item;
pub mod paste;
pub mod pick;
pub mod privacy;
mod providers;
pub mod quick_look;
pub mod selection;
pub mod sharing;
pub mod system_commands;
pub mod system_hotkey;
pub mod thumbnail;
pub mod watcher;
pub mod window;
pub mod window_management;
pub mod workspace;

pub use apps::{AppsProvider, AppsWatcher};
pub use providers::{
    EventStore, FIXTURE_CLOCK, FolderProvider, RecentProvider, SettingsProvider, SpotlightProvider,
    SystemDictionary, Target,
};
pub use watcher::PathWatcher;
