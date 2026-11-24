//! `clipboard:watch`: a package registers a handler, and while it is on and holds the scope,
//! each copy reaches it through the extension host. One delivery runs at a time per package;
//! copies that arrive meanwhile are coalesced to the newest, so a slow handler never queues
//! more than one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use dupin_broker::{ClipboardAccess, Decision, PackageId, Permit, Request, Trigger};
use serde_json::Value;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::Inner;
use crate::services::Subscription;

/// The most a change event carries of copied text.
const MAX_EVENT_TEXT_BYTES: usize = 64 * 1024;

/// A package's latest copied image, held by the bridge under the token its event carries.
pub(crate) struct Staged {
    pub token: String,
    pub png: Arc<Vec<u8>>,
}

pub(crate) type Deliver = Arc<dyn Fn(Value) + Send + Sync>;

#[derive(Default)]
pub(crate) struct Watches {
    entries: Mutex<HashMap<PackageId, Watch>>,
}

struct Watch {
    deliver: Deliver,
    /// The subscription and the permit it runs under, while the package may watch.
    active: Option<(Permit, Subscription)>,
    slot: Arc<Slot>,
}

#[derive(Default)]
struct Slot {
    latest: Mutex<Option<Value>>,
    busy: AtomicBool,
}

impl Watches {
    pub(crate) fn register(&self, id: PackageId, deliver: Deliver) {
        self.lock().insert(
            id,
            Watch {
                deliver,
                active: None,
                slot: Arc::default(),
            },
        );
    }

    pub(crate) fn unregister(&self, id: &PackageId) {
        self.lock().remove(id);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PackageId, Watch>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Inner {
    /// Subscribes each registered package that may watch now and drops the rest, after any
    /// change to grants, enablement or OS access.
    pub(crate) fn refresh_watches(self: &Arc<Self>) {
        let ids: Vec<_> = self.watches.lock().keys().cloned().collect();
        for id in ids {
            let permit = {
                let state = self.lock();
                state.broker.revision(&id).and_then(|revision| {
                    match state.broker.check(
                        &id,
                        revision,
                        &Request::Clipboard(ClipboardAccess::Watch),
                        Trigger::Background,
                    ) {
                        Decision::Allow(permit) => Some(permit),
                        _ => None,
                    }
                })
            };
            let mut entries = self.watches.lock();
            let Some(watch) = entries.get_mut(&id) else {
                continue;
            };
            match permit {
                None => watch.active = None,
                Some(permit) if watch.active.as_ref().is_some_and(|(held, _)| *held == permit) => {}
                Some(permit) => {
                    watch.active = None;
                    let changed = changed(
                        Arc::downgrade(self),
                        id.clone(),
                        permit.clone(),
                        watch.slot.clone(),
                        watch.deliver.clone(),
                    );
                    match self.services.clipboard.watch(changed) {
                        Ok(subscription) => watch.active = Some((permit, subscription)),
                        Err(_) => watch.active = None,
                    }
                }
            }
        }
    }

    fn still_holds(self: &Arc<Self>, permit: &Permit) -> bool {
        self.sync();
        self.lock().broker.validate(permit)
    }

    /// The change as `id`'s handler receives it: an image's bytes stay with the bridge under a
    /// token the handler can store them by, and each copied file carries a handle that lets the
    /// package put it back on the clipboard, while it may use its storage.
    fn prepared(self: &Arc<Self>, id: &PackageId, mut event: Value) -> Value {
        if let Some(data) = event.as_object_mut().and_then(|event| event.remove("data"))
            && let Some(png) = data.as_str().and_then(|data| BASE64.decode(data).ok())
        {
            let mut clips = self.clips.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let serial = clips
                .get(id)
                .and_then(|staged| staged.token.strip_prefix("clip:")?.parse::<u64>().ok())
                .map_or(1, |serial| serial + 1);
            let token = format!("clip:{serial}");
            event["size"] = png.len().into();
            event["blob"] = token.clone().into();
            clips.insert(
                id.clone(),
                Staged {
                    token,
                    png: Arc::new(png),
                },
            );
        }
        let handles = self.session(id, Trigger::Background).map(|session| {
            event
                .get("files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|file| {
                    file.get("path")
                        .and_then(Value::as_str)
                        .map(|path| session.file_handle(path))
                })
                .collect::<Vec<_>>()
        });
        if let (Some(handles), Some(files)) = (handles, event.get_mut("files").and_then(Value::as_array_mut))
        {
            for (file, handle) in files.iter_mut().zip(handles) {
                if let Some(Some(handle)) = handle {
                    file["handle"] = handle.into();
                }
            }
        }
        event
    }
}

fn changed(
    inner: Weak<Inner>,
    id: PackageId,
    permit: Permit,
    slot: Arc<Slot>,
    deliver: Deliver,
) -> Box<dyn Fn(Value) + Send + Sync> {
    Box::new(move |event| {
        let Some(bridge) = inner.upgrade() else { return };
        if !bridge.still_holds(&permit) {
            return;
        }
        let event = bridge.prepared(&id, bounded(event));
        *slot
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(event);
        if slot.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let (inner, permit, delivering, deliver) =
            (inner.clone(), permit.clone(), slot.clone(), deliver.clone());
        let started = std::thread::Builder::new()
            .name("extension-clipboard-watch".into())
            .spawn(move || {
                let slot = delivering;
                loop {
                    let next = slot
                        .latest
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let Some(event) = next else {
                        slot.busy.store(false, Ordering::Release);
                        // A copy that landed between the take and the release starts no thread
                        // of its own, so look once more.
                        if slot
                            .latest
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .is_none()
                            || slot.busy.swap(true, Ordering::AcqRel)
                        {
                            return;
                        }
                        continue;
                    };
                    let holds = inner.upgrade().is_some_and(|bridge| bridge.still_holds(&permit));
                    if holds {
                        deliver(event);
                    }
                }
            });
        if started.is_err() {
            slot.busy.store(false, Ordering::Release);
        }
    })
}

fn bounded(mut event: Value) -> Value {
    if let Some(text) = event
        .get_mut("text")
        .and_then(|text| text.as_str().map(str::to_owned))
        && text.len() > MAX_EVENT_TEXT_BYTES
    {
        let mut end = MAX_EVENT_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        event["text"] = text[..end].into();
        event["truncated"] = true.into();
    }
    event
}
