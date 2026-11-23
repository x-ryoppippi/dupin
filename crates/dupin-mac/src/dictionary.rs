//! Dictionary Services lookups for the `define` launcher mode.

use std::ptr::NonNull;

use dupin_core::{Action, Batch, Item, ItemAction, Payload, Provider, ProviderError, Request, View};
use objc2_core_foundation::{CFRange, CFRetained, CFString};

mod layout;

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
        "run" => "run | rʌn | verb (runs, running; past ran; past participle run) 1 [no object] move on \
            foot faster than walking: she ran to the station | he ran the last few yards. • (of a machine) \
            operate: the engine runs on diesel. • [with object] cause to operate: run the dishwasher twice a \
            week. 2 [with object] be in charge of: he runs a small bakery. 3 extend in a direction: the path \
            runs along the river. noun 1 an act of running: a morning run. 2 a journey on a regular route: \
            the school run. PHRASES on the run trying to avoid being caught. ORIGIN Old English."
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
            let detail = layout::detail(term, &definition);
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
    use dupin_core::detail::Block;

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
        let [Block::Paragraph(text)] = detail.body.blocks.as_slice() else {
            panic!("expected one paragraph: {:?}", detail.body.blocks)
        };
        assert_eq!(text.text, "The occurrence of events by chance.");
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
