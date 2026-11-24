//! The app's side of the broker bridge: where the grant files live, which services back each
//! capability, and what macOS allows, read without ever prompting.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use dupin_extension_bridge::services::{self, Services, Subscription};
use dupin_extension_bridge::{
    Base, Bridge, Broker, Failure, Local, OsAccess, OsPermission, PackageStorage, Scope, Trigger,
};
use dupin_extension_http::{HttpConfig, HttpService, Limits};
use dupin_host_services::{calendar, clipboard, dictionary, exec};
use dupin_mac::privacy::CalendarAccess;
use serde_json::Value;

use crate::scripts::catalog::Catalog;
use crate::scripts::runner::Runner;

/// The declarative base beside config.toml, which Home Manager writes.
pub const BASE_FILE: &str = "extension-grants.toml";
/// The user's own grants, in Dupin's data directory beside extensions.lock.
pub const LOCAL_FILE: &str = "grants.toml";

pub fn base_path() -> Option<PathBuf> {
    Some(crate::settings::path()?.parent()?.join(BASE_FILE))
}

pub fn local_path() -> Option<PathBuf> {
    crate::store::data_dir().map(|dir| dir.join(LOCAL_FILE))
}

/// The root of every `dupin-storage` namespace, in Dupin's data directory.
pub fn storage_path() -> Option<PathBuf> {
    crate::store::data_dir().map(|dir| dir.join("storage"))
}

/// Encrypted storage under the data key in the user's Keychain, which is read only when a
/// namespace is first opened.
pub fn storage() -> Option<dupin_storage::Storage> {
    Some(dupin_storage::Storage::new(
        storage_path()?,
        dupin_mac::keychain::KeychainKey,
    ))
}

/// What the platform services need from the launcher.
pub struct Platform {
    pub scripts: Arc<RwLock<Catalog>>,
    pub runner: Runner,
    pub calendar_access: CalendarAccess,
    /// Clipboard history's policy, whose ignored apps extensions never see copies from either.
    pub history: Arc<Mutex<dupin_core::clipboard::History>>,
}

/// The bridge the launcher's guest calls go through, with the user's grant files and the
/// services that act on the Mac. Tests and headless runs get one that keeps grants in memory
/// and reaches no service; a headless render may still declare a base beside its own config.
pub fn open(platform: Platform) -> Bridge {
    if cfg!(test) || rune::tooling::is_headless() {
        let base = base_path()
            .filter(|_| !cfg!(test))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|source| Base::parse(&source).ok())
            .unwrap_or_default();
        return Bridge::new(Broker::new(base, Local::default()), None, Services::none());
    }
    let services = Services {
        http: Box::new(HttpService::new(HttpConfig {
            limits: Limits {
                // A guest receives the body whole, inside a 16 MiB heap.
                max_response_bytes: 2 * 1024 * 1024,
                ..Limits::default()
            },
            ..HttpConfig::default()
        })),
        exec: Box::new(Programs {
            scripts: platform.scripts,
            runner: platform.runner,
        }),
        storage: match storage() {
            Some(storage) => Box::new(PackageStorage::new(storage).on_error(|namespace, error| {
                crate::log::warn(&format!("extension storage {namespace}: {error}"));
            })),
            None => Box::new(services::Unavailable(
                "extension storage needs XDG_DATA_HOME or HOME",
            )),
        },
        clipboard: Box::new(Pasteboard {
            history: platform.history,
            service: OnceLock::new(),
        }),
        dictionary: Box::new(Dictionaries(dictionary::Dictionary::new(
            dupin_mac::DictionaryProvider::default(),
        ))),
        calendar: Box::new(Calendars(calendar::Calendar::new(
            dupin_mac::CalendarProvider::new(platform.calendar_access),
        ))),
    };
    let bridge = match (base_path(), local_path()) {
        (Some(base), Some(local)) => Bridge::open(&base, &local, services),
        _ => Bridge::new(Broker::new(Base::default(), Local::unreadable()), None, services),
    };
    for problem in bridge.problems() {
        crate::log::warn(&format!("extension grants: {problem}"));
    }
    bridge
}

/// Reports what macOS allows for each permission an extension scope can need, as the Settings
/// → Permissions row for it reads it, and reads the grant files again if they changed. Reading
/// never prompts, so this runs at startup, when the panel shows and when Settings regains focus.
pub fn refresh_os_access(bridge: &Bridge, calendar: &CalendarAccess) {
    bridge.reload();
    if cfg!(test) || rune::tooling::is_headless() {
        return;
    }
    for permission in OsPermission::ALL {
        bridge.set_os_access(permission, os_access(permission, calendar));
    }
}

