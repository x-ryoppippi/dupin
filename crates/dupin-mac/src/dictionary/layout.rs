//! Lays out Dictionary Services' plain-text definitions as a [`Detail`].
//!
//! The text has no markup, only conventions that vary by dictionary: Oxford-style entries run
//! `headword | pronunciation | part of speech 1 sense • sub-sense: example 2 …` and end with
//! labeled parts in capitals; thesauruses drop the pronunciation; Japanese dictionaries number
//! senses ①②③ and English–Japanese ones mark examples with `▸`. Whatever is not recognized
//! stays as plain text, so no layout loses content.

use dupin_core::Detail;
use dupin_core::detail::{Block, Inline, escape_markdown};
use dupin_host_services::dictionary::{Entry, Group, Section, Sense};

/// Parts that Oxford-style dictionaries start with a label in capitals, in the order they appear.
const PARTS: [(&str, &str); 6] = [
    ("PHRASAL VERBS", "Phrasal verbs"),
    ("PHRASES", "Phrases"),
    ("DERIVATIVES", "Derivatives"),
    ("USAGE", "Usage"),
    ("ORIGIN", "Origin"),
    ("WORD LINKS", "Word links"),
];

/// Labels inside a sense, shown in bold.
const SENSE_LABELS: [(&str, &str); 2] = [("ANTONYMS", "Antonyms"), ("SYNONYMS", "Synonyms")];

/// Longer labels come first so `auxiliary verb` is not read as `verb`.
const PARTS_OF_SPEECH: [&str; 26] = [
    "auxiliary verb",
    "modal verb",
    "phrasal verb",
    "plural noun",
    "proper noun",
    "combining form",
    "cardinal number",
    "ordinal number",
    "predeterminer",
    "noun",
    "verb",
    "adjective",
    "adverb",
    "pronoun",
    "preposition",
    "conjunction",
    "determiner",
    "exclamation",
    "interjection",
    "abbreviation",
    "symbol",
    "prefix",
    "suffix",
    "contraction",
    "article",
    "numeral",
];

const CIRCLED: [char; 20] = [
    '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩', '⑪', '⑫', '⑬', '⑭', '⑮', '⑯', '⑰', '⑱', '⑲', '⑳',
];

/// The definition laid out: the pronunciation between the headword's bars as metadata, each
/// part of speech under a subheading with its numbered senses as an ordered list, sub-senses
/// after `•` nested under their sense with examples in italics, and each labeled part under a
/// heading.
pub(super) fn detail(term: &str, definition: &str) -> Detail {
    let (pronunciation, markdown) = laid_out(term, definition);
    let detail = Detail::new(term, &markdown);
    match pronunciation {
        Some(pronunciation) => detail.with_metadata("Pronunciation", pronunciation),
        None => detail,
    }
}

/// The pronunciation and the rest of the definition as Markdown, as [`detail`] shows them.
fn laid_out<'a>(term: &str, definition: &'a str) -> (Option<&'a str>, String) {
    let (pronunciation, body) = pronunciation(term, definition)
        .map_or((None, definition), |(pronunciation, body)| {
            (Some(pronunciation), body)
        });
    let mut markdown = String::new();
    let mut rest = body;
    let mut heading = None;
    loop {
        let next = PARTS
            .iter()
            .filter_map(|&(label, title)| Some((label_start(rest, label)?, label, title)))
            .min_by_key(|&(at, ..)| at);
        let (text, following) = match next {
            Some((at, label, title)) => (&rest[..at], Some((&rest[at + label.len()..], title))),
            None => (rest, None),
        };
        match heading {
            Some(title) => {
                markdown.push_str(&format!("## {title}\n\n"));
                unnumbered(&mut markdown, text, 0, false);
            }
            None => entry(&mut markdown, term, text),
        }
        let Some((following, title)) = following else {
            break;
        };
        rest = following;
        heading = Some(title);
    }
    markdown.truncate(markdown.trim_end().len());
    (pronunciation, markdown)
}

/// `pronunciation` and the rest from `headword | pronunciation | rest`, when the definition
/// opens that way.
fn pronunciation<'a>(term: &str, definition: &'a str) -> Option<(&'a str, &'a str)> {
    let mut parts = definition.splitn(3, " | ");
    let (headword, pronunciation, rest) = (parts.next()?, parts.next()?, parts.next()?);
    let short = |part: &str| part.chars().count() <= 60 && !part.contains('|');
    // Some dictionaries split the headword into syllables: `hap･py`.
    let headword: String = headword
        .chars()
        .filter(|c| !matches!(c, '･' | '·' | '‧'))
        .collect();
    (short(&headword) && short(pronunciation) && headword.to_lowercase().starts_with(&term.to_lowercase()))
        .then(|| (pronunciation.trim(), rest))
}

