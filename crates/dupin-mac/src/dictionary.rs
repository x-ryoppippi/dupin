//! Dictionary Services lookups, laid out for the extension `dictionary` capability.

use std::ptr::NonNull;

use dupin_host_services::ServiceError;
use dupin_host_services::dictionary::{Dictionaries, Entry};
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

/// The dictionaries turned on in Dictionary's settings, or fixed definitions in their layout.
pub struct SystemDictionary {
    lookup: fn(&str) -> Option<String>,
}

impl Default for SystemDictionary {
    fn default() -> Self {
        Self { lookup: definition }
    }
}

impl SystemDictionary {
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

impl Dictionaries for SystemDictionary {
    fn lookup(&self, term: &str, dictionaries: Option<&[String]>) -> Result<Vec<Entry>, ServiceError> {
        if dictionaries.is_some() {
            return Err(ServiceError::Unsupported(
                "macOS looks terms up in the dictionaries turned on in Dictionary's settings, \
                 not in named ones"
                    .into(),
            ));
        }
        Ok((self.lookup)(term)
            .map(|definition| layout::structure(term, &definition))
            .into_iter()
            .collect())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_returns_the_layout_as_data() {
        use dupin_host_services::dictionary::{Dictionary, Lookup};

        let service = Dictionary::new(SystemDictionary::fixtures());
        let [entry] = service
            .lookup(&Lookup::new("serendipity", None).unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(entry.headword, "serendipity");
        assert_eq!(entry.pronunciation.as_deref(), Some("ˌsɛr(ə)nˈdɪpɪti"));
        assert_eq!(entry.groups[0].part_of_speech.as_deref(), Some("Noun"));
        assert_eq!(
            entry.groups[0].senses[0].examples,
            ["a moment of pure serendipity."]
        );
        assert_eq!(entry.sections[0].title, "Origin");
        assert!(entry.text.starts_with("serendipity | "));
        assert!(entry.markdown.starts_with("### Noun\n\n"), "{}", entry.markdown);
        assert!(
            service
                .lookup(&Lookup::new("unknown", None).unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            service.lookup(&Lookup::new("run", Some(vec!["Oxford".into()])).unwrap()),
            Err(ServiceError::Unsupported(_))
        ));
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
