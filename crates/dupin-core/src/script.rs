use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

const MAX_ARGS: usize = 64;
const MAX_ARG_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 8 * 1024;
const MAX_STDERR_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputMode {
    Silent,
    #[default]
    Inline,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRun {
    pub path: PathBuf,
    pub title: String,
    pub args: Vec<String>,
    pub mode: OutputMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArgKind {
    Required,
    Optional,
    ZeroOrMore,
    OneOrMore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgSpec {
    name: String,
    kind: ArgKind,
}

impl ArgSpec {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_optional(&self) -> bool {
        matches!(self.kind, ArgKind::Optional | ArgKind::ZeroOrMore)
    }

    pub fn is_variadic(&self) -> bool {
        matches!(self.kind, ArgKind::ZeroOrMore | ArgKind::OneOrMore)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub title: String,
    pub icon: Option<PathBuf>,
    pub mode: OutputMode,
    pub arguments: Vec<ArgSpec>,
}

impl Header {
    pub fn parse(bytes: &[u8], fallback: &str) -> Result<(Self, Vec<String>), String> {
        let mut header = Self {
            title: fallback.into(),
            icon: None,
            mode: OutputMode::Inline,
            arguments: vec![],
        };
        let mut warnings = Vec::new();
        let mut scalars = HashSet::new();
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(8192)]);
        for (index, line) in text.lines().take(64).enumerate() {
            let line = line.trim();
            if line.is_empty() || (index == 0 && line.starts_with("#!")) {
                continue;
            }
            let Some(comment) = ["#", "//", "--", ";"]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
            else {
                break;
            };
            let Some(metadata) = comment.trim_start().strip_prefix("@dupin.") else {
                continue;
            };
            let (key, value) = metadata.split_once(char::is_whitespace).unwrap_or((metadata, ""));
            let value = value.trim();
            let problem = |message: &str| format!("line {}: {message}", index + 1);
            if ["title", "icon", "output"].contains(&key) && !scalars.insert(key.to_owned()) {
                return Err(problem(&format!("duplicate {key}")));
            }
            match key {
                "title" if !value.is_empty() && !value.chars().any(char::is_control) => {
                    header.title = value.into()
                }
                "icon" if !value.is_empty() && !value.chars().any(char::is_control) => {
                    header.icon = Some(value.into())
                }
                "title" | "icon" => return Err(problem(&format!("{key} needs a non-empty value"))),
                "output" => {
                    header.mode = match value.split(" #").next().unwrap_or("").trim() {
                        "silent" => OutputMode::Silent,
                        "inline" => OutputMode::Inline,
                        "full" => OutputMode::Full,
                        _ => return Err(problem(&format!("output “{value}” — use silent, inline or full"))),
                    }
                }
                "argument" => {
                    let value = value.split(" #").next().unwrap_or("").trim();
                    let (name, kind) = if let Some(name) = value.strip_suffix("...") {
                        (name, ArgKind::ZeroOrMore)
                    } else if let Some(name) = value.strip_suffix('+') {
                        (name, ArgKind::OneOrMore)
                    } else if let Some(name) = value.strip_suffix('?') {
                        (name, ArgKind::Optional)
                    } else {
                        (value, ArgKind::Required)
                    };
                    let valid = name
                        .as_bytes()
                        .first()
                        .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
                        && name
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
                    if !valid || header.arguments.iter().any(|arg| arg.name == name) {
                        return Err(problem("invalid or duplicate argument name"));
                    }
                    if header.arguments.len() >= MAX_ARGS {
                        return Err(problem("too many arguments"));
                    }
                    if header
                        .arguments
                        .iter()
                        .any(|arg| matches!(arg.kind, ArgKind::ZeroOrMore | ArgKind::OneOrMore))
                    {
                        return Err(problem("a variadic argument must be last"));
                    }
                    if kind == ArgKind::Required
                        && header.arguments.iter().any(|arg| arg.kind == ArgKind::Optional)
                    {
                        return Err(problem("required arguments must come before optional arguments"));
                    }
                    header.arguments.push(ArgSpec {
                        name: name.into(),
                        kind,
                    });
                }
                _ => warnings.push(problem(&format!("unknown header key {key}"))),
            }
        }
        Ok((header, warnings))
    }

    pub fn synopsis(&self) -> String {
        self.arguments
            .iter()
            .map(|arg| match arg.kind {
                ArgKind::Required => arg.name.clone(),
                ArgKind::Optional => format!("[{}]", arg.name),
                ArgKind::ZeroOrMore => format!("[{}…]", arg.name),
                ArgKind::OneOrMore => format!("{}…", arg.name),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn argument_values<'a>(
        &'a self,
        mut remaining: &'a [String],
    ) -> impl Iterator<Item = (&'a ArgSpec, &'a [String])> {
        let mut required = self.arguments.iter().filter(|arg| !arg.is_optional()).count();
        self.arguments.iter().map(move |arg| {
            required -= usize::from(!arg.is_optional());
            let count = match arg.kind {
                ArgKind::Required => usize::from(!remaining.is_empty()),
                ArgKind::Optional => usize::from(remaining.len() > required),
                ArgKind::ZeroOrMore | ArgKind::OneOrMore => remaining.len(),
            };
            let (values, rest) = remaining.split_at(count);
            remaining = rest;
            (arg, values)
        })
    }

    pub fn bind_args(&self, args: &[String]) -> Result<Vec<(String, Vec<String>)>, String> {
        if args.len() > MAX_ARGS || args.iter().map(String::len).sum::<usize>() > MAX_ARG_BYTES {
            return Err("Too many argument bytes or values".into());
        }
        let mut used = 0;
        let mut bound = Vec::new();
        for (arg, values) in self.argument_values(args) {
            if values.is_empty() && !arg.is_optional() {
                return Err(format!("Needs {}", arg.name));
            }
            used += values.len();
            bound.push((arg.name.clone(), values.to_vec()));
        }
        if used != args.len() {
            return Err("Too many arguments".into());
        }
        Ok(bound)
    }
}

pub fn split_args(input: &str) -> Result<Vec<String>, String> {
    if input.len() > MAX_ARG_BYTES {
        return Err("Arguments exceed 64 KiB".into());
    }
    let mut args = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => word.push(ch),
            (Some('"'), '\\') if matches!(chars.peek(), Some('"' | '\\')) => word.push(chars.next().unwrap()),
            (Some('"'), _) => word.push(ch),
            (None, '\'' | '"') => {
                quote = Some(ch);
                started = true;
            }
            (None, '\\') => {
                word.push(chars.next().ok_or("Trailing backslash")?);
                started = true;
            }
            (None, ch) if ch.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
        if args.len() > MAX_ARGS {
            return Err("At most 64 arguments are allowed".into());
        }
    }
    if let Some(quote) = quote {
        let kind = if quote == '\'' { "single" } else { "double" };
        return Err(format!("Unclosed {kind} quote"));
    }
    if started {
        args.push(word);
    }
    if args.len() > MAX_ARGS {
        return Err("At most 64 arguments are allowed".into());
    }
    Ok(args)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: Arc<str>,
}

// The shared string allocates its reference counts even for an empty line.
const OUTPUT_LINE_OVERHEAD: usize = std::mem::size_of::<OutputLine>() + 2 * std::mem::size_of::<usize>();

#[derive(Clone, Debug, Default)]
pub struct OutputBuffer {
    lines: VecDeque<OutputLine>,
    bytes: usize,
    dropped: usize,
    last_stdout: Option<String>,
    stderr_tail: String,
}

impl OutputBuffer {
    pub fn push(&mut self, stream: Stream, raw: &[u8]) {
        let mut text = strip_sgr(&String::from_utf8_lossy(raw));
        truncate_line(&mut text);
        if stream == Stream::Stdout && !text.trim().is_empty() {
            self.last_stdout = Some(text.clone());
        }
        if stream == Stream::Stderr {
            self.stderr_tail.push_str(&text);
            self.stderr_tail.push('\n');
            if self.stderr_tail.len() > MAX_STDERR_BYTES {
                let mut boundary = self.stderr_tail.len() - MAX_STDERR_BYTES;
                while !self.stderr_tail.is_char_boundary(boundary) {
                    boundary += 1;
                }
                self.stderr_tail.drain(..boundary);
            }
        }
        self.bytes += text.len() + OUTPUT_LINE_OVERHEAD;
        self.lines.push_back(OutputLine {
            stream,
            text: text.into(),
        });
        while self.bytes > MAX_OUTPUT_BYTES {
            let line = self.lines.pop_front().unwrap();
            self.bytes -= line.text.len() + OUTPUT_LINE_OVERHEAD;
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    pub fn lines(&self) -> &VecDeque<OutputLine> {
        &self.lines
    }
    pub fn dropped(&self) -> usize {
        self.dropped
    }
    pub fn last_stdout(&self) -> Option<&str> {
        self.last_stdout.as_deref()
    }
    pub fn stderr_tail(&self) -> &str {
        &self.stderr_tail
    }
}

fn truncate_line(text: &mut String) {
    if text.len() > MAX_LINE_BYTES {
        let marker = "… [line truncated]";
        let mut boundary = MAX_LINE_BYTES - marker.len();
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
        text.push_str(marker);
    }
}

#[derive(Default)]
pub struct LineDecoder {
    line: Vec<u8>,
    truncated: bool,
    carriage_return: bool,
}

impl LineDecoder {
    pub fn feed(&mut self, bytes: &[u8], stream: Stream, output: &mut OutputBuffer) {
        for &byte in bytes {
            if byte == b'\n' && self.carriage_return {
                self.carriage_return = false;
                continue;
            }
            self.carriage_return = byte == b'\r';
            if matches!(byte, b'\n' | b'\r') {
                self.finish(stream, output);
            } else if self.line.len() < MAX_LINE_BYTES {
                self.line.push(byte);
            } else {
                self.truncated = true;
            }
        }
    }

    pub fn finish(&mut self, stream: Stream, output: &mut OutputBuffer) {
        if self.truncated {
            self.line.extend_from_slice(b" [line truncated]");
        }
        output.push(stream, &self.line);
        self.line.clear();
        self.truncated = false;
    }

    pub fn has_pending(&self) -> bool {
        !self.line.is_empty() || self.truncated
    }
}

fn strip_sgr(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut clean = String::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(b"\x1b[") {
            let mut end = at + 2;
            while end < bytes.len() && (bytes[end].is_ascii_digit() || matches!(bytes[end], b';' | b':')) {
                end += 1;
            }
            if bytes.get(end) == Some(&b'm') {
                at = end + 1;
                continue;
            }
        }
        let ch = text[at..].chars().next().unwrap();
        clean.push(ch);
        at += ch.len_utf8();
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_accept_comment_styles_and_stop_before_executable_content() {
        for prefix in ["#", "//", "--", ";"] {
            let source = format!(
                "#!/bin/sh\n{prefix} @dupin.title A title\n{prefix} @dupin.output full\n{prefix} @dupin.future value\necho run\n{prefix} @dupin.title ignored\n"
            );
            let (header, warnings) = Header::parse(source.as_bytes(), "fallback").unwrap();
            assert_eq!(header.title, "A title");
            assert_eq!(header.mode, OutputMode::Full);
            assert_eq!(warnings.len(), 1);
        }
        let source = format!("{}# @dupin.title ignored", "# filler\n".repeat(64));
        assert_eq!(
            Header::parse(source.as_bytes(), "fallback").unwrap().0.title,
            "fallback"
        );
        let source = format!("# {}\n# @dupin.output full", "x".repeat(8192));
        assert_eq!(
            Header::parse(source.as_bytes(), "fallback").unwrap().0.mode,
            OutputMode::Inline
        );
    }

    #[test]
    fn malformed_metadata_and_argument_order_are_rejected() {
        for source in [
            "# @dupin.title a\n# @dupin.title b",
            "# @dupin.output loud",
            "# @dupin.title",
            "# @dupin.argument bad name",
            "# @dupin.argument a\n# @dupin.argument a?",
            "# @dupin.argument optional?\n# @dupin.argument required",
            "# @dupin.argument files...\n# @dupin.argument other?",
        ] {
            assert!(Header::parse(source.as_bytes(), "fallback").is_err(), "{source}");
        }
        let (header, _) = Header::parse(
            b"# @dupin.output inline # default mode\n# @dupin.argument name # required",
            "run",
        )
        .unwrap();
        assert_eq!(header.synopsis(), "name");
    }

    #[test]
    fn arguments_are_literal_and_quoted_empty_values_are_retained() {
        assert_eq!(
            split_args(r#"one 'two words' "three words" '' four\ five $HOME * ~ "a\"b" "c\\d""#).unwrap(),
            [
                "one",
                "two words",
                "three words",
                "",
                "four five",
                "$HOME",
                "*",
                "~",
                "a\"b",
                "c\\d"
            ]
        );
        for input in ["'unfinished", "\"unfinished", "trailing\\"] {
            assert!(split_args(input).is_err());
        }
        assert!(split_args(&"x ".repeat(65)).is_err());
        assert!(split_args(&"日".repeat(MAX_ARG_BYTES / 3 + 1)).is_err());
        assert_eq!(split_args("  ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn binding_reserves_required_variadic_values_after_an_optional_argument() {
        let (header, _) = Header::parse(
            b"# @dupin.argument interface\n# @dupin.argument note?\n# @dupin.argument files+",
            "run",
        )
        .unwrap();
        assert_eq!(header.synopsis(), "interface [note] files…");
        assert!(header.bind_args(&[]).is_err());
        assert!(header.bind_args(&["en0".into()]).is_err());
        let partial = ["en0".to_owned()];
        let preview: Vec<_> = header.argument_values(&partial).collect();
        assert_eq!(preview[0].1, ["en0"]);
        assert!(preview[1].0.is_optional() && preview[1].1.is_empty());
        assert!(preview[2].0.is_variadic() && !preview[2].0.is_optional() && preview[2].1.is_empty());
        let bound = header.bind_args(&["en0".into(), "file".into()]).unwrap();
        assert!(bound[1].1.is_empty());
        assert_eq!(bound[2].1, ["file"]);
        let bound = header
            .bind_args(&["en0".into(), "note".into(), "a".into(), "b".into()])
            .unwrap();
        assert_eq!(bound[1].1, ["note"]);
        assert_eq!(bound[2].1, ["a", "b"]);
        let (header, _) = Header::parse(b"# @dupin.argument names...", "run").unwrap();
        assert!(header.bind_args(&[]).is_ok());
        let (header, _) = Header::parse(b"# @dupin.argument name", "run").unwrap();
        assert!(header.bind_args(&["a".into(), "b".into()]).is_err());
    }

    #[test]
    fn output_decoding_handles_split_crlf_invalid_utf8_and_sgr() {
        let mut output = OutputBuffer::default();
        let mut decoder = LineDecoder::default();
        decoder.feed(b"\x1b[31mred\x1b[0m\r", Stream::Stdout, &mut output);
        decoder.feed(b"\nnext\ninvalid \xff", Stream::Stdout, &mut output);
        assert!(decoder.has_pending());
        decoder.finish(Stream::Stdout, &mut output);
        assert_eq!(
            output
                .lines
                .iter()
                .map(|line| line.text.as_ref())
                .collect::<Vec<_>>(),
            ["red", "next", "invalid �"]
        );
        assert_eq!(output.last_stdout(), Some("invalid �"));
        output.push(Stream::Stdout, b"  ");
        assert_eq!(output.last_stdout(), Some("invalid �"));
        output.push(Stream::Stderr, b"failure");
        assert_eq!(output.stderr_tail(), "failure\n");
    }

    #[test]
    fn output_and_reader_memory_stay_bounded_for_long_and_empty_lines() {
        let mut output = OutputBuffer::default();
        let mut decoder = LineDecoder::default();
        for _ in 0..640 {
            decoder.feed(&[b'x'; 8192], Stream::Stdout, &mut output);
            assert!(decoder.line.len() <= MAX_LINE_BYTES);
        }
        decoder.feed(b"\n", Stream::Stdout, &mut output);
        assert!(output.lines.back().unwrap().text.ends_with("[line truncated]"));
        assert!(output.lines.back().unwrap().text.len() <= MAX_LINE_BYTES);
        for _ in 0..40_000 {
            output.push(Stream::Stdout, b"");
        }
        assert!(output.bytes <= MAX_OUTPUT_BYTES);
        assert!(output.dropped() > 0);
        for _ in 0..300 {
            output.push(Stream::Stderr, "日".repeat(4000).as_bytes());
        }
        assert!(output.stderr_tail().len() <= MAX_STDERR_BYTES);
        assert!(output.bytes <= MAX_OUTPUT_BYTES);
        assert!(output.lines.iter().all(|line| line.text.len() <= MAX_LINE_BYTES));
    }
}
