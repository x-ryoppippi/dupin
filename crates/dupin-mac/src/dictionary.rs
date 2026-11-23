//! Dictionary Services lookups for the `define` launcher mode.

use std::ptr::NonNull;

use dupin_core::detail::escape_markdown;
use dupin_core::{Action, Batch, Detail, Item, ItemAction, Payload, Provider, ProviderError, Request, View};
use objc2_core_foundation::{CFRange, CFRetained, CFString};

#[repr(C)]
struct DCSDictionary {
    _private: [u8; 0],
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C-unwind" {
    fn DCSCopyTextDefinition(
        dictionary: Option<&DCSDictionary>,
        text_string: &CFString,
        range: CFRange,
    ) -> Option<NonNull<CFString>>;
}

pub struct DictionaryProvider {
    lookup: fn(&str) -> Option<String>,
}

impl Default for DictionaryProvider {
    fn default() -> Self {
        Self { lookup: definition }
    }
}

impl DictionaryProvider {
    /// Fixed definitions in Dictionary Services' layout, for headless renders that must not
    /// depend on the dictionaries this Mac has.
    pub fn fixtures() -> Self {
        Self {
            lookup: fixture_definition,
        }
    }
}

fn fixture_definition(term: &str) -> Option<String> {
    Some(match term {
        "serendipity" => "serendipity | ˌsɛr(ə)nˈdɪpɪti | noun the happy accident of finding something good \
            without looking for it: a moment of pure serendipity. ORIGIN mid 18th century: from Serendip, \
            an old name for Sri Lanka, after a fairy tale whose heroes kept making such discoveries."
            .into(),
        "run" => "run | rʌn | verb 1 move quickly on foot, faster than walking: she ran to the station. \
            • (of a machine) operate: the engine runs on diesel. • (of a road or path) extend in a \
            direction: the path runs along the river. 2 be in charge of: he runs a small bakery. noun an \
            act of running: a morning run. PHRASES on the run trying to avoid being caught. ORIGIN Old \
            English."
            .into(),
        _ => return None,
    })
}

impl Provider for DictionaryProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        let term = request.query.trim();
        if request.limit == 0 || term.is_empty() || request.is_cancelled() {
            return Ok(());
        }
        let Some(definition) = (self.lookup)(term) else {
            return Ok(());
        };
        if !request.is_cancelled() {
            let detail = detail(term, &definition);
            let item = item(term, definition);
            let id = item.id.clone();
            emit(Batch::from(vec![item]).with_detail(id, detail));
        }
        Ok(())
    }
}

fn definition(term: &str) -> Option<String> {
    let text = CFString::from_str(term);
    // CFRange uses CFString's UTF-16 code-unit offsets, not Rust's UTF-8 byte length.
    let range = full_range(&text);
    // SAFETY: `text` remains alive for the call, and Dictionary Services documents a null
    // dictionary parameter as the supported way to use active Dictionary.app dictionaries.
    let definition = unsafe { DCSCopyTextDefinition(None, &text, range) }?;
    // SAFETY: DCSCopyTextDefinition follows Copy ownership and returns a retained CFString.
    let definition = unsafe { CFRetained::from_raw(definition) };
    let definition = definition.to_string();
    (!definition.trim().is_empty()).then_some(definition)
}

fn full_range(text: &CFString) -> CFRange {
    CFRange {
        location: 0,
        length: text.length(),
    }
}

fn item(term: &str, definition: String) -> Item {
    Item {
        id: format!("dictionary:{term}"),
        payload: Payload::text(definition.clone()),
        view: View::Detail {
            title: term.to_owned(),
            body: definition,
        },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: None,
        alternate: None,
        actions: vec![ItemAction::new(
            "open-dictionary",
            "Open in Dictionary",
            Action::OpenUrl(dictionary_url(term)),
        )],
    }
}

