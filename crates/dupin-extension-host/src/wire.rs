//! Messages between Dupin and its extension host, over one Unix socket. A frame is two
//! big-endian `u32` lengths, then a JSON header and a binary tail of those lengths. A `run`
//! carries the package in its tail, module after module: its source to compile, or the bytecode
//! a compile returned, unless the helper already holds that bytecode. A `compiled` answer
//! carries that bytecode the same way. While a run is going, the helper may send `host`
//! requests, each with its JSON arguments as the tail, and Dupin answers each with a `reply`
//! whose tail is the JSON result.

use std::io::{self, IoSlice, Read, Write};
use std::sync::Arc;

use dupin_js_runtime::row::MAX_PAGE_BYTES;
use dupin_js_runtime::{
    Acted, CataloguePage, Diagnostic, Environment, Exports, FilterHit, FilterInput, Handled, PageContext,
    Reason, Row, ViewAnswer,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;

/// Raised whenever a message changes shape. The helper states its version first, and Dupin
/// stops a helper that states another.
pub const VERSION: u32 = 11;
/// Above the largest answer, part of a catalogue with [`MAX_PAGE_BYTES`] of text. JSON writes a
/// control character, which copy text and Markdown may hold, as six bytes (`\u0001`), and the
/// rest covers the field names and punctuation of the most rows and fields a part may have.
const MAX_HEADER_BYTES: usize = 6 * MAX_PAGE_BYTES + 2 * 1024 * 1024;
/// A package's bytecode, from at most 4 MiB of JavaScript.
pub const MAX_TAIL_BYTES: usize = 32 * 1024 * 1024;
/// The bytecode a helper holds for later runs; it lets the least recently run go first.
pub const MAX_HELD_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToHelper {
    /// The tail holds each module in `modules`' order. Of a run, the helper keeps only the
    /// bytecode of a package Dupin names, for later runs of it.
    Run {
        id: u64,
        /// Milliseconds the run may take from when the helper reads it.
        deadline_ms: u64,
        entry: String,
        /// The compiled package Dupin runs, which it names the same in every run. None for a
        /// compile.
        package: Option<u64>,
        /// None when Dupin sent this helper `package`'s bytecode before; the helper answers
        /// `missing` if it no longer holds it.
        modules: Option<Vec<ModuleSpan>>,
        call: Call,
        /// A fixture's fixed clock and seed; the launcher's own runs carry none.
        environment: Option<Environment>,
    },
    /// Stop run `id` if it is still running.
    Cancel { id: u64 },
    /// Dupin's answer to request `call` of run `id`, as JSON in the tail.
    Reply { id: u64, call: u32 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleSpan {
    pub name: String,
    pub len: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Call {
    /// The modules are source text; the answer is their bytecode.
    Compile,
    /// The modules are bytecode from a compile.
    Check,
    Search {
        query: String,
        limit: u32,
    },
    Items {
        cursor: Option<String>,
        reason: Reason,
        /// `ctx.preferences`: what the user chose for each preference the package declares.
        preferences: Preferences,
    },
    /// `views[name](ctx)`, for a page the user opened.
    View {
        name: String,
        page: Box<PageContext>,
        preferences: Preferences,
    },
    /// `actions[name](data)`, which the user chose on `page`.
    Action {
        name: String,
        data: serde_json::Value,
        page: Box<PageContext>,
        preferences: Preferences,
    },
    /// `events[name](event)`, for something Dupin watched on the package's behalf.
    Event {
        name: String,
        event: serde_json::Value,
        preferences: Preferences,
    },
    /// `filters[name](input)`, for one batch of a source Dupin composed it onto.
    Filter {
        name: String,
        input: Box<FilterInput>,
        preferences: Preferences,
    },
}

/// A call's `ctx.preferences`, a JSON object. Every part of one catalogue hands the package the
/// same ones, which may hold thousands of snippets, so Dupin encodes them once for all its calls
/// and the helper decodes them for each.
#[derive(Clone, Debug)]
pub struct Preferences(Form);

#[derive(Clone, Debug)]
enum Form {
    Encoded(Arc<RawValue>),
    Decoded(serde_json::Value),
}

impl Preferences {
    pub fn encode(preferences: &serde_json::Value) -> Self {
        let encoded = serde_json::value::to_raw_value(preferences).expect("JSON values always encode");
        Self(Form::Encoded(encoded.into()))
    }

    pub(crate) fn into_value(self) -> serde_json::Value {
        match self.0 {
            Form::Decoded(value) => value,
            Form::Encoded(json) => serde_json::from_str(json.get()).expect("encoded from a JSON value"),
        }
    }
}

impl Serialize for Preferences {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            Form::Encoded(json) => json.serialize(serializer),
            Form::Decoded(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Preferences {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde_json::Value::deserialize(deserializer).map(|value| Self(Form::Decoded(value)))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FromHelper {
    /// Sent once, after the sandbox is in place.
    Hello {
        version: u32,
        engine: String,
    },
    /// Sent instead of `hello` when the helper can't confine itself, before it exits.
    Refused {
        reason: String,
    },
    Done {
        id: u64,
        answer: Answer,
    },
    /// A host request of run `id`, numbered `call` from one, with its JSON arguments as the tail.
    Host {
        id: u64,
        call: u32,
        op: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    /// The tail holds each module's bytecode in this order.
    Compiled {
        modules: Vec<ModuleSpan>,
    },
    Checked {
        exports: Exports,
    },
    Rows {
        items: Vec<Row>,
    },
    Catalogue {
        page: CataloguePage,
    },
    View {
        view: ViewAnswer,
    },
    Acted {
        acted: Acted,
    },
    Handled {
        handled: Handled,
    },
    Filtered {
        hits: Vec<FilterHit>,
    },
    Cancelled,
    Failed {
        diagnostic: Diagnostic,
    },
    /// The run named a package without its bytecode, and the helper doesn't hold it.
    Missing,
}

/// The packages a helper holds, by the name Dupin gives each compile, least recently run first.
/// The helper keeps its packages in one, and Dupin keeps one of its own with nothing in it, fed
/// the same runs, so it sends a package's bytecode exactly when the helper doesn't hold it.
pub struct Held<T> {
    packages: Vec<(u64, T, usize)>,
    bytes: usize,
}

impl<T> Default for Held<T> {
    fn default() -> Self {
        Self {
            packages: Vec::new(),
            bytes: 0,
        }
    }
}

impl<T> Held<T> {
    /// The package held under `name`, which is now the most recently run.
    pub fn run(&mut self, name: u64) -> Option<&T> {
        let at = self.packages.iter().position(|(held, ..)| *held == name)?;
        let held = self.packages.remove(at);
        self.packages.push(held);
        self.packages.last().map(|(_, package, _)| package)
    }

    /// Holds `package`, of `bytes` of bytecode, under `name`, letting the least recently run go
    /// until it fits. One larger than [`MAX_HELD_BYTES`] is never held.
    pub fn hold(&mut self, name: u64, package: T, bytes: usize) {
        self.forget(name);
        if bytes > MAX_HELD_BYTES {
            return;
        }
        while self.bytes + bytes > MAX_HELD_BYTES {
            let (_, _, freed) = self.packages.remove(0);
            self.bytes -= freed;
        }
        self.packages.push((name, package, bytes));
        self.bytes += bytes;
    }

    /// Lets go of the package held under `name`; false if none was.
    pub fn forget(&mut self, name: u64) -> bool {
        let Some(at) = self.packages.iter().position(|(held, ..)| *held == name) else {
            return false;
        };
        let (_, _, freed) = self.packages.remove(at);
        self.bytes -= freed;
        true
    }
}

/// The spans of `modules`, and the parts of a tail that carries them.
pub fn spans<'a>(modules: impl Iterator<Item = (&'a str, &'a [u8])>) -> (Vec<ModuleSpan>, Vec<&'a [u8]>) {
    modules
        .map(|(name, bytes)| {
            (
                ModuleSpan {
                    name: name.into(),
                    len: bytes.len() as u32,
                },
                bytes,
            )
        })
        .unzip()
}

/// Cuts `tail` into the modules `spans` describe, which must cover it exactly.
pub fn split(spans: Vec<ModuleSpan>, tail: &[u8]) -> io::Result<Vec<(String, &[u8])>> {
    let mut rest = tail;
    let mut modules = Vec::with_capacity(spans.len());
    for span in spans {
        let len = span.len as usize;
        if len > rest.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "short package"));
        }
        let (bytes, after) = rest.split_at(len);
        modules.push((span.name, bytes));
        rest = after;
    }
    if !rest.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "long package"));
    }
    Ok(modules)
}

/// Writes one frame in as few writes as the socket takes, so the reader wakes once for it.
pub fn write(writer: &mut impl Write, message: &impl Serialize, tail: &[&[u8]]) -> io::Result<()> {
    let mut frame = vec![0; 8];
    serde_json::to_writer(&mut frame, message).map_err(io::Error::other)?;
    let header_len = frame.len() - 8;
    let tail_len: usize = tail.iter().map(|part| part.len()).sum();
    if header_len > MAX_HEADER_BYTES || tail_len > MAX_TAIL_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "message too large"));
    }
    frame[..4].copy_from_slice(&(header_len as u32).to_be_bytes());
    frame[4..8].copy_from_slice(&(tail_len as u32).to_be_bytes());
    let mut parts: Vec<IoSlice<'_>> = std::iter::once(&frame[..])
        .chain(tail.iter().copied())
        .filter(|part| !part.is_empty())
        .map(IoSlice::new)
        .collect();
    let mut parts = &mut parts[..];
    while !parts.is_empty() {
        match writer.write_vectored(parts) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => IoSlice::advance_slices(&mut parts, written),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    writer.flush()
}