/// Where `label` starts as a word of its own in `text`.
fn label_start(text: &str, label: &str) -> Option<usize> {
    text.match_indices(label).map(|(at, _)| at).find(|&at| {
        let before = text[..at].chars().next_back();
        let after = text[at + label.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && after.is_none_or(char::is_whitespace)
    })
}

/// The main part of an entry: one group per part of speech, after whatever comes before the
/// first of them.
fn entry(markdown: &mut String, term: &str, text: &str) {
    let groups = parts_of_speech(term, text);
    let lead = &text[..groups.first().map_or(text.len(), |&(at, _)| at)];
    // A thesaurus repeats the headword before the first part of speech; the title shows it.
    if groups.is_empty() || !lead.trim().eq_ignore_ascii_case(term.trim()) {
        senses(markdown, lead);
    }
    for (i, &(at, label)) in groups.iter().enumerate() {
        let end = groups.get(i + 1).map_or(text.len(), |&(next, _)| next);
        let mut title = label.chars();
        let title: String = title
            .next()
            .into_iter()
            .flat_map(char::to_uppercase)
            .chain(title)
            .collect();
        markdown.push_str(&format!("### {title}\n\n"));
        senses(markdown, &text[at + label.len()..end]);
    }
}

/// Where each part-of-speech label starts in `text`, with the label. A label counts only at
/// the start, straight after the headword, or after a sentence ends, so a definition that
/// mentions a noun or a verb stays whole.
fn parts_of_speech(term: &str, text: &str) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for (at, _) in text.char_indices() {
        if text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let Some(label) = PARTS_OF_SPEECH.into_iter().find(|label| {
            text[at..].starts_with(label)
                && text[at + label.len()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        }) else {
            continue;
        };
        let before = text[..at].trim();
        let opens = before.is_empty()
            || before.eq_ignore_ascii_case(term.trim())
            || before.ends_with(['.', '!', '?', '’', '”', '\'', '"']);
        if opens {
            found.push((at, label));
        }
    }
    found
}

/// Numbered senses as an ordered list after any text before the first; text without numbers
/// as a paragraph and its sub-senses.
fn senses(markdown: &mut String, text: &str) {
    let numbers = sense_numbers(text);
    let Some(&(first, _)) = numbers.first() else {
        unnumbered(markdown, text, 0, true);
        return;
    };
    unnumbered(markdown, &text[..first], 0, true);
    for (i, &(_, start)) in numbers.iter().enumerate() {
        let end = numbers.get(i + 1).map_or(text.len(), |&(next, _)| next);
        let marker = format!("{}. ", i + 1);
        let mut chunks = bullets(&text[start..end]).into_iter();
        let (line, examples) = inline(chunks.next().unwrap_or_default(), true);
        markdown.push_str(&format!("{marker}{line}\n"));
        let indent = marker.len();
        list(markdown, &examples, indent);
        for chunk in chunks {
            sub_sense(markdown, chunk, indent, true);
        }
    }
    markdown.push('\n');
}

/// Text as a paragraph, then each sub-sense after a `•` as a list item, indented by `indent`.
fn unnumbered(markdown: &mut String, text: &str, indent: usize, examples: bool) {
    if text.trim().is_empty() {
        return;
    }
    let starts_with_bullet = text.trim_start().starts_with('•');
    let mut chunks = bullets(text).into_iter();
    if !starts_with_bullet && let Some(first) = chunks.next() {
        let (line, examples) = inline(first, examples);
        if !line.is_empty() {
            markdown.push_str(&format!("{line}\n\n"));
        }
        list(markdown, &examples, indent);
    }
    for chunk in chunks {
        sub_sense(markdown, chunk, indent, examples);
    }
    markdown.push('\n');
}

fn sub_sense(markdown: &mut String, chunk: &str, indent: usize, examples: bool) {
    let (line, chunk_examples) = inline(chunk, examples);
    if line.is_empty() && chunk_examples.is_empty() {
        return;
    }
    markdown.push_str(&format!("{:indent$}- {line}\n", ""));
    list(markdown, &chunk_examples, indent + 2);
}

fn list(markdown: &mut String, items: &[String], indent: usize) {
    for item in items {
        markdown.push_str(&format!("{:indent$}- {item}\n", ""));
    }
}

/// `text` split before each sub-sense marker: a `•` that starts a word (a `•` inside a word
/// joins a citation such as `〈枕草子•223〉`), or a circled katakana such as `㋐`.
fn bullets(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    for (at, c) in text.char_indices() {
        let opens = match c {
            '•' => text[..at].chars().next_back().is_none_or(char::is_whitespace),
            '\u{32D0}'..='\u{32FE}' => true,
            _ => false,
        };
        if opens && at > start {
            chunks.push(&text[start..at]);
            start = at;
        } else if opens {
            start = at;
        }
    }
    chunks.push(&text[start..]);
    chunks
        .into_iter()
        .map(|chunk| chunk.strip_prefix('•').unwrap_or(chunk).trim())
        .filter(|chunk| !chunk.is_empty())
        .collect()
}

/// Where each numbered sense's marker and text start. Senses count up from one: circled
/// numbers when the text has `①`, otherwise the next number standing as a word of its own
/// outside an example or a bracket.
fn sense_numbers(text: &str) -> Vec<(usize, usize)> {
    if text.contains(CIRCLED[0]) {
        let mut found = Vec::new();
        for (at, c) in text.char_indices() {
            if CIRCLED.get(found.len()) == Some(&c) {
                found.push((at, at + c.len_utf8()));
            }
        }
        return found;
    }
    let mut found: Vec<(usize, usize)> = Vec::new();
    let mut next = String::from("1");
    // A number in brackets is a cross-reference, such as `(→ quite 2 第3例)`, and one after a
    // colon or `▸` is part of an example, such as `: 2 plus 2 is 4.`. Both end with the
    // sentence, so an unclosed bracket does not hide the senses after it.
    let mut depth = 0usize;
    let mut example = false;
    for (at, c) in text.char_indices() {
        let ends_sentence = match c {
            '.' => !is_decimal_point(text, at),
            '!' | '?' | '。' | '．' => true,
            '•' => text[..at].chars().next_back().is_none_or(char::is_whitespace),
            _ => false,
        };
        match c {
            _ if ends_sentence => {
                depth = 0;
                example = false;
            }
            '(' | '[' | '（' | '［' => depth += 1,
            ')' | ']' | '）' | '］' => depth = depth.saturating_sub(1),
            ':' | '：' | '▸' => example = true,
            _ => {}
        }
        let after = at + next.len();
        if depth > 0
            || example
            || !text[at..].starts_with(&next)
            || !text[after..].starts_with(char::is_whitespace)
        {
            continue;
        }
        let mut before = text[..at].chars().rev();
        let stands_alone = match before.next() {
            None => true,
            Some(c) if c.is_whitespace() => true,
            // `…「人」.2 〖…` runs a number straight after the full stop.
            Some('.') => !before.next().is_some_and(|c| c.is_ascii_digit()),
            Some(_) => false,
        };
        if stands_alone {
            found.push((at, after));
            next = (found.len() + 1).to_string();
        }
    }
    found
}

fn is_decimal_point(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_some_and(|c| c.is_ascii_digit())
        && text[at + 1..].starts_with(|c: char| c.is_ascii_digit())
}

/// A sense as Markdown and the examples marked with `▸` after it. With `examples`, the text after
/// a definition's colon is italic, keeping grammar labels such as `[with object] :` upright.
fn inline(text: &str, examples: bool) -> (String, Vec<String>) {
    let mut pieces = text.split('▸');
    let main = tidy(pieces.next().unwrap_or_default());
    let marked = pieces
        .map(tidy)
        .filter(|piece| !piece.is_empty())
        .map(|piece| emphasis(&piece))
        .collect();
    let line = match main.split_once(": ").filter(|_| examples) {
        Some((definition, uses)) => format!(
            "{}: {}",
            plain(definition),
            uses.split(" | ").map(example).collect::<Vec<_>>().join(" | ")
        ),
        None => plain(&main),
    };
    (line, marked)
}

/// An example in italics after its grammar label, if it has one.
fn example(text: &str) -> String {
    let label = ["] : ", ") : "].into_iter().find_map(|end| {
        let opens = if end.starts_with(']') { '[' } else { '(' };
        text.starts_with(opens)
            .then(|| text.find(end))
            .flatten()
            .map(|at| at + end.len())
    });
    match label {
        Some(at) => format!("{}{}", plain(&text[..at]), emphasis(&text[at..])),
        None => emphasis(text),
    }
}

fn emphasis(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!("*{}*", plain(text))
    }
}

