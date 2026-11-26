//! The guest's side of host services. The `dupin` module's functions hand each request to one
//! native function, which queues it and returns a promise; the call's promise loop sends the
//! queue through a [`Bridge`] and settles each promise with Dupin's reply. The guest never
//! learns why a call ran (typing, an action, an event): Dupin attaches that itself.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use rquickjs::{Context, Ctx, Function, Module, Persistent, Promise, Runtime, WriteOptions};

use crate::{Diagnostic, HOST_MODULE};

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
/// runs.
pub(crate) const SOURCE: &str = r#"const host = globalThis.__dupinHost;
delete globalThis.__dupinHost;
const invalidate = globalThis.__dupinInvalidate;
delete globalThis.__dupinInvalidate;

export const apiVersion = 5;

export const items = Object.freeze({
	invalidate() {
		if (typeof invalidate !== 'function' || !invalidate()) {
			throw new Error('items.invalidate() works only in an action or event handler');
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

const BASE64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
const BASE64_VALUES = new Uint8Array(128).fill(255);
for (let index = 0; index < 64; index++) BASE64_VALUES[BASE64.charCodeAt(index)] = index;

function toBase64(bytes) {
	const chunks = [];
	for (let start = 0; start < bytes.length; start += 3 * 4096) {
		const end = Math.min(start + 3 * 4096, bytes.length);
		const codes = [];
		for (let index = start; index < end; index += 3) {
			const left = end - index;
			const n = (bytes[index] << 16) | ((left > 1 ? bytes[index + 1] : 0) << 8) | (left > 2 ? bytes[index + 2] : 0);
			codes.push(
				BASE64.charCodeAt(n >> 18),
				BASE64.charCodeAt((n >> 12) & 63),
				left > 1 ? BASE64.charCodeAt((n >> 6) & 63) : 61,
				left > 2 ? BASE64.charCodeAt(n & 63) : 61,
			);
		}
		chunks.push(String.fromCharCode(...codes));
	}
	return chunks.join('');
}

function fromBase64(text) {
	const padding = text.endsWith('==') ? 2 : text.endsWith('=') ? 1 : 0;
	const bytes = new Uint8Array((text.length / 4) * 3 - padding);
	let at = 0;
	for (let index = 0; index < text.length; index += 4) {
		const n =
			(BASE64_VALUES[text.charCodeAt(index)] << 18) |
			(BASE64_VALUES[text.charCodeAt(index + 1)] << 12) |
			((BASE64_VALUES[text.charCodeAt(index + 2)] & 63) << 6) |
			(BASE64_VALUES[text.charCodeAt(index + 3)] & 63);
		bytes[at++] = n >> 16;
		if (at < bytes.length) bytes[at++] = (n >> 8) & 255;
		if (at < bytes.length) bytes[at++] = n & 255;
	}
	return bytes;
}

function blobBytes(blob) {
	if (blob instanceof ArrayBuffer) return new Uint8Array(blob);
	if (ArrayBuffer.isView(blob)) return new Uint8Array(blob.buffer, blob.byteOffset, blob.byteLength);
	throw new DupinError({ code: 'invalid', message: 'a blob is a Uint8Array, another typed array or an ArrayBuffer' });
}

function stored(record) {
	if (record === null) return undefined;
	return Object.freeze({
		id: record.id,
		createdAt: new Date(record.createdAt).toISOString(),
		data: record.data,
		blobs: Object.freeze(record.blobs.map((blob) => Object.freeze(blob))),
	});
}

export const storage = Object.freeze({
	async put(data, options = {}) {
		const blobs = Array.from(options.blobs ?? [], (blob) =>
			typeof blob === 'string' ? { clip: blob } : toBase64(blobBytes(blob)),
		);
		return stored(await call('storage.put', { data, blobs, key: options.key }));
	},
	async get(id) {
		return stored(await call('storage.get', { id }));
	},
	async list(options = {}) {
		const page = await call('storage.list', { before: options.before, limit: options.limit });
		return Object.freeze({ records: Object.freeze(page.records.map(stored)), next: page.next ?? undefined });
	},
	async readBlob(handle) {
		return fromBase64((await call('storage.readBlob', { handle })).data);
	},
	async delete(id) {
		return (await call('storage.delete', { id })).deleted;
	},
	async clear() {
		await call('storage.clear', {});
	},
	async usage() {
		const usage = await call('storage.usage', {});
		return Object.freeze({ ...usage, retentionDays: usage.retentionDays ?? undefined });
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
	async write(clip) {
		switch (clip?.type) {
			case 'text':
				return void (await call('clipboard.write', { type: 'text', text: clip.text }));
			case 'image':
				return void (await call('clipboard.write', { type: 'image', blob: clip.blob }));
			case 'files':
				return void (await call('clipboard.write', {
					type: 'files',
					files: Array.from(clip.files ?? [], (file) => ({ path: file.path, handle: file.handle })),
				}));
			default:
				throw new DupinError({ code: 'invalid', message: 'write a copy of type "text", "image" or "files"' });
		}
	},
});

export const dictionary = Object.freeze({
	async lookup(word) {
		return (await call('dictionary.lookup', { word })).entries;
	},
});

export const calculator = Object.freeze({
	async evaluate(expression) {
		return await call('calculator.evaluate', { expression: String(expression) });
	},
});

export const snippets = Object.freeze({
	async save(snippet, options = {}) {
		const problems = await call('snippets.save', {
			keyword: snippet?.keyword,
			name: snippet?.name ?? null,
			text: snippet?.text,
			replaces: options.replaces ?? null,
		});
		return Object.freeze(problems);
	},
	async delete(keyword) {
		return (await call('snippets.delete', { keyword })).deleted;
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
			query: range.query,
			limit: range.limit,
			order: range.order,
		});
		return events.map((event) => Object.freeze({ ...event, start: iso(event.start), end: iso(event.end) }));
	},
});

export const windows = Object.freeze({
	async displays() {
		return (await call('windows.displays', {})).displays;
	},
	async focused() {
		return (await call('windows.focused', {})).window;
	},
	async setFrame(frame) {
		const { x, y, width, height } = frame ?? {};
		return (await call('windows.setFrame', { x, y, width, height })).frame;
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

/// The `dupin` module as bytecode, which this process compiles from [`SOURCE`] once and every
/// engine then loads.
pub(crate) fn bytecode() -> rquickjs::Result<&'static [u8]> {
    static BYTECODE: OnceLock<Box<[u8]>> = OnceLock::new();
    if let Some(bytecode) = BYTECODE.get() {
        return Ok(bytecode);
    }
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let bytecode = context.with(|ctx| {
        Module::declare(ctx, HOST_MODULE, SOURCE)?.write(WriteOptions {
            // Nothing reads the host module's own source back.
            strip_source: true,
            ..WriteOptions::default()
        })
    })?;
    Ok(BYTECODE.get_or_init(|| bytecode.into_boxed_slice()))
}

/// Whether the running call may call `items.invalidate()`, which only an action or event
/// handler may, and whether it did.
#[derive(Default)]
pub(crate) struct Invalidation {
    pub(crate) allowed: Cell<bool>,
    pub(crate) called: Cell<bool>,
}

/// Installs the native functions the `dupin` module captures. `items.invalidate()` throws in the
/// guest unless `invalidation` allows it.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    calls: &Shared,
    invalidation: &Rc<Invalidation>,
) -> rquickjs::Result<()> {
    let invalidation = invalidation.clone();
    let invalidate = Function::new(ctx.clone(), move || {
        if invalidation.allowed.get() {
            invalidation.called.set(true);
        }
        invalidation.allowed.get()
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
