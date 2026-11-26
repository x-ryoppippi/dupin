//! The app's side of the broker bridge: where the grants live, which services back each
//! capability, and what macOS allows, read without ever prompting.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use dupin_extension_bridge::services::{self, Services, Subscription};
use dupin_extension_bridge::{
    Base, Bridge, Broker, Failure, Local, OsAccess, OsPermission, PackageStorage, Scope, Trigger,
};
use dupin_extension_http::{HttpConfig, HttpService, Limits};
use dupin_host_services::{calendar, clipboard, dictionary, exec, windows};
use dupin_mac::privacy::CalendarAccess;
use serde_json::Value;

use crate::scripts::catalog::Catalog;
use crate::scripts::runner::Runner;

/// The declarative base beside config.toml, which Home Manager writes. The user's own grants
/// are in encrypted storage ([`super::grants`]).
pub const BASE_FILE: &str = "extension-grants.toml";

pub fn base_path() -> Option<PathBuf> {
    Some(crate::settings::path()?.parent()?.join(BASE_FILE))
}

/// The root of every `dupin-storage` namespace, in Dupin's data directory.
pub fn storage_path() -> Option<PathBuf> {
    crate::store::data_dir().map(|dir| dir.join("storage"))
}

/// The launcher's one encrypted storage, under the data key in the user's Keychain, which is
/// read only when a namespace is first opened. Extensions' namespaces and the built-ins' own,
/// such as stored snippets and usage history, and sync share it, so the device's clock, ID and
/// key have one owner, as a second root over the same folder would not.
pub fn shared_storage() -> Option<Arc<dupin_storage::Storage>> {
    static STORAGE: OnceLock<Option<Arc<dupin_storage::Storage>>> = OnceLock::new();
    STORAGE
        .get_or_init(|| {
            let root = storage_path()?;
            // Tests run the `dupin` executable, which must never reach the user's Keychain; a
            // release build ignores the variable.
            let storage = match std::env::var_os(DEBUG_KEY_FILE_ENV).filter(|_| cfg!(debug_assertions)) {
                Some(path) => dupin_storage::Storage::new(root, dupin_storage::FileKey::new(path)),
                None => dupin_storage::Storage::new(root, dupin_mac::keychain::KeychainKey),
            };
            Some(Arc::new(match dupin_mac::machine::platform_uuid() {
                Some(uuid) => storage.with_hardware_id(uuid.as_bytes()),
                None => storage,
            }))
        })
        .clone()
}

/// Names a key file a debug build uses instead of the Keychain.
pub const DEBUG_KEY_FILE_ENV: &str = "DUPIN_DEBUG_STORAGE_KEY_FILE";

/// What the platform services need from the launcher.
pub struct Platform {
    pub scripts: Arc<RwLock<Catalog>>,
    pub runner: Runner,
    pub calendar_access: CalendarAccess,
    /// Clipboard history's settings, whose ignored apps and limits every extension follows.
    pub clipboard: Arc<RwLock<dupin_core::config::Clipboard>>,
    /// The app the panel opened over, whose focused window the `windows` capability moves.
    pub front: Arc<dupin_mac::window_management::Front>,
    /// What the `calculator` capability works out with, the exchange rates included.
    pub calculator: crate::calculator::Calculator,
}

