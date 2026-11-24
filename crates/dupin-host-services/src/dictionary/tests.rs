use super::*;

fn entry(headword: &str, dictionary: &str) -> Entry {
    Entry {
        headword: headword.into(),
        dictionary: Some(dictionary.into()),
        pronunciation: Some("rʌn".into()),
        groups: vec![Group {
            part_of_speech: Some("verb".into()),
            note: Some("(runs, running; past ran)".into()),
            senses: vec![Sense {
                definition: "move on foot faster than walking".into(),
                examples: vec!["she ran to the station".into()],
                senses: vec![Sense {
                    definition: "(of a machine) operate".into(),
                    examples: vec!["the engine runs on diesel".into()],
                    senses: Vec::new(),
                }],
            }],
        }],
        sections: vec![Section {
            title: "Origin".into(),
            text: "Old English".into(),
        }],
        truncated: false,
    }
}

fn service() -> Dictionary<Fixture> {
    Dictionary::new(Fixture {
        entries: vec![
            entry("run", "Oxford"),
            entry("run", "Thesaurus"),
            entry("walk", "Oxford"),
        ],
    })
}

#[test]
fn lookups_return_structured_entries_from_the_named_dictionaries() {
    let all = service().lookup(&Lookup::new(" Run ", None).unwrap()).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0], entry("run", "Oxford"));

    let named = service()
        .lookup(&Lookup::new("run", Some(vec!["Thesaurus".into()])).unwrap())
        .unwrap();
    assert_eq!(named, [entry("run", "Thesaurus")]);

    assert!(
        service()
            .lookup(&Lookup::new("fly", None).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn entries_serialize_for_javascript() {
    let json = serde_json::to_value(entry("run", "Oxford")).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "headword": "run",
            "dictionary": "Oxford",
            "pronunciation": "rʌn",
            "groups": [{
                "partOfSpeech": "verb",
                "note": "(runs, running; past ran)",
                "senses": [{
                    "definition": "move on foot faster than walking",
                    "examples": ["she ran to the station"],
                    "senses": [{
                        "definition": "(of a machine) operate",
                        "examples": ["the engine runs on diesel"],
                        "senses": [],
                    }],
                }],
            }],
            "sections": [{ "title": "Origin", "text": "Old English" }],
            "truncated": false,
        })
    );
}

#[test]
fn invalid_lookups_are_refused_before_the_backend() {
    for (term, dictionaries) in [
        ("", None),
        ("   ", None),
        ("a\u{7}b", None),
        (&*"x".repeat(MAX_TERM_CHARS + 1), None),
        ("run", Some(vec![])),
        ("run", Some(vec![" ".into()])),
        ("run", Some(vec!["d".into(); MAX_DICTIONARIES + 1])),
        ("run", Some(vec!["d".repeat(MAX_DICTIONARY_NAME_BYTES + 1)])),
    ] {
        assert!(
            matches!(
                Lookup::new(term, dictionaries.clone()),
                Err(ServiceError::Invalid(_))
            ),
            "{term:?} {dictionaries:?}"
        );
    }
    assert!(Lookup::new(&"é".repeat(MAX_TERM_CHARS), None).is_ok());
}

#[test]
fn results_are_cut_to_their_bounds_and_marked() {
    let long = "é".repeat(MAX_TEXT_BYTES);
    let sense = Sense {
        definition: long.clone(),
        examples: vec!["e".into(); MAX_EXAMPLES + 1],
        senses: vec![Sense {
            definition: "nested".into(),
            examples: Vec::new(),
            senses: vec![Sense::default()],
        }],
    };
    let big = Entry {
        headword: "run".into(),
        groups: vec![
            Group {
                part_of_speech: None,
                note: None,
                senses: vec![sense; MAX_SENSES + 1],
            };
            MAX_GROUPS + 1
        ],
        sections: vec![Section::default(); MAX_SECTIONS + 1],
        ..Entry::default()
    };
    let service = Dictionary::new(Fixture {
        entries: vec![big; MAX_ENTRIES + 1],
    });
    let entries = service.lookup(&Lookup::new("run", None).unwrap()).unwrap();
    assert_eq!(entries.len(), MAX_ENTRIES);
    let entry = &entries[0];
    assert!(entry.truncated);
    assert_eq!(entry.groups.len(), MAX_GROUPS);
    assert_eq!(entry.sections.len(), MAX_SECTIONS);
    let senses = &entry.groups[0].senses;
    assert_eq!(senses.len(), MAX_SENSES);
    assert!(senses[0].definition.len() <= MAX_TEXT_BYTES);
    assert!(long.starts_with(&senses[0].definition));
    assert_eq!(senses[0].examples.len(), MAX_EXAMPLES);
    assert!(senses[0].senses[0].senses.is_empty(), "sub-senses nest one level");

    let small = service_with(entry_with_text("short"));
    assert!(!small[0].truncated);
}

fn entry_with_text(text: &str) -> Entry {
    Entry {
        headword: "run".into(),
        sections: vec![Section {
            title: "Origin".into(),
            text: text.into(),
        }],
        ..Entry::default()
    }
}

fn service_with(entry: Entry) -> Vec<Entry> {
    Dictionary::new(Fixture { entries: vec![entry] })
        .lookup(&Lookup::new("run", None).unwrap())
        .unwrap()
}
