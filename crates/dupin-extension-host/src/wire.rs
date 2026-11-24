//! Messages between Dupin and its extension host, over one Unix socket. A frame is two
//! big-endian `u32` lengths, then a JSON header and a binary tail of those lengths. A `run`
//! carries the package in its tail, module after module: its source to compile, or the bytecode
//! a compile returned. A `compiled` answer carries that bytecode the same way. While a run is
//! going, the helper may send `host` requests, each with its JSON arguments as the tail, and Dupin
//! answers each with a `reply` whose tail is the JSON result.

use std::io::{self, Read, Write};

use dupin_js_runtime::row::MAX_PAGE_BYTES;
use dupin_js_runtime::{Acted, CataloguePage, Diagnostic, Exports, Reason, Row};
use serde::{Deserialize, Serialize};

/// Raised whenever a message changes shape. The helper states its version first, and Dupin
/// stops a helper that states another.
pub const VERSION: u32 = 4;
/// Above the largest answer, part of a catalogue with [`MAX_PAGE_BYTES`] of text. JSON writes a
/// control character, which copy text and Markdown may hold, as six bytes (`\u0001`), and the
/// rest covers the field names and punctuation of the most rows and fields a part may have.
const MAX_HEADER_BYTES: usize = 6 * MAX_PAGE_BYTES + 2 * 1024 * 1024;
/// A package's bytecode, from at most 4 MiB of JavaScript.
pub const MAX_TAIL_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToHelper {
    /// The tail holds each module in `modules`' order. The helper keeps nothing once it answers.
    Run {
        id: u64,
        /// Milliseconds the run may take from when the helper reads it.
        deadline_ms: u64,
        entry: String,
        modules: Vec<ModuleSpan>,
        call: Call,
    },
    /// Stop run `id` if it is still running.
    Cancel { id: u64 },
    /// Dupin's answer to request `call` of run `id`, as JSON in the tail.
    Reply { id: u64, call: u32 },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleSpan {
    pub name: String,
    pub len: u32,
}

#[derive(Debug, Serialize, Deserialize)]
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
    },
    /// `actions[name](data)`, which the user chose.
    Action {
        name: String,
        data: serde_json::Value,
    },
    /// `events[name](event)`, for something Dupin watched on the package's behalf.
    Event {
        name: String,
        event: serde_json::Value,
    },
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
    Acted {
        acted: Acted,
    },
    Handled,
    Cancelled,
    Failed {
        diagnostic: Diagnostic,
    },
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

pub fn write(writer: &mut impl Write, message: &impl Serialize, tail: &[&[u8]]) -> io::Result<()> {
    let header = serde_json::to_vec(message).map_err(io::Error::other)?;
    let tail_len: usize = tail.iter().map(|part| part.len()).sum();
    if header.len() > MAX_HEADER_BYTES || tail_len > MAX_TAIL_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "message too large"));
    }
    let mut frame = Vec::with_capacity(8 + header.len());
    frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(tail_len as u32).to_be_bytes());
    frame.extend_from_slice(&header);
    writer.write_all(&frame)?;
    for part in tail {
        writer.write_all(part)?;
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
            modules,
            call: Call::Compile,
        };
        write(&mut bytes, &message, &tail).unwrap();
        write(&mut bytes, &ToHelper::Cancel { id: 2 }, &[]).unwrap();
        let mut reader = &bytes[..];
        let (first, tail) = read::<ToHelper>(&mut reader).unwrap().unwrap();
        let ToHelper::Run { id: 7, modules, .. } = first else {
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
                keywords: vec!["a".into(); 16],
                icon: Some("a.png".into()),
                section: Some("a".into()),
                detail: Some(RowDetail {
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
                        title: "a".into(),
                        effect: RowEffect::Copy { text: control(1) },
                    };
                    8
                ],
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