/// The bridge the launcher's guest calls go through, with the user's grants and the services
/// that act on the Mac. Tests and headless runs get one that keeps grants in memory
/// and reaches no service of the Mac's but a headless run's fixture calendar, as the built-in
/// Calendar search reads it, and fixed definitions, as the bundled Dictionary renders them; a
/// headless render may still declare a base beside its own config, and one `dupin ext snap`
/// starts answers from the fixture it renders.
pub fn open(platform: Platform) -> Bridge {
    if cfg!(test) {
        return Bridge::new(
            Broker::new(Base::default(), Local::default()),
            None,
            Services::none(),
        );
    }
    if rune::tooling::is_headless() {
        let base = base_path().map(|path| Base::read(&path)).unwrap_or_default();
        if let Some(fixture) = super::fixtures::headless() {
            return Bridge::new(Broker::new(base, Local::default()), None, fixture.services());
        }
        let services = Services {
            calendar: Box::new(Calendars(calendar::Calendar::new(
                dupin_mac::EventStore::fixtures(),
            ))),
            dictionary: fixed_dictionary(),
            calculator: Box::new(platform.calculator),
            storage: match crate::clipboard::headless_storage() {
                Some(storage) => Box::new(storage),
                None => Box::new(services::Unavailable(
                    "a headless run couldn't make scratch storage",
                )),
            },
            snippets: Box::new(crate::snippets::library()),
            ..Services::none()
        };
        let bridge = Bridge::new(Broker::new(base, Local::default()), None, services);
        bridge.set_os_access(OsPermission::Calendars, OsAccess::Allowed);
        return bridge;
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
        storage: match shared_storage() {
            Some(storage) => {
                let sync = crate::synced::ExtensionSync::new();
                let packages = Arc::new(
                    PackageStorage::new(storage)
                        .on_error(|namespace, error| {
                            crate::log::warn(&format!("extension storage {namespace}: {error}"));
                        })
                        .with_sync(sync.clone()),
                );
                sync.serve(&packages);
                Box::new(packages)
            }
            None => Box::new(services::Unavailable(
                "extension storage needs XDG_DATA_HOME or HOME",
            )),
        },
        clipboard: Box::new(Pasteboard {
            settings: platform.clipboard,
            service: OnceLock::new(),
        }),
        dictionary: Box::new(Dictionaries(dictionary::Dictionary::new(
            dupin_mac::SystemDictionary::default(),
        ))),
        calculator: Box::new(platform.calculator),
        snippets: Box::new(crate::snippets::library()),
        calendar: Box::new(Calendars(calendar::Calendar::new(dupin_mac::EventStore::new(
            platform.calendar_access,
        )))),
        windows: Box::new(Windows(windows::Service::new(
            dupin_mac::window_management::Windows(platform.front),
        ))),
    };
    let bridge = match base_path() {
        Some(base) => Bridge::open(&base, Box::new(super::grants::Shared), services),
        None => Bridge::new(Broker::new(Base::default(), Local::unreadable()), None, services),
    };
    for problem in bridge.problems() {
        crate::log::warn(&format!("extension grants: {problem}"));
    }
    bridge
}

/// Reads the user's grants into `bridge` on a thread of its own, since that reads the Keychain,
/// unless they are read or being read; until they are, extensions are granted nothing. Call it
/// once the bridge's listener is set, so what reading them changes reaches the app.
pub fn read_grants(bridge: &Bridge) {
    static READING: AtomicBool = AtomicBool::new(false);
    if bridge.grants_read() || READING.swap(true, Ordering::AcqRel) {
        return;
    }
    let reader = bridge.clone();
    let spawned = std::thread::Builder::new()
        .name("dupin-grants".into())
        .spawn(move || {
            reader.open_grants();
            for problem in reader.problems() {
                crate::log::warn(&format!("extension grants: {problem}"));
            }
            READING.store(false, Ordering::Release);
        });
    if let Err(error) = spawned {
        READING.store(false, Ordering::Release);
        crate::log::warn(&format!("couldn't start reading extension grants: {error}"));
    }
}

