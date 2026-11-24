//! The macOS sources Linux has none for yet: Spotlight, folders, System Settings, the
//! dictionary, calendars and recent items. Each provider finds nothing, and the dictionary and
//! calendar services report that Linux doesn't support them.

use std::sync::{Arc, RwLock};

use dupin_core::session::RecentHistory;
use dupin_core::{Item, Provider, ProviderError, Request};
use dupin_host_services::calendar::{self as calendar_service, Calendars, Millis};
use dupin_host_services::dictionary::{Dictionaries, Entry};
use dupin_host_services::{OsAccess, ServiceError};

use crate::AppsProvider;
use crate::privacy::CalendarAccess;

/// What a Spotlight search matches the query words against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Names,
    Contents,
}

pub struct SpotlightProvider(());

impl SpotlightProvider {
    pub fn new(_target: Target) -> Self {
        Self(())
    }

    pub fn with_demotions(_target: Target, _rules: Arc<RwLock<Vec<String>>>) -> Self {
        Self(())
    }
}

#[derive(Default)]
pub struct FolderProvider;

#[derive(Default)]
pub struct SettingsProvider(());

#[derive(Default)]
pub struct SystemDictionary(());

impl SystemDictionary {
    pub fn fixtures() -> Self {
        Self(())
    }
}

impl Dictionaries for SystemDictionary {
    fn lookup(&self, _term: &str, _dictionaries: Option<&[String]>) -> Result<Vec<Entry>, ServiceError> {
        Err(ServiceError::Unsupported(
            "the dictionary isn't supported on Linux yet".into(),
        ))
    }
}

/// When the fixture events are laid out around, in milliseconds: 23 November 2026, 08:00 UTC.
pub const FIXTURE_CLOCK: i64 = 1_795_420_800_000;

pub struct EventStore(());

impl EventStore {
    pub fn new(_access: CalendarAccess) -> Self {
        Self(())
    }

    pub fn fixtures() -> Self {
        Self(())
    }
}

impl Calendars for EventStore {
    fn access(&self) -> OsAccess {
        OsAccess::Restricted
    }

    fn day_offset(&self, _at: Millis) -> Result<i64, ServiceError> {
        Err(unsupported_calendars())
    }

    fn events(&self, _start: Millis, _end: Millis) -> Result<Vec<calendar_service::Event>, ServiceError> {
        Err(unsupported_calendars())
    }
}

fn unsupported_calendars() -> ServiceError {
    ServiceError::Unsupported("calendars aren't supported on Linux yet".into())
}

pub struct RecentProvider(());

impl RecentProvider {
    pub fn new(_apps: Arc<AppsProvider>, _history: RecentHistory) -> Self {
        Self(())
    }
}

macro_rules! finds_nothing {
    ($($provider:ty),*) => {$(
        impl Provider for $provider {
            fn search(&self, _: &Request, _: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
                Ok(())
            }
        }
    )*};
}

finds_nothing!(
    SpotlightProvider,
    FolderProvider,
    SettingsProvider,
    RecentProvider
);
