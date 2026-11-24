//! The guest's side of host services. The `dupin` module's functions hand each request to one
//! native function, which queues it and returns a promise; the call's promise loop sends the
//! queue through a [`Bridge`] and settles each promise with Dupin's reply. The guest never
//! learns why a call ran (typing, an action, an event): Dupin attaches that itself.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{Ctx, Function, Persistent, Promise};

use crate::Diagnostic;

/// Requests one call may have waiting for a reply at once.
pub const MAX_PENDING: usize = 16;
/// Requests one call may make in all.
pub const MAX_CALLS: u32 = 256;
/// The longest operation name and encoded arguments the guest may send.
pub const MAX_OP_BYTES: usize = 64;
pub const MAX_ARGS_BYTES: usize = 2 * 1024 * 1024;

const NATIVE: &str = "__dupinHost";
const INVALIDATE: &str = "__dupinInvalidate";

/// What the `dupin` module exports. `host` is captured and deleted before any package module
/// that imports `dupin` runs.
pub(crate) const SOURCE: &str = r#"const host = globalThis.__dupinHost;
delete globalThis.__dupinHost;
const invalidate = globalThis.__dupinInvalidate;
delete globalThis.__dupinInvalidate;

export const apiVersion = 2;

export const items = Object.freeze({
	invalidate() {
		if (typeof invalidate !== 'function' || !invalidate()) {
			throw new Error('items.invalidate() works only in an action handler');
		}
	},
});

export class DupinError extends Error {
	constructor(error) {
		super(String(error?.message ?? 'the host call failed'));
		this.name = 'DupinError';
		this.code = String(error?.code ?? 'failed');
		for (const key of ['capability', 'scope', 'description', 'permission']) {
			if (error?.[key] !== undefined) this[key] = error[key];
		}
	}
}

async function call(op, args) {
	if (typeof host !== 'function') {
		throw new DupinError({ code: 'unavailable', message: 'host services are unavailable here' });
	}
	const reply = JSON.parse(await host(op, JSON.stringify(args)));
	if (reply.error !== undefined) throw new DupinError(reply.error);
	return reply.ok;
}

function response(reply) {
	const body = reply.body;
	return Object.freeze({
		ok: reply.status >= 200 && reply.status < 300,
		status: reply.status,
		url: reply.url,
		redirected: reply.redirected,
		headers: Object.freeze(reply.headers),
		truncated: reply.truncated,
		async text() {
			return body;
		},
		async json() {
			return JSON.parse(body);
		},
	});
}

export const http = Object.freeze({
	async fetch(url, init = {}) {
		return response(
			await call('http.fetch', {
				url,
				method: init.method ?? 'GET',
				headers: init.headers ?? {},
				body: init.body,
			}),
		);
	},
});

export const files = Object.freeze({
	read(scope, path, options = {}) {
		return call('files.read', { scope, path, limit: options.limit });
	},
	async readText(scope, path, options = {}) {
		return (await call('files.read', { scope, path, limit: options.limit })).text;
	},
	async list(scope, path = '', options = {}) {
		return (await call('files.list', { scope, path, limit: options.limit })).entries;
	},
});

export const exec = Object.freeze({
	run(profile, inputs = {}) {
		return call('exec.run', { profile, inputs });
	},
});

export const storage = Object.freeze({
	async get(key) {
		return (await call('storage.get', { key })).value;
	},
	async set(key, value) {
		await call('storage.set', { key, value });
	},
	async delete(key) {
		await call('storage.delete', { key });
	},
	async keys() {
		return (await call('storage.keys', {})).keys;
	},
});

export const clipboard = Object.freeze({
	read() {
		return call('clipboard.read', {});
	},
	async readText() {
		const clip = await call('clipboard.read', {});
		return clip?.type === 'text' ? clip.text : undefined;
	},
	async writeText(text) {
		await call('clipboard.write', { text });
	},
});

export const dictionary = Object.freeze({
	async lookup(word) {
		return (await call('dictionary.lookup', { word })).entries;
	},
});

const time = (value) =>
	value instanceof Date ? value.getTime() : typeof value === 'string' ? Date.parse(value) : value;
const iso = (time) => new Date(time).toISOString();