/// Parts Oxford-style dictionaries start with a label in capitals, in the order they appear.
const PARTS: [(&str, &str); 5] = [
    ("PHRASAL VERBS", "Phrasal verbs"),
    ("PHRASES", "Phrases"),
    ("DERIVATIVES", "Derivatives"),
    ("USAGE", "Usage"),
    ("ORIGIN", "Origin"),
];

/// The definition laid out: the pronunciation between the headword's bars as metadata, each
/// labeled part under a heading, and the senses after each `•` as a list.
fn detail(term: &str, definition: &str) -> Detail {
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
            .filter_map(|&(label, title)| Some((part_start(rest, label)?, label, title)))
            .min_by_key(|&(at, ..)| at);
        let (text, following) = match next {
            Some((at, label, title)) => (&rest[..at], Some((&rest[at + label.len()..], title))),
            None => (rest, None),
        };
        if let Some(title) = heading {
            markdown.push_str(&format!("## {title}\n\n"));
        }
        senses(&mut markdown, text);
        let Some((following, title)) = following else {
            break;
        };
        rest = following;
        heading = Some(title);
    }
    let detail = Detail::new(term, &markdown);
    match pronunciation {
        Some(pronunciation) => detail.with_metadata("Pronunciation", pronunciation),
        None => detail,
    }
}

/// `pronunciation` and the rest from `headword | pronunciation | rest`, when the definition
/// opens that way.
fn pronunciation<'a>(term: &str, definition: &'a str) -> Option<(&'a str, &'a str)> {
    let mut parts = definition.splitn(3, " | ");
    let (headword, pronunciation, rest) = (parts.next()?, parts.next()?, parts.next()?);
    let short = |part: &str| part.chars().count() <= 60 && !part.contains('|');
    (short(headword) && short(pronunciation) && headword.to_lowercase().starts_with(&term.to_lowercase()))
        .then(|| (pronunciation.trim(), rest))
}

/// Where `label` starts as a word of its own in `text`.
fn part_start(text: &str, label: &str) -> Option<usize> {
    text.match_indices(label)
        .map(|(at, _)| at)
        .find(|&at| text[..at].ends_with(' ') && text[at + label.len()..].starts_with(' '))
}

fn senses(markdown: &mut String, text: &str) {
    let mut chunks = text.split('•').map(str::trim).filter(|chunk| !chunk.is_empty());
    let line = |chunk: &str| escape_markdown(&chunk.split_whitespace().collect::<Vec<_>>().join(" "));
    if let Some(first) = chunks.next() {
        if text.trim_start().starts_with('•') {
            markdown.push_str(&format!("- {}\n", line(first)));
        } else {
            markdown.push_str(&format!("{}\n\n", line(first)));
        }
    }
    for chunk in chunks {
        markdown.push_str(&format!("- {}\n", line(chunk)));
    }
    markdown.push('\n');
}

fn dictionary_url(term: &str) -> String {
    format!("dict://{}", crate::workspace::encode_url_component(term))
}

#[cfg(test)]
mod tests {
    fn detail_body(item: &dupin_core::Item) -> &str {
        let dupin_core::View::Detail { body, .. } = &item.view else {
            panic!("expected detail")
        };
        body
    }
    use super::*;
    use dupin_core::detail::{Block, Metadata};

    fn chance(term: &str) -> Option<String> {
        (term == "serendipity").then(|| "The occurrence of events by chance.".into())
    }