/// Reports what macOS allows for each permission an extension scope can need, as the Settings
/// → Permissions row for it reads it, reads the declared base again if it changed, and tries
/// the user's grants again while they couldn't be read, as a locked Keychain may have been
/// unlocked since. Reading what macOS allows never prompts, so this runs at startup, when the panel shows and when
/// Settings regains focus.
pub fn refresh_os_access(bridge: &Bridge, calendar: &CalendarAccess) {
    bridge.reload();
    read_grants(bridge);
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

/// Asks macOS to trust Dupin with Accessibility, which pasting or arranging windows for an
/// extension needs, on a deliberate press. macOS shows its prompt only while it hasn't been asked.
pub fn request_accessibility() {
    if cfg!(test) || rune::tooling::is_headless() {
        return;
    }
    if dupin_mac::privacy::Permission::Accessibility.status() == dupin_mac::privacy::Access::NotDetermined {
        dupin_mac::accessibility::request_once();
    }
}

fn encoded(value: impl serde::Serialize) -> Result<Value, Failure> {
    serde_json::to_value(value).map_err(|error| Failure::new("failed", error.to_string()))
}

/// The general pasteboard, read only while macOS allows it and never from an ignored app's
/// copy. It is opened on first use, since opening it keeps the clipboard monitor running.
struct Pasteboard {
    settings: Arc<RwLock<dupin_core::config::Clipboard>>,
    service: OnceLock<clipboard::Clipboard<dupin_mac::clipboard::GeneralPasteboard>>,
}

impl Pasteboard {
    fn service(&self) -> &clipboard::Clipboard<dupin_mac::clipboard::GeneralPasteboard> {
        self.service.get_or_init(|| {
            let settings = self.settings.clone();
            clipboard::Clipboard::new(dupin_mac::clipboard::GeneralPasteboard::default(), move || {
                let settings = settings.read().unwrap_or_else(|poisoned| poisoned.into_inner());
                clipboard::Policy {
                    ignored_apps: settings.ignored_apps().to_vec(),
                    max_text_bytes: settings.max_entry_bytes(),
                    max_image_bytes: settings.max_image_bytes(),
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

    fn write_image(&self, png: &[u8]) -> Result<(), Failure> {
        let content = clipboard::Write::image(png.to_vec()).map_err(Failure::from)?;
        self.service()
            .write(Trigger::Activation, &content)
            .map_err(Failure::from)
    }

    fn write_files(&self, paths: &[PathBuf]) -> Result<(), Failure> {
        let content = clipboard::Write::copied_files(paths.to_vec()).map_err(Failure::from)?;
        self.service()
            .write(Trigger::Activation, &content)
            .map_err(Failure::from)
    }

    fn watch(&self, changed: Box<dyn Fn(Value) + Send + Sync>) -> Result<Subscription, Failure> {
        let watch = self
            .service()
            .watch(move |clip| {
                if let Ok(mut value) = serde_json::to_value(&clip.content) {
                    if let Some(source) = &clip.source {
                        value["source"] = source.clone().into();
                    }
                    changed(value);
                }
            })
            .map_err(Failure::from)?;
        Ok(Box::new(watch))
    }
}

struct Dictionaries(dictionary::Dictionary<dupin_mac::SystemDictionary>);

/// Fixed definitions in Dictionary Services' layout, for renders and tests that must not
/// depend on the dictionaries this Mac has.
fn fixed_dictionary() -> Box<dyn services::Dictionary> {
    Box::new(Dictionaries(dictionary::Dictionary::new(
        dupin_mac::SystemDictionary::fixtures(),
    )))
}

impl services::Dictionary for Dictionaries {
    fn lookup(&self, word: &str) -> Result<Value, Failure> {
        let lookup = dictionary::Lookup::new(word, None).map_err(Failure::from)?;
        encoded(self.0.lookup(&lookup).map_err(Failure::from)?)
    }
}

struct Calendars(calendar::Calendar<dupin_mac::EventStore>);

impl services::Calendar for Calendars {
    fn window(&self, from: i64, to: i64) -> Result<(u32, u32), Failure> {
        let query = calendar::Query::new(from, to, None, None, None).map_err(Failure::from)?;
        let bounds = self.0.days(&query).map_err(Failure::from)?;
        Ok((bounds.past_days, bounds.future_days))
    }

    fn events(
        &self,
        from: i64,
        to: i64,
        query: Option<&str>,
        limit: usize,
        order: calendar::Order,
        scope: &Scope,
    ) -> Result<Value, Failure> {
        let query = calendar::Query::new(from, to, None, query, Some(limit))
            .map_err(Failure::from)?
            .ordered(order);
        let bounds = calendar::Bounds::of(scope)
            .ok_or_else(|| Failure::new("failed", "the permit isn't for calendars"))?;
        encoded(self.0.events(&query, bounds).map_err(Failure::from)?)
    }
}

struct Windows(windows::Service<dupin_mac::window_management::Windows>);

impl services::Windows for Windows {
    fn displays(&self) -> Result<Value, Failure> {
        encoded(self.0.displays().map_err(Failure::from)?)
    }

    fn focused(&self) -> Result<Value, Failure> {
        encoded(self.0.focused().map_err(Failure::from)?)
    }

    fn set_frame(&self, frame: windows::Frame) -> Result<Value, Failure> {
        encoded(self.0.set_frame(frame).map_err(Failure::from)?)
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