export const calendar = Object.freeze({
	async events(range = {}) {
		const { events } = await call('calendar.events', {
			from: time(range.from),
			to: time(range.to),
			limit: range.limit,
		});
		return events.map((event) => Object.freeze({ ...event, start: iso(event.start), end: iso(event.end) }));
	},
});
"#;

/// Carries a call's host requests to Dupin and its replies back.
pub trait Bridge {
    /// Sends request `call`: an operation such as `http.fetch` and its JSON arguments.
    fn send(&mut self, call: u32, op: &str, args: &str) -> Result<(), Diagnostic>;
    /// Waits for the next reply until `stop` says the call must end, then gives up with `None`.
    fn receive(&mut self, stop: &dyn Fn() -> bool) -> Option<Reply>;
}

/// Dupin's answer to one request: `{"ok": value}` or `{"error": {"code", "message", …}}`.
#[derive(Debug)]
pub struct Reply {
    pub call: u32,
    pub json: String,
}

/// For calls where no host is reachable, such as `check`: every request fails at once.
#[derive(Default)]
pub struct Unavailable {
    queued: Vec<u32>,
}

impl Bridge for Unavailable {
    fn send(&mut self, call: u32, _op: &str, _args: &str) -> Result<(), Diagnostic> {
        self.queued.push(call);
        Ok(())
    }

    fn receive(&mut self, _stop: &dyn Fn() -> bool) -> Option<Reply> {
        let call = self.queued.pop()?;
        Some(Reply {
            call,
            json: failure("unavailable", "host services are unavailable here"),
        })
    }
}

pub(crate) fn failure(code: &str, message: &str) -> String {
    serde_json::json!({ "error": { "code": code, "message": message } }).to_string()
}

/// The requests a call has made, those waiting to be sent and the promises waiting on replies.
#[derive(Default)]
pub(crate) struct Calls {
    outbox: Vec<(u32, String, String)>,
    pending: HashMap<u32, Persistent<Function<'static>>>,
    issued: u32,
}

pub(crate) type Shared = Rc<RefCell<Calls>>;

/// Installs the native functions the `dupin` module captures. `invalidated` is set when an
/// action calls `items.invalidate()`; any other call gets `None`, and the guest an error.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    calls: &Shared,
    invalidated: Option<Rc<Cell<bool>>>,
) -> rquickjs::Result<()> {
    let invalidate = Function::new(ctx.clone(), move || match &invalidated {
        Some(invalidated) => {
            invalidated.set(true);
            true
        }
        None => false,
    })?;
    ctx.globals().set(INVALIDATE, invalidate)?;
    let calls = calls.clone();
    let native = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, op: String, args: String| -> rquickjs::Result<Promise<'js>> {
            let (promise, resolve, _reject) = ctx.promise()?;
            let mut state = calls.borrow_mut();
            let refused = if op.len() > MAX_OP_BYTES || args.len() > MAX_ARGS_BYTES {
                Some(failure(
                    "too-large",
                    "the request is larger than Dupin accepts from an extension",
                ))
            } else if state.issued >= MAX_CALLS {
                Some(failure(
                    "too-many-calls",
                    &format!("one call may make at most {MAX_CALLS} host requests"),
                ))
            } else if state.pending.len() >= MAX_PENDING {
                Some(failure(
                    "too-many-calls",
                    &format!("at most {MAX_PENDING} host requests may wait at once"),
                ))
            } else {
                None
            };
            if let Some(json) = refused {
                drop(state);
                resolve.call::<_, ()>((json,))?;
                return Ok(promise);
            }
            state.issued += 1;
            let call = state.issued;
            state.outbox.push((call, op, args));
            state.pending.insert(call, Persistent::save(&ctx, resolve));
            Ok(promise)
        },
    )?;
    ctx.globals().set(NATIVE, native)
}

impl Calls {
    pub(crate) fn take_outbox(&mut self) -> Vec<(u32, String, String)> {
        std::mem::take(&mut self.outbox)
    }

    pub(crate) fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn take(&mut self, call: u32) -> Option<Persistent<Function<'static>>> {
        self.pending.remove(&call)
    }

    /// Drops every saved resolver, which must happen before the runtime that owns them goes.
    pub(crate) fn clear(&mut self) {
        self.outbox.clear();
        self.pending.clear();
    }
}