    #[test]
    fn provider_returns_the_definition_from_its_lookup() {
        let provider = DictionaryProvider { lookup: chance };
        let request = Request::detached("serendipity", 1);
        let mut batches = Vec::new();
        provider
            .search(&request, &mut |items| batches.push(items))
            .unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0][0].display_title(), "serendipity");
        assert_eq!(detail_body(&batches[0][0]), "The occurrence of events by chance.");
    }

    fn texts(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|block| match block {
                Block::Heading { level, text } => format!("h{level} {}", text.text),
                Block::Paragraph(text) => text.text.clone(),
                Block::List { items, .. } => items
                    .iter()
                    .map(|item| format!("- {}", texts(item).join(" ")))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Block::Code(code) => format!("`{code}`"),
            })
            .collect()
    }

    #[test]
    fn a_definition_becomes_a_detail_with_its_pronunciation_parts_and_senses() {
        let detail = detail(
            "Cat",
            "cat 1 | kat | noun 1 a small mammal • a wild animal: a marbled cat. • *slang* a man | \
             the cat. PHRASES cat and mouse a series [of] moves. ORIGIN Old English catt.",
        );
        assert_eq!(detail.title, "Cat");
        assert_eq!(
            detail.metadata,
            [Metadata {
                label: "Pronunciation".into(),
                value: "kat".into()
            }]
        );
        assert_eq!(
            texts(&detail.body.blocks),
            [
                "noun 1 a small mammal",
                "- a wild animal: a marbled cat.\n- *slang* a man | the cat.",
                "h2 Phrases",
                "cat and mouse a series [of] moves.",
                "h2 Origin",
                "Old English catt.",
            ]
        );
    }

    #[test]
    fn a_definition_without_a_headword_line_keeps_all_its_text() {
        for definition in [
            "UN abbreviation United Nations.",
            "other | word | text",
            "Japanese 日本 | にほん | text with ORIGINAL words",
        ] {
            let detail = detail("UN", definition);
            assert!(detail.metadata.is_empty(), "{definition}");
            assert_eq!(texts(&detail.body.blocks), [definition], "{definition}");
        }
    }

    #[test]
    fn the_provider_shows_its_result_as_a_detail() {
        let provider = DictionaryProvider { lookup: chance };
        let mut batches = Vec::new();
        provider
            .search_batches(&Request::detached("serendipity", 1), &mut |batch| {
                batches.push(batch)
            })
            .unwrap();
        let [batch] = batches.as_slice() else {
            panic!("expected one batch")
        };
        assert_eq!(batch.section, None);
        let detail = &batch.details[&batch.items[0].id];
        assert_eq!(
            texts(&detail.body.blocks),
            ["The occurrence of events by chance."]
        );
    }

    #[test]
    fn the_fixtures_cover_senses_and_labeled_parts() {
        let definition = fixture_definition("run").unwrap();
        let blocks = texts(&detail("run", &definition).body.blocks);
        assert_eq!(blocks.len(), 6, "{blocks:?}");
        assert!(blocks[1].starts_with("- (of a machine)") && blocks[1].contains("\n- (of a road"));
        assert_eq!(blocks[2], "h2 Phrases");
        assert_eq!(blocks[4], "h2 Origin");
        assert!(fixture_definition("other").is_none());
    }

    #[test]
    fn zero_limit_does_not_lookup_or_emit() {
        let provider = DictionaryProvider {
            lookup: |_| panic!("zero limit must skip lookup"),
        };
        provider
            .search(&Request::detached("serendipity", 0), &mut |_| {
                panic!("zero limit must not emit")
            })
            .unwrap();
    }

    #[test]
    fn definition_result_displays_the_definition_and_opens_dictionary() {
        let item = item("serendipity", "The occurrence of events by chance.".into());
        assert!(!item.record_usage);
        assert_eq!(item.display_title(), "serendipity");
        assert_eq!(detail_body(&item), "The occurrence of events by chance.");
        assert_eq!(item.primary, None);
        assert_eq!(
            item.action("open-dictionary"),
            Some(&Action::OpenUrl("dict://serendipity".into()))
        );
        assert_eq!(item.alternate, None);
    }

    #[test]
    fn dictionary_url_escapes_url_delimiters_and_unicode() {
        let value = dictionary_url("a b/#é");
        assert_eq!(value, "dict://a%20b%2F%23%C3%A9");
        assert!(
            objc2_foundation::NSURL::URLWithString(&objc2_foundation::NSString::from_str(&value)).is_some()
        );
    }

    #[test]
    fn dictionary_range_uses_utf16_code_units() {
        let term = CFString::from_str("😀");
        assert_eq!(term.length(), 2);
        assert_eq!(
            full_range(&term),
            CFRange {
                location: 0,
                length: 2
            }
        );
    }
}
