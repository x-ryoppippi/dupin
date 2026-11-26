//! Host services that the extension broker exposes to guests: reads and listings inside
//! user-picked file scopes, user-approved executable profiles, and the OS-backed dictionary,
//! calendar, clipboard, windows and context. Each acts with the user's native authority, so the broker checks
//! grants before calling it. Nothing here prompts, stores grants or talks to a guest.
//!
//! Each OS-backed service defines a narrow backend trait that `dupin-mac` implements and a
//! `Fixture` for tests, and returns serialisable results or a [`ServiceError`].

pub mod calendar;
pub mod clipboard;
pub mod context;
pub mod dictionary;
pub mod exec;
pub mod files;
mod service;
pub mod windows;

pub use dupin_broker::{OsAccess, OsPermission, Trigger};
pub use service::{ServiceError, permission_id};

fn display_path(path: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty())
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        };
    }
    path.display().to_string()
}
