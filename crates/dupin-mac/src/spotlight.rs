//! File search through the Spotlight index (`NSMetadataQuery`), matching file and display
//! names and streaming results in the order Spotlight finds them.

use std::path::PathBuf;
use std::time::Duration;

use dupin_core::fuzzy;
use dupin_core::{Item, Provider, ProviderError, Request};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSArray, NSDate, NSDefaultRunLoopMode, NSMetadataItem, NSMetadataItemPathKey, NSMetadataQuery,
    NSMetadataQueryLocalComputerScope, NSPredicate, NSRunLoop, NSString,
};

/// How long the worker's run loop turns between looks at the query: short enough that a
/// cancel lands within a frame or two and the first results show at once, long enough not to spin.
const SLICE: Duration = Duration::from_millis(16);

/// What a Spotlight search matches the query words against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// File and display names, anywhere in the name.
    Names,
    /// The text Spotlight indexed from inside files, by word prefix.
    Contents,
}

/// Spotlight's file search. Its `search` must run on a thread it can turn the run loop of,
/// which a [`Searcher`](dupin_core::Searcher)'s dedicated worker is.
pub struct SpotlightProvider {
    target: Target,
}

impl SpotlightProvider {
    pub fn new(target: Target) -> Self {
        SpotlightProvider { target }
    }
}

impl Provider for SpotlightProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        autoreleasepool(|_| run_query(self.target, request, emit))
    }
}

fn run_query(
    target: Target,
    request: &Request,
    emit: &mut dyn FnMut(Vec<Item>),
) -> Result<(), ProviderError> {
    let Some(predicate) = predicate(target, &request.query) else {
        return Ok(());
    };
    let query = NSMetadataQuery::new();
    query.setPredicate(Some(&predicate));
    // SAFETY: The scope constant is an immutable NSString AppKit exports; the property takes an
    // array of such scope strings.
    unsafe {
        let scopes: Retained<NSArray> =
            NSArray::from_slice(&[NSMetadataQueryLocalComputerScope as &AnyObject]);
        query.setSearchScopes(&scopes);
    }
    query.setNotificationBatchingInterval(SLICE.as_secs_f64());
    if !query.startQuery() {
        return Err(ProviderError);
    }

    let run_loop = NSRunLoop::currentRunLoop();
    let mut seen = 0;
    loop {
        let until = NSDate::dateWithTimeIntervalSinceNow(SLICE.as_secs_f64());
        // SAFETY: The default mode constant is a valid run loop mode.
        if !run_loop.runMode_beforeDate(unsafe { NSDefaultRunLoopMode }, &until) {
            // No input sources yet: nothing would wake the loop, so don't spin.
            std::thread::sleep(SLICE);
        }
        if request.is_cancelled() {
            break;
        }
        // Freeze the result list while reading it, so indices stay put.
        query.disableUpdates();
        let gathering = query.isGathering();
        let count = query.resultCount().min(request.limit);
        let batch: Vec<Item> = (seen..count)
            .filter_map(|i| item(&query.resultAtIndex(i), &request.query))
            .collect();
        query.enableUpdates();
        seen = count;
        if !batch.is_empty() {
            emit(batch);
        }
        if !gathering || seen >= request.limit {
            break;
        }
    }
    query.stopQuery();
    Ok(())
}

fn item(result: &AnyObject, query: &str) -> Option<Item> {
    let result: &NSMetadataItem = result.downcast_ref()?;
    // SAFETY: The path key is a valid attribute name constant.
    let path = result.valueForAttribute(unsafe { NSMetadataItemPathKey })?;
    let path: Retained<NSString> = path.downcast().ok()?;
    let path = PathBuf::from(path.to_string());
    let title = path.file_name()?.to_string_lossy().into_owned();
    // Spotlight matched each word somewhere in the name; highlight where the fuzzy matcher
    // finds the query, when it does.
    let matched = fuzzy::score(query, &title)
        .map(|m| m.positions)
        .unwrap_or_default();
    Some(Item::path(path, title, matched))
}

fn predicate(target: Target, query: &str) -> Option<Retained<NSPredicate>> {
    NSPredicate::predicateFromMetadataQueryString(&NSString::from_str(&query_string(target, query)?))
}

/// The Spotlight query string for `query`, ignoring case and diacritics; every word must
/// match. For names a word may appear anywhere in the file or display name; for contents it
/// must start a word of the indexed text (`w`), which the index answers quickly where a
/// substring of the whole text would scan it. None for a query with no words.
fn query_string(target: Target, query: &str) -> Option<String> {
    let clauses: Vec<String> = query
        .split_whitespace()
        .map(|word| {
            let w = escape(word);
            match target {
                Target::Names => {
                    format!("(kMDItemFSName == \"*{w}*\"cd || kMDItemDisplayName == \"*{w}*\"cd)")
                }
                Target::Contents => format!("kMDItemTextContent == \"{w}*\"cdw"),
            }
        })
        .collect();
    (!clauses.is_empty()).then(|| clauses.join(" && "))
}

/// Inside a quoted Spotlight value, `*` is a wildcard and `\` and `"` need escaping; a typed
/// `*` should match itself.
fn escape(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if matches!(c, '\\' | '"' | '*' | '\'') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_must_match_a_name() {
        assert_eq!(
            query_string(Target::Names, "tax  2026").unwrap(),
            "(kMDItemFSName == \"*tax*\"cd || kMDItemDisplayName == \"*tax*\"cd) && \
             (kMDItemFSName == \"*2026*\"cd || kMDItemDisplayName == \"*2026*\"cd)"
        );
        assert_eq!(query_string(Target::Names, "   "), None);
    }

    #[test]
    fn every_word_must_start_a_word_of_the_contents() {
        assert_eq!(
            query_string(Target::Contents, "quarterly rep").unwrap(),
            "kMDItemTextContent == \"quarterly*\"cdw && kMDItemTextContent == \"rep*\"cdw"
        );
    }

    #[test]
    fn escapes_quotes_wildcards_and_backslashes() {
        assert_eq!(escape(r#"a"b*c\d"#), r#"a\"b\*c\\d"#);
    }

    #[test]
    fn the_query_string_parses_as_a_predicate() {
        for target in [Target::Names, Target::Contents] {
            assert!(predicate(target, r#"weird "name" *.txt"#).is_some(), "{target:?}");
        }
    }
}
