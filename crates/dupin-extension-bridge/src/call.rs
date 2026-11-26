//! One host request: decoded strictly from what the guest sent, checked by the broker, run by
//! its service and released only while the permit still holds.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use dupin_broker::{
    ClipboardAccess, ContextAccess, FilesAccess, FilesKind, Method, Permit, Request, Scope, ScopeKey,
    ScopeName, StorageOp, WindowsAccess,
};
use dupin_extension_http as http;
use dupin_host_services::{calendar, exec, files, windows};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::services::Failure;
use crate::{Session, failure_for, storage};

const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// A response a guest receives whole; its heap is 16 MiB.
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Blobs one stored record may carry, within `MAX_ITEM_BYTES` together with its JSON.
const MAX_BLOBS: usize = 16;
const DEFAULT_PAGE: usize = 50;
const MAX_PAGE: usize = 200;
/// The stored JSON one page of records returns at most, beyond its first record; the reply
/// must fit in the guest's heap.
const MAX_PAGE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_WORD_CHARS: usize = 200;
const MAX_EXPRESSION_BYTES: usize = 512;
/// What a snippet's keyword or name may be before Dupin checks it against its own, narrower
/// rules and answers with what is wrong.
const MAX_SNIPPET_FIELD_BYTES: usize = 1024;
const MAX_SNIPPET_TEXT_BYTES: usize = 128 * 1024;
const MAX_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;
const MAX_INPUTS: usize = 32;
const MAX_INPUT_BYTES: usize = 4 * 1024;
const MAX_EVENTS: usize = 500;
/// A calendar query's words, as the panel's query field holds them.
const MAX_QUERY_BYTES: usize = 4 * 1024;
/// The guest's own read limit can lower these but never raise them past the service's.
const DEFAULT_READ_BYTES: usize = 256 * 1024;
const MAX_READ_BYTES: usize = 2 * 1024 * 1024;
const SCRIPT_PREFIX: &str = "@script:";

fn decode<'a, T: Deserialize<'a>>(args: &'a [u8]) -> Result<T, Failure> {
    serde_json::from_slice(args).map_err(|error| Failure::invalid(format!("invalid arguments: {error}")))
}

fn bounded(text: &str, name: &str, max: usize) -> Result<(), Failure> {
    if text.len() > max {
        return Err(Failure::invalid(format!("`{name}` is longer than {max} bytes")));
    }
    Ok(())
}