/// `text` escaped, with labels such as `ANTONYMS` in bold.
fn plain(text: &str) -> String {
    let mut markdown = String::new();
    let mut rest = text;
    while let Some((at, label, title)) = SENSE_LABELS
        .iter()
        .filter_map(|&(label, title)| Some((label_start(rest, label)?, label, title)))
        .min_by_key(|&(at, ..)| at)
    {
        markdown.push_str(&escape_markdown(&rest[..at]));
        markdown.push_str(&format!("**{title}**"));
        rest = &rest[at + label.len()..];
    }
    markdown.push_str(&escape_markdown(rest));
    markdown
}

fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The definition with its layout as [`detail`] has it, as Markdown and as data: each `###`
/// part of speech becomes a group whose numbered items are senses, `•` items under a sense its
/// sub-senses and italic `▸` items its examples, and each `##` part a section. A group's text
/// before its numbered senses is its note.
pub(super) fn structure(term: &str, definition: &str) -> Entry {
    let (pronunciation, markdown) = laid_out(term, definition);
    let detail = Detail::new(term, &markdown);
    let mut entry = Entry {
        headword: detail.title,
        pronunciation: pronunciation.map(str::to_owned),
        text: definition.to_owned(),
        markdown,
        truncated: detail.body.truncated,
        ..Entry::default()
    };
    let mut blocks = detail.body.blocks.into_iter().peekable();
    let mut part_of_speech = None;
    loop {
        let mut body = Vec::new();
        while let Some(block) = blocks.next_if(|block| !matches!(block, Block::Heading { .. })) {
            body.push(block);
        }
        if part_of_speech.is_some() || !body.is_empty() {
            entry.groups.push(group(part_of_speech.take(), body));
        }
        match blocks.next() {
            Some(Block::Heading { level: 2, text }) => {
                let mut lines = Vec::new();
                while let Some(block) = blocks.next_if(|block| !matches!(block, Block::Heading { .. })) {
                    plain_text(&block, &mut lines);
                }
                entry.sections.push(Section {
                    title: text.text,
                    text: lines.join("\n"),
                });
            }
            Some(Block::Heading { text, .. }) => part_of_speech = Some(text.text),
            Some(_) | None => break,
        }
    }
    entry
}

