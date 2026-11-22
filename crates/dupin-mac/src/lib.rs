//! The macOS side of Dupin: the providers that need the system (installed apps, Spotlight)
//! and the actions that hand results to it (NSWorkspace).

pub mod apps;
pub mod bundle;
mod bundle_names;
pub mod font;
pub mod input_source;
pub mod spotlight;
pub mod window;
pub mod workspace;

pub use apps::{AppsProvider, AppsWatcher};
pub use spotlight::{SpotlightProvider, Target};