/// The next frame's header and tail, or `None` at the end of the stream.
pub fn read<T: for<'de> Deserialize<'de>>(reader: &mut impl Read) -> io::Result<Option<(T, Vec<u8>)>> {
    let mut lengths = [0; 8];
    match reader.read_exact(&mut lengths) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let header_len = u32::from_be_bytes(lengths[..4].try_into().expect("four bytes")) as usize;
    let tail_len = u32::from_be_bytes(lengths[4..].try_into().expect("four bytes")) as usize;
    if header_len > MAX_HEADER_BYTES || tail_len > MAX_TAIL_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut header = vec![0; header_len];
    reader.read_exact(&mut header)?;
    let mut tail = vec![0; tail_len];
    reader.read_exact(&mut tail)?;
    let message =
        serde_json::from_slice(&header).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some((message, tail)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_oversized_ones_are_refused() {
        let mut bytes = Vec::new();
        let (modules, tail) = spans([("main.js", &b"abc"[..]), ("lib.js", &b"de"[..])].into_iter());
        let message = ToHelper::Run {
            id: 7,
            deadline_ms: 100,
            entry: "main.js".into(),
            package: None,
            modules: Some(modules),
            call: Call::Compile,
            environment: None,
        };
        write(&mut bytes, &message, &tail).unwrap();
        write(&mut bytes, &ToHelper::Cancel { id: 2 }, &[]).unwrap();
        let mut reader = &bytes[..];
        let (first, tail) = read::<ToHelper>(&mut reader).unwrap().unwrap();
        let ToHelper::Run {
            id: 7,
            modules: Some(modules),
            ..
        } = first
        else {
            panic!("{first:?}");
        };
        assert_eq!(
            split(modules, &tail).unwrap(),
            [
                ("main.js".to_owned(), &b"abc"[..]),
                ("lib.js".to_owned(), &b"de"[..])
            ]
        );
        assert!(matches!(
            read::<ToHelper>(&mut reader).unwrap().unwrap().0,
            ToHelper::Cancel { id: 2 }
        ));
        assert!(read::<ToHelper>(&mut reader).unwrap().is_none());

        let mut huge = Vec::new();
        huge.extend_from_slice(&u32::MAX.to_be_bytes());
        huge.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(
            read::<ToHelper>(&mut &huge[..]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut cut = Vec::new();
        write(&mut cut, &ToHelper::Cancel { id: 2 }, &[]).unwrap();
        cut.pop();
        assert!(read::<ToHelper>(&mut &cut[..]).is_err());
    }

    /// The most rows and fields a part may have, its text all control characters, which JSON
    /// writes at six bytes each.
    fn largest_catalogue_part() -> CataloguePage {
        use dupin_js_runtime::row::{MAX_PAGE_ROWS, Metadata, PageChange, RowAction, RowDetail, RowEffect};

        let control = |len: usize| "\u{1}".repeat(len);
        let mut rows: Vec<Row> = (0..MAX_PAGE_ROWS)
            .map(|index| Row {
                id: index.to_string(),
                title: "a".into(),
                subtitle: control(1),
                aliases: vec!["a".into(); 16],
                keywords: vec!["a".into(); 16],
                match_text: None,
                time: None,
                file: None,
                icon: Some("a.png".into()),
                image: None,
                section: Some("a".into()),
                detail: Some(RowDetail {
                    image: None,
                    markdown: String::new(),
                    metadata: vec![
                        Metadata {
                            label: "a".into(),
                            text: "a".into(),
                        };
                        12
                    ],
                }),
                actions: vec![
                    RowAction {
                        template: true,
                        ..RowAction::new("a", RowEffect::Copy { text: control(1) })
                    };
                    8
                ],
                web_search: None,
                accessories: Vec::new(),
                answer: None,
            })
            .collect();
        let mut left = MAX_PAGE_BYTES - rows.iter().map(Row::text_bytes).sum::<usize>();
        for row in &mut rows {
            let len = left.min(32 * 1024);
            row.detail.as_mut().expect("set above").markdown = control(len);
            left -= len;
        }
        assert_eq!(left, 0);
        CataloguePage {
            change: PageChange::Reset { rows },
            ttl: Some(1),
            cursor: Some("a".repeat(256)),
            more: true,
        }
    }

    #[test]
    fn the_largest_catalogue_part_fits_a_frame() {
        let page = largest_catalogue_part();
        page.validate().unwrap();
        let message = FromHelper::Done {
            id: u64::MAX,
            answer: Answer::Catalogue { page },
        };
        let mut bytes = Vec::new();
        write(&mut bytes, &message, &[]).unwrap();
        assert!(bytes.len() > 6 * MAX_PAGE_BYTES, "{}", bytes.len());
        let (read, _) = read::<FromHelper>(&mut &bytes[..]).unwrap().unwrap();
        assert!(matches!(
            read,
            FromHelper::Done {
                answer: Answer::Catalogue { .. },
                ..
            }
        ));
    }

    /// One of each message as its JSON reads, so a change of shape can't land without
    /// [`VERSION`] being raised with it.
    #[test]
    fn every_message_keeps_the_shape_its_version_names() {
        use dupin_js_runtime::row::{PageChange, RowImage};

        let row = Row {
            id: "a".into(),
            title: "A".into(),
            subtitle: String::new(),
            aliases: Vec::new(),
            keywords: Vec::new(),
            match_text: None,
            time: None,
            file: None,
            icon: None,
            image: Some(RowImage::Blob("h".into())),
            section: None,
            detail: None,
            actions: Vec::new(),
            web_search: None,
            accessories: Vec::new(),
            answer: None,
        };
        let web_search = Row {
            web_search: Some(dupin_js_runtime::web_search::WebSearch {
                alias: "g".into(),
                url: "https://example.com/?q={query}".into(),
                title: "G {query}".into(),
                fallback: Some(0),
            }),
            ..row.clone()
        };
        let span = || ModuleSpan {
            name: "main.js".into(),
            len: 1,
        };
        let calls = [
            Call::Compile,
            Call::Check,
            Call::Search {
                query: "q".into(),
                limit: 1,
            },
            Call::Items {
                cursor: Some("c".into()),
                reason: Reason::Invalidated,
                preferences: Preferences::encode(&serde_json::json!({"a": true})),
            },
            Call::View {
                name: "note".into(),
                page: Box::new(dupin_js_runtime::PageContext {
                    query: "q".into(),
                    state: serde_json::json!({"id": 7}),
                }),
                preferences: Preferences::encode(&serde_json::json!({})),
            },
            Call::Action {
                name: "run".into(),
                data: serde_json::Value::Null,
                page: Box::default(),
                preferences: Preferences::encode(&serde_json::json!({})),
            },
            Call::Event {
                name: "clipboard".into(),
                event: serde_json::json!({}),
                preferences: Preferences::encode(&serde_json::json!({})),
            },
            Call::Filter {
                name: "romaji".into(),
                input: Box::new(FilterInput::new(
                    dupin_js_runtime::FilterStage::Matcher,
                    Some("q".into()),
                    ["A".into()],
                )),
                preferences: Preferences::encode(&serde_json::json!({})),
            },
        ];
        let mut shapes: Vec<String> = calls
            .into_iter()
            .map(|call| ToHelper::Run {
                id: 1,
                deadline_ms: 2,
                entry: "main.js".into(),
                package: Some(5),
                modules: Some(vec![span()]),
                call,
                environment: Some(Environment { now: 3, seed: 4 }),
            })
            .chain([
                ToHelper::Run {
                    id: 1,
                    deadline_ms: 2,
                    entry: "main.js".into(),
                    package: Some(5),
                    modules: None,
                    call: Call::Check,
                    environment: None,
                },
                ToHelper::Cancel { id: 1 },
                ToHelper::Reply { id: 1, call: 2 },
            ])
            .map(|message| serde_json::to_string(&message).unwrap())
            .collect();
        let answers = [
            Answer::Compiled {
                modules: vec![span()],
            },
            Answer::Checked {
                exports: Exports {
                    search: true,
                    items: true,
                },
            },
            Answer::Rows {
                items: vec![row.clone()],
            },
            Answer::Catalogue {
                page: CataloguePage {
                    change: PageChange::Update {
                        prepend: Vec::new(),
                        append: vec![row.clone()],
                        replace: vec![web_search],
                        remove: vec!["b".into()],
                    },
                    ttl: Some(1),
                    cursor: Some("c".into()),
                    more: true,
                },
            },
            Answer::View {
                view: ViewAnswer::List {
                    rows: vec![Row {
                        accessories: vec![dupin_js_runtime::row::Accessory::Tag("t".into())],
                        answer: Some("1".into()),
                        actions: vec![dupin_js_runtime::row::RowAction::new(
                            "Open",
                            dupin_js_runtime::row::RowEffect::Push {
                                view: "note".into(),
                                state: Some(serde_json::json!(1)),
                            },
                        )],
                        ..row.clone()
                    }],
                    empty_text: Some("none".into()),
                },
            },
            Answer::View {
                view: ViewAnswer::Form(dupin_js_runtime::view::Form {
                    title: "F".into(),
                    fields: vec![dupin_js_runtime::view::Field {
                        id: "a".into(),
                        title: "A".into(),
                        kind: dupin_js_runtime::view::FieldKind::Select {
                            options: vec![dupin_js_runtime::view::SelectOption {
                                value: "x".into(),
                                title: "X".into(),
                            }],
                            default: "x".into(),
                        },
                    }],
                    submit: "save".into(),
                    submit_title: None,
                }),
            },
            Answer::Acted {
                acted: Acted {
                    invalidated: true,
                    effect: Some(dupin_js_runtime::ActionEffect::Toast { text: "t".into() }),
                },
            },
            Answer::Acted {
                acted: Acted {
                    invalidated: false,
                    effect: Some(dupin_js_runtime::ActionEffect::Push {
                        view: "note".into(),
                        title: None,
                        state: Some(serde_json::json!({"id": 7})),
                        toast: Some("t".into()),
                    }),
                },
            },
            Answer::Handled {
                handled: Handled { invalidated: true },
            },
            Answer::Filtered {
                hits: vec![
                    FilterHit {
                        index: 0,
                        score: Some(1),
                        positions: vec![0],
                    },
                    FilterHit {
                        index: 1,
                        score: None,
                        positions: Vec::new(),
                    },
                ],
            },
            Answer::Cancelled,
            Answer::Failed {
                diagnostic: Diagnostic::new("code", "message"),
            },
            Answer::Missing,
        ];
        shapes.extend(
            [
                FromHelper::Hello {
                    version: 0,
                    engine: "e".into(),
                },
                FromHelper::Refused { reason: "r".into() },
                FromHelper::Host {
                    id: 1,
                    call: 2,
                    op: "storage".into(),
                },
            ]
            .into_iter()
            .chain(
                answers
                    .into_iter()
                    .map(|answer| FromHelper::Done { id: 1, answer }),
            )
            .map(|message| serde_json::to_string(&message).unwrap()),
        );
        assert_eq!(
            (VERSION, shapes),
            (11, SHAPES.map(str::to_owned).to_vec()),
            "a message changed shape: raise VERSION and record the new shapes here"
        );
    }

    const SHAPES: [&str; 27] = [
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":[{"name":"main.js","len":1}],"call":{"kind":"compile"},"environment":{"now":3,"seed":4}}"#,
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":[{"name":"main.js","len":1}],"call":{"kind":"check"},"environment":{"now":3,"seed":4}}"#,
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":[{"name":"main.js","len":1}],"call":{"kind":"search","query":"q","limit":1},"environment":{"now":3,"seed":4}}"#,
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":[{"name":"main.js","len":1}],"call":{"kind":"items","cursor":"c","reason":"invalidated","preferences":{"a":true}},"environment":{"now":3,"seed":4}}"#,
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":[{"name":"main.js","len":1}],"call":{"kind":"view","name":"note","page":{"query":"q","state":{"id":7}},"preferences":{}},"environment":{"now":3,"seed":4}}"#,
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":[{"name":"main.js","len":1}],"call":{"kind":"action","name":"run","data":null,"page":{"query":"","state":null},"preferences":{}},"environment":{"now":3,"seed":4}}"#,
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":[{"name":"main.js","len":1}],"call":{"kind":"event","name":"clipboard","event":{},"preferences":{}},"environment":{"now":3,"seed":4}}"#,
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":[{"name":"main.js","len":1}],"call":{"kind":"filter","name":"romaji","input":{"stage":"matcher","query":"q","titles":["A"]},"preferences":{}},"environment":{"now":3,"seed":4}}"#,
        r#"{"kind":"run","id":1,"deadline_ms":2,"entry":"main.js","package":5,"modules":null,"call":{"kind":"check"},"environment":null}"#,
        r#"{"kind":"cancel","id":1}"#,
        r#"{"kind":"reply","id":1,"call":2}"#,
        r#"{"kind":"hello","version":0,"engine":"e"}"#,
        r#"{"kind":"refused","reason":"r"}"#,
        r#"{"kind":"host","id":1,"call":2,"op":"storage"}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"compiled","modules":[{"name":"main.js","len":1}]}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"checked","exports":{"search":true,"items":true}}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"rows","items":[{"id":"a","title":"A","image":"h"}]}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"catalogue","page":{"change":{"kind":"update","append":[{"id":"a","title":"A","image":"h"}],"replace":[{"id":"a","title":"A","image":"h","web_search":{"alias":"g","url":"https://example.com/?q={query}","title":"G {query}","fallback":0}}],"remove":["b"]},"ttl":1,"cursor":"c","more":true}}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"view","view":{"kind":"list","rows":[{"id":"a","title":"A","image":"h","actions":[{"title":"Open","effect":{"kind":"push","view":"note","state":1}}],"accessories":[{"tag":"t"}],"answer":"1"}],"empty_text":"none"}}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"view","view":{"kind":"form","title":"F","fields":[{"id":"a","title":"A","kind":{"type":"select","options":[{"value":"x","title":"X"}],"default":"x"}}],"submit":"save"}}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"acted","acted":{"invalidated":true,"effect":{"type":"toast","text":"t"}}}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"acted","acted":{"invalidated":false,"effect":{"type":"push","view":"note","state":{"id":7},"toast":"t"}}}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"handled","handled":{"invalidated":true}}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"filtered","hits":[{"index":0,"score":1,"positions":[0]},{"index":1}]}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"cancelled"}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"failed","diagnostic":{"code":"code","message":"message"}}}"#,
        r#"{"kind":"done","id":1,"answer":{"kind":"missing"}}"#,
    ];

    #[test]
    fn a_package_too_large_to_hold_or_let_go_since_is_not_held() {
        const MIB: usize = 1024 * 1024;
        let mut held = Held::default();
        held.hold(1, (), MAX_HELD_BYTES + 1);
        assert!(held.run(1).is_none());
        for name in [2, 3, 4] {
            held.hold(name, (), 3 * MIB);
        }
        assert!(
            held.run(2).is_none(),
            "three packages of 3 MiB don't fit in 8 MiB"
        );
        assert!(held.run(3).is_some());
        held.hold(2, (), 3 * MIB);
        assert!(held.run(4).is_none(), "4 was the least recently run");
        assert!(held.run(3).is_some() && held.run(2).is_some());
    }

    #[test]
    fn holding_a_package_again_counts_its_bytes_once() {
        let mut held = Held::default();
        for _ in 0..3 {
            held.hold(1, (), MAX_HELD_BYTES / 2);
        }
        held.hold(2, (), MAX_HELD_BYTES / 2);
        assert!(held.run(1).is_some() && held.run(2).is_some());
        assert!(held.forget(1) && !held.forget(1));
    }

    #[test]
    fn spans_must_cover_the_tail_exactly() {
        let span = |len| ModuleSpan {
            name: "main.js".into(),
            len,
        };
        assert!(split(vec![span(4)], b"abc").is_err());
        assert!(split(vec![span(2)], b"abc").is_err());
        assert!(split(vec![span(3)], b"abc").is_ok());
    }
}