fn group(part_of_speech: Option<String>, body: Vec<Block>) -> Group {
    let numbered = body
        .iter()
        .any(|block| matches!(block, Block::List { start: Some(_), .. }));
    let mut note = Vec::new();
    let mut senses: Vec<Sense> = Vec::new();
    let mut before_numbers = numbered;
    for block in body {
        match block {
            Block::Paragraph(text) if before_numbers => note.push(text.text),
            Block::List {
                start: Some(_),
                items,
            } => {
                before_numbers = false;
                senses.extend(items.iter().map(|item| item_sense(item)));
            }
            Block::List { start: None, items } => {
                for item in &items {
                    let sense = item_sense(item);
                    match senses.last_mut() {
                        Some(parent) => nest(parent, sense),
                        None => senses.push(sense),
                    }
                }
            }
            Block::Paragraph(text) => senses.push(sense(&text)),
            Block::Code(text) => senses.push(Sense {
                definition: text,
                ..Sense::default()
            }),
            Block::Heading { .. } => {}
        }
    }
    Group {
        part_of_speech,
        note: (!note.is_empty()).then(|| note.join("\n")),
        senses,
    }
}

/// A list item's first paragraph as a sense, with the items nested under it.
fn item_sense(blocks: &[Block]) -> Sense {
    let mut parent: Option<Sense> = None;
    for block in blocks {
        match block {
            Block::Paragraph(text) if parent.is_none() => parent = Some(sense(text)),
            Block::List { items, .. } => {
                let parent = parent.get_or_insert_with(Sense::default);
                for item in items {
                    nest(parent, item_sense(item));
                }
            }
            other => {
                let mut lines = Vec::new();
                plain_text(other, &mut lines);
                let parent = parent.get_or_insert_with(Sense::default);
                parent.definition = [std::mem::take(&mut parent.definition), lines.join(" ")]
                    .join(" ")
                    .trim()
                    .to_owned();
            }
        }
    }
    parent.unwrap_or_default()
}

/// An item that is only an example joins its parent's examples; anything else is a sub-sense.
fn nest(parent: &mut Sense, child: Sense) {
    if child.definition.is_empty() && child.senses.is_empty() {
        parent.examples.extend(child.examples);
    } else {
        parent.senses.push(child);
    }
}

/// A sense line is `definition: example | label : example`, with the examples in italics.
fn sense(text: &Inline) -> Sense {
    let Some(first) = text.runs.iter().find(|run| run.style.emphasis) else {
        return Sense {
            definition: text.text.trim().to_owned(),
            ..Sense::default()
        };
    };
    Sense {
        definition: text.text[..first.range.start]
            .trim_end()
            .trim_end_matches(':')
            .trim()
            .to_owned(),
        examples: text.text[first.range.start..]
            .split(" | ")
            .map(str::trim)
            .filter(|example| !example.is_empty())
            .map(str::to_owned)
            .collect(),
        senses: Vec::new(),
    }
}

fn plain_text(block: &Block, lines: &mut Vec<String>) {
    match block {
        Block::Heading { text, .. } | Block::Paragraph(text) => lines.push(text.text.clone()),
        Block::List { items, .. } => items.iter().flatten().for_each(|block| plain_text(block, lines)),
        Block::Code(code) => lines.push(code.clone()),
    }
}

#[cfg(test)]
mod tests;