/// The broker's view of the Settings → Permissions row `permission` joins.
pub fn os_access(permission: OsPermission, calendar: &CalendarAccess) -> OsAccess {
    use dupin_mac::privacy::Permission;
    match Permission::of(permission) {
        Permission::Calendars => calendar.status().os_access(),
        row => row.status().os_access(),
    }
}

fn encoded(value: impl serde::Serialize) -> Result<Value, Failure> {
    serde_json::to_value(value).map_err(|error| Failure::new("failed", error.to_string()))
}

/// The general pasteboard, read only while macOS allows it and never from an ignored app's
/// copy. It is opened on first use, since opening it keeps the clipboard monitor running.
struct Pasteboard {
    history: Arc<Mutex<dupin_core::clipboard::History>>,
    service: OnceLock<clipboard::Clipboard<dupin_mac::clipboard::GeneralPasteboard>>,
}

impl Pasteboard {
    fn service(&self) -> &clipboard::Clipboard<dupin_mac::clipboard::GeneralPasteboard> {
        self.service.get_or_init(|| {
            let history = self.history.clone();
            clipboard::Clipboard::new(dupin_mac::clipboard::GeneralPasteboard::default(), move || {
                let ignored_apps = history
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .policy()
                    .ignored_apps()
                    .to_vec();
                clipboard::Policy {
                    ignored_apps,
                    ..clipboard::Policy::default()
                }
            })
        })
    }
}

impl services::Clipboard for Pasteboard {
    fn read(&self) -> Result<Option<Value>, Failure> {
        // The broker has already required the activation this read needs.
        let clip = self.service().read(Trigger::Activation).map_err(Failure::from)?;
        clip.map(|clip| encoded(clip.content)).transpose()
    }

    fn write_text(&self, text: &str) -> Result<(), Failure> {
        let content = clipboard::Write::text(text.to_owned()).map_err(Failure::from)?;
        self.service()
            .write(Trigger::Activation, &content)
            .map_err(Failure::from)
    }

    fn watch(&self, changed: Box<dyn Fn(Value) + Send + Sync>) -> Result<Subscription, Failure> {
        let watch = self
            .service()
            .watch(move |clip| {
                if let Ok(value) = serde_json::to_value(&clip.content) {
                    changed(value);
                }
            })
            .map_err(Failure::from)?;
        Ok(Box::new(watch))
    }
}

struct Dictionaries(dictionary::Dictionary<dupin_mac::DictionaryProvider>);

impl services::Dictionary for Dictionaries {
    fn lookup(&self, word: &str) -> Result<Value, Failure> {
        let lookup = dictionary::Lookup::new(word, None).map_err(Failure::from)?;
        encoded(self.0.lookup(&lookup).map_err(Failure::from)?)
    }
}

struct Calendars(calendar::Calendar<dupin_mac::CalendarProvider>);

impl services::Calendar for Calendars {
    fn window(&self, from: i64, to: i64) -> Result<(u32, u32), Failure> {
        let query = calendar::Query::new(from, to, None, None, None).map_err(Failure::from)?;
        let bounds = self.0.days(&query).map_err(Failure::from)?;
        Ok((bounds.past_days, bounds.future_days))
    }

    fn events(&self, from: i64, to: i64, limit: usize, scope: &Scope) -> Result<Value, Failure> {
        let query = calendar::Query::new(from, to, None, None, Some(limit)).map_err(Failure::from)?;
        let bounds = calendar::Bounds::of(scope)
            .ok_or_else(|| Failure::new("failed", "the permit isn't for calendars"))?;
        encoded(self.0.events(&query, bounds).map_err(Failure::from)?)
    }
}

/// Executable profiles and `@script:` targets, run as script commands are: through the
/// script catalogue and the runner's process groups and extension slots.
struct Programs {
    scripts: Arc<RwLock<Catalog>>,
    runner: Runner,
}

/// Looks a script up under the catalogue's lock only for as long as the lookup takes.
struct Lookup<'a>(&'a RwLock<Catalog>);

impl exec::Scripts for Lookup<'_> {
    fn resolve(&self, name: &str, args: &[String]) -> Result<PathBuf, String> {
        let catalog = self.0.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        exec::Scripts::resolve(&*catalog, name, args)
    }
}

impl services::Exec for Programs {
    fn run(
        &self,
        profile: &exec::Profile,
        values: &BTreeMap<String, exec::Value>,
        cancel: &AtomicBool,
    ) -> Result<exec::Outcome, exec::ExecError> {
        exec::run(profile, values, &Lookup(&self.scripts), &self.runner, cancel)
    }
}