fn scope_name(name: &str) -> Result<ScopeName, Failure> {
    ScopeName::new(name).map_err(|error| Failure::invalid(format!("`{name}`: {error}")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fetch {
    url: String,
    #[serde(default = "get")]
    method: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    body: Option<String>,
}

fn get() -> String {
    "GET".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilesArgs {
    scope: String,
    #[serde(default)]
    path: String,
    limit: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecArgs {
    profile: String,
    #[serde(default)]
    inputs: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Put {
    data: Value,
    #[serde(default)]
    blobs: Vec<BlobArg>,
    /// Stores the record as the one for this key, replacing the record stored for it before.
    key: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BlobArg {
    /// Base64, as JSON carries bytes.
    Bytes(String),
    /// The token of the image the package's latest clipboard change carried, which the bridge
    /// holds so the bytes never cross to the guest.
    Clip { clip: String },
}

/// What `clipboard.write` puts on the clipboard: `{ text }`, or a copy shaped as a watch
/// delivers one, naming an image by its stored blob and files by their handles.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipWrite {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    blob: Option<String>,
    files: Option<Vec<FileWrite>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileWrite {
    path: String,
    handle: String,
}

/// What a stored key may be: the guest's own name for a record.
const MAX_KEY_BYTES: usize = 256;
/// What a copied file's handle vouches for, in the namespace's host tags.
const FILE_HANDLE_PURPOSE: &str = "clipboard file";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    before: Option<String>,
    limit: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Handle {
    handle: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Word {
    word: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expression {
    expression: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnippetArgs {
    keyword: String,
    name: Option<String>,
    text: String,
    replaces: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Keyword {
    keyword: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    from: f64,
    to: f64,
    query: Option<String>,
    limit: Option<u64>,
    order: Option<calendar::Order>,
}

/// A frame as the guest sends it; the windows service checks its bounds.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameArgs {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Session {
    pub(crate) fn dispatch(
        &self,
        op: &str,
        args: &[u8],
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<Value, Failure> {
        match op {
            "http.fetch" => self.fetch(decode(args)?, deadline, cancelled),
            "files.read" => self.files(decode(args)?, FilesAccess::Read),
            "files.list" => self.files(decode(args)?, FilesAccess::List),
            "exec.run" => self.exec(decode(args)?, cancelled),
            "storage.put" => {
                let Put { data, blobs, key } = decode(args)?;
                if let Some(key) = &key
                    && (key.is_empty() || key.len() > MAX_KEY_BYTES)
                {
                    return Err(Failure::invalid(format!("a key is 1–{MAX_KEY_BYTES} bytes")));
                }
                let record = new_record(data, blobs, |token| self.clip(token))?;
                let (permit, namespace) = self.storage(StorageOp::Write)?;
                // A write is an effect, so the permit is checked again just before it.
                self.release(&permit, cancelled)?;
                let record = namespace.with(|store| match &key {
                    Some(key) => store.upsert_within_limits(key, record),
                    None => store.put_within_limits(record),
                })?;
                Ok(record_json(record))
            }
            "storage.get" => {
                let Id { id } = decode(args)?;
                let id = record_id(&id)?;
                let (permit, namespace) = self.storage(StorageOp::Read)?;
                let record = namespace.with(|store| store.get(&id))?;
                self.release(&permit, cancelled)?;
                Ok(record.map_or(Value::Null, record_json))
            }
            "storage.list" => {
                let ListArgs { before, limit } = decode(args)?;
                let before = before.as_deref().map(cursor).transpose()?;
                let limit = limit.map_or(DEFAULT_PAGE, |limit| (limit as usize).clamp(1, MAX_PAGE));
                let (permit, namespace) = self.storage(StorageOp::Read)?;
                let page = namespace.with(|store| store.list(before.as_ref(), limit))?;
                self.release(&permit, cancelled)?;
                Ok(page_json(page))
            }
            "storage.readBlob" => {
                let Handle { handle } = decode(args)?;
                let handle: dupin_storage::BlobHandle = handle
                    .parse()
                    .map_err(|_| Failure::invalid("not a blob handle"))?;
                let (permit, namespace) = self.storage(StorageOp::Read)?;
                let bytes = namespace.with(|store| store.read_blob(&handle))?;
                self.release(&permit, cancelled)?;
                Ok(json!({ "data": BASE64.encode(&*bytes) }))
            }
            "storage.delete" => {
                let Id { id } = decode(args)?;
                let id = record_id(&id)?;
                let (permit, namespace) = self.storage(StorageOp::Delete)?;
                self.release(&permit, cancelled)?;
                let deleted = namespace.with(|store| store.delete(&id))?;
                Ok(json!({ "deleted": deleted }))
            }
            "storage.clear" => {
                let Nothing {} = decode(args)?;
                let (permit, namespace) = self.storage(StorageOp::Delete)?;
                self.release(&permit, cancelled)?;
                namespace.with(dupin_storage::Store::clear)?;
                Ok(Value::Null)
            }
            "storage.usage" => {
                let Nothing {} = decode(args)?;
                let (permit, namespace) = self.storage(StorageOp::Read)?;
                let usage = namespace.with(|store| Ok(store.usage()))?;
                self.release(&permit, cancelled)?;
                let limits = storage_limits(&permit)?;
                Ok(json!({
                    "entries": usage.entries,
                    "bytes": usage.bytes,
                    "quotaBytes": limits.max_total_bytes,
                    "retentionDays": limits.max_age.map(|age| age.as_secs() / (24 * 60 * 60)),
                }))
            }
            "clipboard.read" => {
                let Nothing {} = decode(args)?;
                let permit = self.permit(&Request::Clipboard(ClipboardAccess::Read))?;
                let clip = self.bridge.services.clipboard.read()?;
                self.release(&permit, cancelled)?;
                Ok(clip.unwrap_or(Value::Null))
            }
            "clipboard.write" => {
                let write: ClipWrite = decode(args)?;
                self.write_clipboard(write, cancelled)?;
                Ok(Value::Null)
            }
            "dictionary.lookup" => {
                let Word { word } = decode(args)?;
                let word = word.trim();
                if word.is_empty()
                    || word.chars().count() > MAX_WORD_CHARS
                    || word.chars().any(char::is_control)
                {
                    return Err(Failure::invalid(format!(
                        "a word is 1–{MAX_WORD_CHARS} characters without control characters"
                    )));
                }
                let permit = self.permit(&Request::DictionaryLookup)?;
                let entries = self.bridge.services.dictionary.lookup(word)?;
                self.release(&permit, cancelled)?;
                Ok(json!({ "entries": entries }))
            }
            "calculator.evaluate" => {
                let Expression { expression } = decode(args)?;
                if expression.trim().is_empty()
                    || expression.len() > MAX_EXPRESSION_BYTES
                    || expression.chars().any(char::is_control)
                {
                    return Err(Failure::invalid(format!(
                        "an expression is 1–{MAX_EXPRESSION_BYTES} bytes on one line"
                    )));
                }
                let permit = self.permit(&Request::CalculatorEvaluate)?;
                let answer = self.bridge.services.calculator.evaluate(&expression, cancelled)?;
                self.release(&permit, cancelled)?;
                Ok(answer)
            }
            "snippets.save" => {
                let SnippetArgs {
                    keyword,
                    name,
                    text,
                    replaces,
                } = decode(args)?;
                bounded(&keyword, "keyword", MAX_SNIPPET_FIELD_BYTES)?;
                bounded(
                    name.as_deref().unwrap_or_default(),
                    "name",
                    MAX_SNIPPET_FIELD_BYTES,
                )?;
                bounded(&text, "text", MAX_SNIPPET_TEXT_BYTES)?;
                bounded(
                    replaces.as_deref().unwrap_or_default(),
                    "replaces",
                    MAX_SNIPPET_FIELD_BYTES,
                )?;
                let permit = self.permit(&Request::SnippetsWrite)?;
                // Storing it is the effect, so the permit is checked again just before it.
                self.release(&permit, cancelled)?;
                self.bridge.services.snippets.save(
                    crate::services::SnippetDraft { keyword, name, text },
                    replaces.as_deref(),
                )
            }
            "snippets.delete" => {
                let Keyword { keyword } = decode(args)?;
                bounded(&keyword, "keyword", MAX_SNIPPET_FIELD_BYTES)?;
                let permit = self.permit(&Request::SnippetsWrite)?;
                self.release(&permit, cancelled)?;
                let deleted = self.bridge.services.snippets.delete(&keyword)?;
                Ok(json!({ "deleted": deleted }))
            }
            "calendar.events" => {
                let Range {
                    from,
                    to,
                    query,
                    limit,
                    order,
                } = decode(args)?;
                if let Some(query) = &query {
                    bounded(query, "query", MAX_QUERY_BYTES)?;
                }
                let instant = |value: f64, name: &str| {
                    if value.is_finite() && value.fract() == 0.0 && value.abs() < 8.64e15 {
                        Ok(value as i64)
                    } else {
                        Err(Failure::invalid(format!("`{name}` must be a valid date")))
                    }
                };
                let (from, to) = (instant(from, "from")?, instant(to, "to")?);
                if to <= from {
                    return Err(Failure::invalid("`to` must come after `from`"));
                }
                let calendar = &self.bridge.services.calendar;
                let (past_days, future_days) = calendar.window(from, to)?;
                let permit = self.permit(&Request::CalendarRead {
                    past_days,
                    future_days,
                })?;
                let limit = limit.map_or(MAX_EVENTS, |limit| (limit as usize).min(MAX_EVENTS));
                let events = calendar.events(
                    from,
                    to,
                    query.as_deref(),
                    limit,
                    order.unwrap_or_default(),
                    permit.scope(),
                )?;
                self.release(&permit, cancelled)?;
                self.issue_links(&events);
                Ok(json!({ "events": events }))
            }
            "windows.displays" => {
                let Nothing {} = decode(args)?;
                let permit = self.permit(&Request::Windows(WindowsAccess::Read))?;
                let displays = self.bridge.services.windows.displays()?;
                self.release(&permit, cancelled)?;
                Ok(json!({ "displays": displays }))
            }
            "windows.focused" => {
                let Nothing {} = decode(args)?;
                let permit = self.permit(&Request::Windows(WindowsAccess::Read))?;
                let window = self.bridge.services.windows.focused()?;
                self.release(&permit, cancelled)?;
                Ok(json!({ "window": window }))
            }
            "windows.setFrame" => {
                let FrameArgs { x, y, width, height } = decode(args)?;
                let permit = self.permit(&Request::Windows(WindowsAccess::Arrange))?;
                // Moving the window is the effect, so the permit is checked again just before it.
                self.release(&permit, cancelled)?;
                let frame = self
                    .bridge
                    .services
                    .windows
                    .set_frame(windows::Frame::new(x, y, width, height))?;
                Ok(json!({ "frame": frame }))
            }
            "context.frontmost" => {
                let Nothing {} = decode(args)?;
                let permit = self.permit(&Request::Context(ContextAccess::Frontmost))?;
                let app = self.bridge.services.context.frontmost()?;
                self.release(&permit, cancelled)?;
                Ok(json!({ "app": app }))
            }
            "context.document" => {
                let Nothing {} = decode(args)?;
                let permit = self.permit(&Request::Context(ContextAccess::Document))?;
                let document = self.bridge.services.context.document()?;
                self.release(&permit, cancelled)?;
                Ok(json!({ "document": document }))
            }
            _ => Err(Failure::new(
                "unknown-operation",
                format!(
                    "`{}` isn't a host service",
                    op.chars().take(64).collect::<String>()
                ),
            )),
        }
    }

    fn write_clipboard(&self, write: ClipWrite, cancelled: &AtomicBool) -> Result<(), Failure> {
        let clipboard = &self.bridge.services.clipboard;
        let shape = |kind: &str| {
            Failure::invalid(format!(
                "a `{kind}` copy takes only what `clipboard.write` documents for it"
            ))
        };
        match (write.kind.as_deref(), write.text, write.blob, write.files) {
            (None | Some("text"), Some(text), None, None) => {
                bounded(&text, "text", MAX_CLIPBOARD_TEXT_BYTES)?;
                if text.is_empty() {
                    return Err(Failure::invalid("the text is empty"));
                }
                let permit = self.permit(&Request::Clipboard(ClipboardAccess::Write))?;
                self.release(&permit, cancelled)?;
                clipboard.write_text(&text)
            }
            (Some("image"), None, Some(blob), None) => {
                let permit = self.permit(&Request::Clipboard(ClipboardAccess::Write))?;
                let handle: dupin_storage::BlobHandle = blob
                    .parse()
                    .map_err(|_| Failure::invalid("`blob` is a stored blob's handle"))?;
                let (read, namespace) = self.storage(StorageOp::Read)?;
                let png = namespace.with(|store| store.read_blob(&handle))?;
                let png =
                    dupin_host_services::clipboard::Write::image(png.to_vec()).map_err(Failure::from)?;
                self.release(&read, cancelled)?;
                self.release(&permit, cancelled)?;
                let dupin_host_services::clipboard::Write::Image(png) = png else {
                    unreachable!("`Write::image` makes an image");
                };
                clipboard.write_image(&png)
            }
            (Some("files"), None, None, Some(files)) => {
                let permit = self.permit(&Request::Clipboard(ClipboardAccess::Write))?;
                let (read, namespace) = self.storage(StorageOp::Read)?;
                let paths = files
                    .iter()
                    .map(|file| {
                        let expected = namespace.with(|store| Ok(handle_for(store, &file.path)))?;
                        if expected != file.handle {
                            return Err(Failure::new(
                                "out-of-scope",
                                "only files a clipboard watch delivered, with their handles, can be copied",
                            ));
                        }
                        Ok(std::path::PathBuf::from(&file.path))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let write =
                    dupin_host_services::clipboard::Write::copied_files(paths).map_err(Failure::from)?;
                self.release(&read, cancelled)?;
                self.release(&permit, cancelled)?;
                let dupin_host_services::clipboard::Write::CopiedFiles(paths) = write else {
                    unreachable!("`Write::copied_files` makes files");
                };
                clipboard.write_files(&paths)
            }
            (Some(kind @ ("text" | "image" | "files")), ..) => Err(shape(kind)),
            _ => Err(Failure::invalid(
                "write `{ type: \"text\", text }`, `{ type: \"image\", blob }` or `{ type: \"files\", files }`",
            )),
        }
    }

    /// The image the package's latest clipboard change carried, by the token it came with.
    fn clip(&self, token: &str) -> Result<Arc<Vec<u8>>, Failure> {
        self.bridge
            .clips
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&self.package)
            .filter(|staged| staged.token == token)
            .map(|staged| staged.png.clone())
            .ok_or_else(|| {
                Failure::new(
                    "not-found",
                    "that clipboard image is gone; only the latest change's can be stored",
                )
            })
    }

    /// The handle a watch gives the package for the copied file at `path`, while the package
    /// may read its storage, whose key the handle derives from. Asking never records a consent
    /// request.
    pub(crate) fn file_handle(&self, path: &str) -> Option<String> {
        let permit = {
            let state = self.bridge.lock();
            match state.broker.check(
                &self.package,
                &self.revision,
                &Request::Storage { op: StorageOp::Read },
                self.trigger,
            ) {
                dupin_broker::Decision::Allow(permit) => permit,
                _ => return None,
            }
        };
        let namespace = self.open_namespace(&permit).ok()?;
        namespace.with(|store| Ok(handle_for(store, path))).ok()
    }

    /// Hands `read` the image a row of the package names, from the package's own namespace
    /// alone: a blob one of its records carries, or a file its clipboard watch delivered with
    /// that handle. Drawing a row is no request of the package's, so a refusal records no
    /// consent request; the row is drawn without its image.
    pub(crate) fn read_image<R>(
        &self,
        image: &crate::ImageRef<'_>,
        read: impl FnOnce(crate::ImageData<'_>) -> R,
    ) -> Result<R, Failure> {
        let permit = {
            let state = self.bridge.lock();
            if state.broker.generation(&self.package) != Some(self.generation) {
                return Err(revoked());
            }
            match state.broker.check(
                &self.package,
                &self.revision,
                &Request::Storage { op: StorageOp::Read },
                self.trigger,
            ) {
                dupin_broker::Decision::Allow(permit) => permit,
                dupin_broker::Decision::Deny(denial) => return Err(failure_for(&denial)),
                dupin_broker::Decision::NeedsConsent(_) => {
                    return Err(
                        Failure::new("needs-consent", "the package may not read its storage yet")
                            .scope("storage"),
                    );
                }
            }
        };
        let namespace = self.open_namespace(&permit)?;
        let result = match image {
            crate::ImageRef::Blob(handle) => {
                let handle: dupin_storage::BlobHandle = handle
                    .parse()
                    .map_err(|_| Failure::invalid("not a blob handle"))?;
                let bytes = namespace.with(|store| store.read_blob(&handle))?;
                self.release(&permit, &AtomicBool::new(false))?;
                read(crate::ImageData::Bytes(&bytes))
            }
            crate::ImageRef::File { path, handle } => {
                let expected = namespace.with(|store| Ok(handle_for(store, path)))?;
                if expected != *handle {
                    return Err(Failure::new(
                        "out-of-scope",
                        "only files a clipboard watch delivered, with their handles, can be drawn",
                    ));
                }
                self.release(&permit, &AtomicBool::new(false))?;
                read(crate::ImageData::File(std::path::Path::new(path)))
            }
        };
        Ok(result)
    }

    fn fetch(&self, args: Fetch, deadline: Instant, cancelled: &AtomicBool) -> Result<Value, Failure> {
        bounded(&args.url, "url", MAX_URL_BYTES)?;
        let method = match args.method.to_ascii_uppercase().as_str() {
            "GET" => (Method::Get, http::Method::Get),
            "HEAD" => (Method::Head, http::Method::Head),
            "POST" => (Method::Post, http::Method::Post),
            "PUT" => (Method::Put, http::Method::Put),
            "PATCH" => (Method::Patch, http::Method::Patch),
            "DELETE" => (Method::Delete, http::Method::Delete),
            _ => {
                return Err(Failure::invalid(format!(
                    "`{}` isn't a supported method",
                    args.method
                )));
            }
        };
        if args.headers.len() > MAX_HEADERS {
            return Err(Failure::invalid(format!("at most {MAX_HEADERS} headers")));
        }
        let header_bytes: usize = args
            .headers
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum();
        if header_bytes > MAX_HEADER_BYTES {
            return Err(Failure::invalid(format!(
                "headers are at most {MAX_HEADER_BYTES} bytes"
            )));
        }
        if let Some(body) = &args.body {
            bounded(body, "body", MAX_BODY_BYTES)?;
        }
        let permit = self.permit(&Request::Http {
            url: args.url.clone(),
            method: method.0,
        })?;
        let Scope::Http { origin, methods } = permit.scope() else {
            return Err(Failure::new("failed", "the permit doesn't name an origin"));
        };
        let origin = http::Origin::parse(&origin.to_string()).map_err(Failure::invalid)?;
        let methods = methods
            .iter()
            .filter_map(|method| http::Method::parse(method.as_str()).ok())
            .collect();
        let scope = http::HttpScope::new(vec![http::HttpRule { origin, methods }]);
        let request = http::Request {
            method: method.1,
            url: args.url,
            headers: args.headers.into_iter().collect(),
            body: args.body.map(String::into_bytes),
        };
        let cancel = http::CancelToken::new();
        let response = with_cancel(&cancel, deadline, cancelled, || {
            self.bridge.services.http.fetch(request, &scope, &cancel)
        })
        .map_err(|error| Failure::new(error.code(), error.to_string()))?;
        self.release(&permit, cancelled)?;
        let truncated = response.body.len() > MAX_RESPONSE_BYTES;
        let mut body = response.body;
        body.truncate(MAX_RESPONSE_BYTES);
        let body = body.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&body);
        let headers: BTreeMap<_, _> = response.headers.into_iter().collect();
        Ok(json!({
            "status": response.status,
            "url": response.url,
            "redirected": response.redirected,
            "headers": headers,
            "body": String::from_utf8_lossy(body),
            "truncated": truncated,
        }))
    }

    fn files(&self, args: FilesArgs, access: FilesAccess) -> Result<Value, Failure> {
        let name = scope_name(&args.scope)?;
        bounded(&args.path, "path", 4096)?;
        let permit = self.permit(&Request::Files {
            scope: name.clone(),
            access,
            path: args.path.clone(),
        })?;
        let (Scope::Files { kind, .. }, Some(binding)) = (permit.scope(), permit.binding()) else {
            return Err(Failure::new(
                "failed",
                "the permit doesn't name a picked file or folder",
            ));
        };
        let kind = match kind {
            FilesKind::File => files::ScopeKind::File,
            FilesKind::Directory => files::ScopeKind::Directory,
        };
        let generation = permit.generation();
        let table = self.bridge.files(&self.package);
        let mut table = table.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let table = &mut *table;
        let table = table.get_or_insert_with(|| (generation, files::Files::new()));
        // Handles belong to one generation: a revocation or a new pick invalidates them all.
        if table.0 != generation {
            *table = (generation, files::Files::new());
        }
        let table = &mut table.1;
        let picked = table
            .scopes()
            .find(|scope| scope.name() == name.as_str())
            .is_some_and(|scope| scope.kind() == kind);
        if !picked {
            let scope = files::Scope::pick(name.as_str(), kind, binding).map_err(file_failure)?;
            table.grant(scope);
        }
        let target = files::Target::Path {
            scope: name.as_str(),
            path: &args.path,
        };
        let value = match access {
            FilesAccess::Read => {
                let limit = args
                    .limit
                    .map_or(DEFAULT_READ_BYTES, |limit| (limit as usize).min(MAX_READ_BYTES));
                let contents = table.read(target, limit).map_err(file_failure)?;
                json!({
                    "text": String::from_utf8_lossy(&contents.bytes),
                    "size": contents.size,
                    "truncated": contents.truncated,
                    "handle": contents.handle.to_string(),
                })
            }
            FilesAccess::List => {
                let limit = args
                    .limit
                    .map_or(files::DEFAULT_LIST_ENTRIES, |limit| limit as usize);
                let listing = table.list(target, limit).map_err(file_failure)?;
                let entries: Vec<_> = listing
                    .entries
                    .iter()
                    .map(|entry| {
                        json!({
                            "name": entry.name,
                            "kind": match entry.kind {
                                files::EntryKind::File => "file",
                                files::EntryKind::Directory => "directory",
                                files::EntryKind::Symlink => "symlink",
                                files::EntryKind::Other => "other",
                            },
                            "size": entry.size,
                            "handle": entry.handle.map(|handle| handle.to_string()),
                        })
                    })
                    .collect();
                json!({ "entries": entries, "truncated": listing.truncated })
            }
        };
        self.release(&permit, &AtomicBool::new(false))?;
        Ok(value)
    }

    fn exec(&self, args: ExecArgs, cancelled: &AtomicBool) -> Result<Value, Failure> {
        if args.inputs.len() > MAX_INPUTS {
            return Err(Failure::invalid(format!("at most {MAX_INPUTS} inputs")));
        }
        let mut values = BTreeMap::new();
        for (name, value) in args.inputs {
            let value = match value {
                Value::String(text) => {
                    bounded(&text, &name, MAX_INPUT_BYTES)?;
                    exec::Value::Text(text)
                }
                Value::Number(number) => match number.as_i64() {
                    Some(number) => exec::Value::Integer(number),
                    None => return Err(Failure::invalid(format!("`{name}` must be a whole number"))),
                },
                _ => {
                    return Err(Failure::invalid(format!(
                        "`{name}` must be text or a whole number"
                    )));
                }
            };
            values.insert(name, value);
        }
        let (request, script) = match args.profile.strip_prefix(SCRIPT_PREFIX) {
            Some(name) => {
                let name = scope_name(name)?;
                let output = self.declares_output(&ScopeKey::Script(name.clone()));
                (Request::Script { name, output }, true)
            }
            None => (
                Request::Exec {
                    profile: scope_name(&args.profile)?,
                },
                false,
            ),
        };
        let permit = self.permit(&request)?;
        let profile = profile(&permit, script)?;
        self.release(&permit, cancelled)?;
        let outcome = self
            .bridge
            .services
            .exec
            .run(&profile, &values, cancelled)
            .map_err(|error| match error {
                exec::ExecError::InvalidInput { .. }
                | exec::ExecError::UnknownInput(_)
                | exec::ExecError::InvalidProfile(_) => Failure::invalid(error.to_string()),
                exec::ExecError::Unavailable(_) => Failure::new("unavailable", error.to_string()),
                exec::ExecError::Failed(_) => Failure::new("failed", error.to_string()),
            })?;
        self.release(&permit, cancelled)?;
        let (code, signal, timed_out) = match outcome.exit {
            exec::Exit::Code(code) => (Some(code), false, false),
            exec::Exit::Signal => (None, true, false),
            exec::Exit::TimedOut => (None, false, true),
            exec::Exit::Cancelled => return Err(Failure::new("cancelled", "the command was cancelled")),
        };
        let mut result = json!({ "code": code, "signal": signal, "timedOut": timed_out });
        if let Some(output) = outcome.output {
            result["stdout"] = output.stdout.into();
            result["stderr"] = output.stderr.into();
            result["truncated"] = output.truncated.into();
        }
        Ok(result)
    }

    /// The package's own namespace, open with the quota and retention the permit names.
    fn storage(&self, op: StorageOp) -> Result<(Permit, Arc<storage::Namespace>), Failure> {
        let permit = self.permit(&Request::Storage { op })?;
        let store = self.open_namespace(&permit)?;
        Ok((permit, store))
    }

    /// The package's namespace with the permit's limits, syncing while the package holds
    /// `storage:sync` on this Mac and has a name every Mac knows it by.
    fn open_namespace(&self, permit: &Permit) -> Result<Arc<storage::Namespace>, Failure> {
        let storage = &self.bridge.services.storage;
        let synced = storage.synced_name(&self.package).filter(|_| {
            let state = self.bridge.lock();
            matches!(
                state
                    .broker
                    .check(&self.package, &self.revision, &Request::StorageSync, self.trigger),
                dupin_broker::Decision::Allow(_)
            )
        });
        storage.open(&self.namespace, storage_limits(permit)?, synced.as_deref())
    }

    /// Asks the broker for `request` under this call's trigger.
    pub(crate) fn permit(&self, request: &Request) -> Result<Permit, Failure> {
        self.bridge.sync();
        let mut state = self.bridge.lock();
        if state.broker.generation(&self.package) != Some(self.generation) {
            return Err(revoked());
        }
        match state
            .broker
            .check(&self.package, &self.revision, request, self.trigger)
        {
            dupin_broker::Decision::Allow(permit) => Ok(permit),
            dupin_broker::Decision::Deny(denial) => {
                if denial == dupin_broker::Denial::Disabled
                    && state.asked_while_off.insert(self.package.clone())
                {
                    drop(state);
                    self.bridge
                        .notify(crate::Notice::ConsentNeeded(self.package.clone()));
                }
                Err(failure_for(&denial))
            }
            dupin_broker::Decision::NeedsConsent(needed) => {
                let failure = self.bridge.needs_consent(&mut state, &self.package, needed);
                drop(state);
                self.bridge
                    .notify(crate::Notice::ConsentNeeded(self.package.clone()));
                Err(failure)
            }
        }
    }

    /// Checks the permit again before a result is released or an effect performed, so a
    /// revocation during the operation drops what it produced.
    fn release(&self, permit: &Permit, cancelled: &AtomicBool) -> Result<(), Failure> {
        if cancelled.load(Ordering::Relaxed) {
            return Err(Failure::new(
                "cancelled",
                "the call ended before the request finished",
            ));
        }
        self.bridge.sync();
        let state = self.bridge.lock();
        if permit.generation() != self.generation || !state.broker.validate(permit) {
            return Err(revoked());
        }
        Ok(())
    }

    fn declares_output(&self, key: &ScopeKey) -> bool {
        self.bridge
            .lock()
            .broker
            .consents(&self.package)
            .iter()
            .any(|(scope, _)| scope.key() == *key && matches!(scope, Scope::Script { output: true, .. }))
    }
}

/// The handle of the copied file at `path` in `store`'s namespace.
fn handle_for(store: &dupin_storage::Store, path: &str) -> String {
    store.host_tag(FILE_HANDLE_PURPOSE, path)
}

/// The record to store, refused whole when the JSON and blobs the guest sent are larger than
/// one record may be. A clipboard image the bridge holds for the package, named by `clip`,
/// doesn't count toward that, since it never crossed from the guest.
fn new_record(
    data: Value,
    blobs: Vec<BlobArg>,
    clip: impl Fn(&str) -> Result<Arc<Vec<u8>>, Failure>,
) -> Result<dupin_storage::NewRecord, Failure> {
    if blobs.len() > MAX_BLOBS {
        return Err(Failure::new(
            "too-large",
            format!("a record carries at most {MAX_BLOBS} blobs"),
        ));
    }
    let too_large = || {
        Failure::new(
            "too-large",
            format!(
                "a record's JSON and blobs are at most {} bytes",
                storage::MAX_ITEM_BYTES
            ),
        )
    };
    let mut sent = data.to_string().len();
    let blobs = blobs
        .into_iter()
        .map(|blob| match blob {
            BlobArg::Bytes(blob) => {
                let bytes = BASE64
                    .decode(blob)
                    .map_err(|_| Failure::invalid("a blob must be Base64"))?;
                sent += bytes.len();
                Ok(bytes)
            }
            BlobArg::Clip { clip: token } => Ok(clip(&token)?.as_ref().clone()),
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    if sent as u64 > storage::MAX_ITEM_BYTES {
        return Err(too_large());
    }
    Ok(dupin_storage::NewRecord { data, blobs })
}

fn storage_limits(permit: &Permit) -> Result<dupin_storage::Limits, Failure> {
    match permit.scope() {
        Scope::Storage {
            quota_bytes,
            retention_days,
        } => Ok(storage::limits(*quota_bytes, *retention_days)),
        _ => Err(Failure::new("failed", "the permit isn't for storage")),
    }
}

fn record_id(id: &str) -> Result<dupin_storage::RecordId, Failure> {
    id.parse().map_err(|_| Failure::invalid("not a record ID"))
}

fn cursor(text: &str) -> Result<dupin_storage::Cursor, Failure> {
    text.parse().map_err(|_| Failure::invalid("not a page cursor"))
}

/// A record as the guest receives it: its ID and handles are opaque, and `createdAt` is
/// milliseconds since the epoch.
fn record_json(record: dupin_storage::Record) -> Value {
    let created_at = record
        .created_at()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    let blobs: Vec<Value> = record
        .blobs
        .iter()
        .map(|blob| json!({ "handle": blob.handle.to_string(), "size": blob.len }))
        .collect();
    json!({
        "id": record.id.to_string(),
        "createdAt": created_at,
        "data": record.data,
        "blobs": blobs,
    })
}

/// A page cut short where its records' JSON would pass [`MAX_PAGE_BYTES`], continuing after
/// the last record it kept.
fn page_json(page: dupin_storage::Page) -> Value {
    let total = page.records.len();
    let mut bytes = 0;
    let mut records = Vec::new();
    let mut next = page.next;
    for record in page.records {
        bytes += record.data.to_string().len() as u64;
        if !records.is_empty() && bytes > MAX_PAGE_BYTES {
            break;
        }
        records.push(record);
    }
    if records.len() < total {
        next = records.last().map(dupin_storage::Record::cursor);
    }
    json!({
        "records": records.into_iter().map(record_json).collect::<Vec<_>>(),
        "next": next.map(|cursor| cursor.to_string()),
    })
}

fn revoked() -> Failure {
    Failure::new(
        "revoked",
        "The extension's access changed while the request ran, so its result was dropped.",
    )
}

fn file_failure(error: files::FileError) -> Failure {
    let code = match error {
        files::FileError::InvalidPath(_) | files::FileError::InvalidScopeName => "invalid",
        files::FileError::NotFound => "not-found",
        files::FileError::ScopeChanged => "scope-changed",
        _ => "failed",
    };
    Failure::new(code, error.to_string())
}

/// The profile the permit approves. Its placeholders become required one-line text inputs; a
/// script target takes none.
fn profile(permit: &Permit, script: bool) -> Result<exec::Profile, Failure> {
    let unavailable = |error: exec::ExecError| Failure::new("unavailable", error.to_string());
    match permit.scope() {
        Scope::Script { name, output } if script => exec::Profile::new(
            name.as_str(),
            exec::Target::Script(name.to_string()),
            &[],
            BTreeMap::new(),
            exec::Options {
                return_output: *output,
                ..exec::Options::default()
            },
        )
        .map_err(unavailable),
        scope @ Scope::Exec { program, .. } if !script => {
            let target = match program {
                Some(program) => std::path::PathBuf::from(program),
                None => permit
                    .binding()
                    .ok_or_else(|| Failure::new("failed", "no program was picked for the profile"))?
                    .to_path_buf(),
            };
            exec_profile(scope, &target).map_err(unavailable)
        }
        _ => Err(Failure::new("failed", "the permit doesn't name a program")),
    }
}

/// The profile an `exec` scope declares, run with the program picked for it. Its placeholders
/// become required one-line text inputs.
pub fn exec_profile(scope: &Scope, program: &std::path::Path) -> Result<exec::Profile, exec::ExecError> {
    let Scope::Exec {
        name,
        args,
        output,
        ssh_agent,
        output_bytes,
        ..
    } = scope
    else {
        return Err(exec::ExecError::InvalidProfile(
            "the scope doesn't run a program".into(),
        ));
    };
    let inputs = args
        .iter()
        .filter_map(|arg| placeholder(arg))
        .map(|name| {
            (
                name,
                exec::Input {
                    kind: exec::InputKind::Text {
                        max_bytes: MAX_INPUT_BYTES,
                        multiline: false,
                        option_like: false,
                    },
                    required: true,
                },
            )
        })
        .collect();
    let template: Vec<&str> = args.iter().map(String::as_str).collect();
    exec::Profile::new(
        name.as_str(),
        exec::Target::Executable(program.to_path_buf()),
        &template,
        inputs,
        exec::Options {
            return_output: *output,
            ssh_agent: *ssh_agent,
            output_bytes: output_bytes.map_or(exec::DEFAULT_OUTPUT_BYTES, |bytes| bytes as usize),
            ..exec::Options::default()
        },
    )
}

/// The input an argument template element names, as in `--count={count}`.
pub(crate) fn placeholder(arg: &str) -> Option<String> {
    let unescaped = arg.replace("{{", "").replace("}}", "");
    let start = unescaped.find('{')?;
    let end = start + unescaped[start..].find('}')?;
    Some(unescaped[start + 1..end].to_owned())
}

/// Runs `fetch` while a watcher cancels it once the call ends or its deadline passes.
fn with_cancel<T>(
    cancel: &http::CancelToken,
    deadline: Instant,
    cancelled: &AtomicBool,
    fetch: impl FnOnce() -> T,
) -> T {
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                if cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline {
                    cancel.cancel();
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let result = fetch();
        done.store(true, Ordering::Relaxed);
        result
    })
}
