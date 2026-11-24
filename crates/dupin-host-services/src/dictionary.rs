//! The `dictionary` capability: looking a term up in the dictionaries this Mac has.
//!
//! Entries have the structure the native Dictionary feature lays out (headword,
//! pronunciation, senses grouped by part of speech with their examples, and labeled sections
//! such as Origin), so a guest renders them without parsing dictionary text. Lookups need no
//! OS permission.

use serde::Serialize;

use crate::service::{ServiceError, bounded};

pub const MAX_TERM_CHARS: usize = 200;
pub const MAX_DICTIONARIES: usize = 16;
pub const MAX_DICTIONARY_NAME_BYTES: usize = 256;
pub const MAX_ENTRIES: usize = 8;
pub const MAX_GROUPS: usize = 32;
pub const MAX_SENSES: usize = 64;
pub const MAX_EXAMPLES: usize = 16;
pub const MAX_SECTIONS: usize = 16;
/// Each headword, definition, example and section text.
pub const MAX_TEXT_BYTES: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub headword: String,
    /// The dictionary the entry came from, when the backend can tell.
    pub dictionary: Option<String>,
    pub pronunciation: Option<String>,
    pub groups: Vec<Group>,
    pub sections: Vec<Section>,
    /// Whether the entry was cut to fit the bounds.
    pub truncated: bool,
}

/// The senses under one part of speech, or under none before the first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub part_of_speech: Option<String>,
    /// What comes before the numbered senses, such as inflections.
    pub note: Option<String>,
    pub senses: Vec<Sense>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sense {
    pub definition: String,
    pub examples: Vec<String>,
    /// Sub-senses, one level deep.
    pub senses: Vec<Sense>,
}

/// A labeled part such as Phrases or Origin, as plain text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub title: String,
    pub text: String,
}

/// A validated lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lookup {
    term: String,
    dictionaries: Option<Vec<String>>,
}

impl Lookup {
    pub fn new(term: &str, dictionaries: Option<Vec<String>>) -> Result<Self, ServiceError> {
        let term = term.trim();
        if term.is_empty() {
            return Err(ServiceError::Invalid("the term is empty".into()));
        }
        if term.chars().count() > MAX_TERM_CHARS || term.chars().any(char::is_control) {
            return Err(ServiceError::Invalid(format!(
                "a term is at most {MAX_TERM_CHARS} characters, with no control characters"
            )));
        }
        if let Some(names) = &dictionaries {
            if names.is_empty() || names.len() > MAX_DICTIONARIES {
                return Err(ServiceError::Invalid(format!(
                    "name 1–{MAX_DICTIONARIES} dictionaries, or leave them out"
                )));
            }
            if names
                .iter()
                .any(|name| name.trim().is_empty() || name.len() > MAX_DICTIONARY_NAME_BYTES)
            {
                return Err(ServiceError::Invalid(format!(
                    "a dictionary name is 1–{MAX_DICTIONARY_NAME_BYTES} bytes"
                )));
            }
        }
        Ok(Self {
            term: term.to_owned(),
            dictionaries,
        })
    }

    pub fn term(&self) -> &str {
        &self.term
    }

    pub fn dictionaries(&self) -> Option<&[String]> {
        self.dictionaries.as_deref()
    }
}

/// Where definitions come from: Dictionary Services on macOS, [`Fixture`] in tests.
pub trait Dictionaries: Send + Sync {
    /// Entries for `term` from the named dictionaries, or from the user's active ones.
    fn lookup(&self, term: &str, dictionaries: Option<&[String]>) -> Result<Vec<Entry>, ServiceError>;
}

pub struct Dictionary<D> {
    backend: D,
}

impl<D: Dictionaries> Dictionary<D> {
    pub fn new(backend: D) -> Self {
        Self { backend }
    }

    /// The entries for the lookup, cut to the bounds. No entry is not an error.
    pub fn lookup(&self, lookup: &Lookup) -> Result<Vec<Entry>, ServiceError> {
        let mut entries = self.backend.lookup(lookup.term(), lookup.dictionaries())?;
        entries.truncate(MAX_ENTRIES);
        Ok(entries.into_iter().map(Entry::bounded).collect())
    }
}

impl Entry {
    fn bounded(self) -> Self {
        let mut cut = Cut(self.truncated);
        let headword = cut.text(self.headword);
        let dictionary = self.dictionary.map(|value| cut.text(value));
        let pronunciation = self.pronunciation.map(|value| cut.text(value));
        let groups = cut
            .list(self.groups, MAX_GROUPS)
            .into_iter()
            .map(|group| Group {
                part_of_speech: group.part_of_speech.map(|value| cut.text(value)),
                note: group.note.map(|value| cut.text(value)),
                senses: cut.senses(group.senses, true),
            })
            .collect();
        let sections = cut
            .list(self.sections, MAX_SECTIONS)
            .into_iter()
            .map(|section| Section {
                title: cut.text(section.title),
                text: cut.text(section.text),
            })
            .collect();
        Self {
            headword,
            dictionary,
            pronunciation,
            groups,
            sections,
            truncated: cut.0,
        }
    }
}

/// Bounds an entry, remembering whether anything was cut.
struct Cut(bool);

impl Cut {
    fn text(&mut self, value: String) -> String {
        self.0 |= value.len() > MAX_TEXT_BYTES;
        bounded(value, MAX_TEXT_BYTES)
    }

    fn list<T>(&mut self, mut items: Vec<T>, max: usize) -> Vec<T> {
        self.0 |= items.len() > max;
        items.truncate(max);
        items
    }

    fn senses(&mut self, senses: Vec<Sense>, nested: bool) -> Vec<Sense> {
        self.list(senses, MAX_SENSES)
            .into_iter()
            .map(|sense| {
                let senses = if nested {
                    self.senses(sense.senses, false)
                } else {
                    self.0 |= !sense.senses.is_empty();
                    Vec::new()
                };
                Sense {
                    definition: self.text(sense.definition),
                    examples: self
                        .list(sense.examples, MAX_EXAMPLES)
                        .into_iter()
                        .map(|example| self.text(example))
                        .collect(),
                    senses,
                }
            })
            .collect()
    }
}

/// Fixed entries, each answering its headword (ignoring case) from the dictionary it names.
#[derive(Clone, Debug, Default)]
pub struct Fixture {
    pub entries: Vec<Entry>,
}

impl Dictionaries for Fixture {
    fn lookup(&self, term: &str, dictionaries: Option<&[String]>) -> Result<Vec<Entry>, ServiceError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.headword.to_lowercase() == term.to_lowercase())
            .filter(|entry| {
                dictionaries.is_none_or(|names| {
                    entry
                        .dictionary
                        .as_ref()
                        .is_some_and(|dictionary| names.contains(dictionary))
                })
            })
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests;
