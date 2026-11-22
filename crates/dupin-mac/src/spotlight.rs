//! File search through the Spotlight index (`NSMetadataQuery`), matching file and display
//! names and streaming results as Spotlight gathers them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_core::Config;
use dupin_core::file_rank::{self, FileDates};
use dupin_core::fuzzy;
use dupin_core::{Item, Provider, ProviderError, Request};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSArray, NSDate, NSDefaultRunLoopMode, NSMetadataItem, NSMetadataItemFSContentChangeDateKey,
    NSMetadataItemLastUsedDateKey, NSMetadataItemPathKey, NSMetadataQuery, NSMetadataQueryLocalComputerScope,
    NSPredicate, NSRunLoop, NSString,
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
    demotions: Arc<RwLock<Vec<String>>>,
}

impl SpotlightProvider {
    pub fn new(target: Target) -> Self {
        Self::with_demotions(target, Arc::new(RwLock::new(Config::default().file_demotions)))
    }

    pub fn with_demotions(target: Target, rules: Arc<RwLock<Vec<String>>>) -> Self {
        SpotlightProvider {
            target,
            demotions: rules,
        }
    }
}

impl Provider for SpotlightProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let demotions = if self.target == Target::Names {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            snapshot_demotions(&self.demotions, home.as_deref())
        } else {
            Vec::new()
        };
        let now = if self.target == Target::Names {
            unix_now()
        } else {
            0
        };
        autoreleasepool(|_| run_query(self.target, request, emit, &demotions, now))
    }
}

fn run_query(
    target: Target,
    request: &Request,
    emit: &mut dyn FnMut(Vec<Item>),
    demotions: &[PathBuf],
    now: u64,
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
    let mut seen_index = 0;
    let mut seen_paths = HashSet::new();
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
        let batch: Vec<Item> = match target {
            Target::Names => {
                // Spotlight may reorder results while gathering. Rescan the bounded prefix so
                // a newly surfaced path at an earlier index is not skipped.
                let mut batch = Vec::new();
                for i in 0..count {
                    if seen_paths.len() >= request.limit || request.is_cancelled() {
                        break;
                    }
                    let result = query.resultAtIndex(i);
                    let Some(path) = result_path(&result) else {
                        continue;
                    };
                    if !seen_paths.insert(path.clone()) {
                        continue;
                    }
                    if let Some(item) = item(target, &result, path, &request.query, now, demotions) {
                        batch.push(item);
                    }
                }
                batch
            }
            Target::Contents => (seen_index..count)
                .filter_map(|i| {
                    let result = query.resultAtIndex(i);
                    let path = result_path(&result)?;
                    item(target, &result, path, &request.query, now, demotions)
                })
                .collect(),
        };
        query.enableUpdates();
        if request.is_cancelled() {
            break;
        }
        seen_index = count;
        if !batch.is_empty() {
            emit(batch);
        }
        if !gathering
            || match target {
                Target::Names => seen_paths.len() >= request.limit,
                Target::Contents => seen_index >= request.limit,
            }
        {
            break;
        }
    }
    query.stopQuery();
    Ok(())
}

fn item(
    target: Target,
    result: &AnyObject,
    path: PathBuf,
    query: &str,
    now: u64,
    demotions: &[PathBuf],
) -> Option<Item> {
    let dates = if target == Target::Names {
        file_dates(result.downcast_ref()?)
    } else {
        FileDates {
            last_used: None,
            modified: None,
        }
    };
    item_for_path(target, path, query, dates, now, demotions)
}

fn item_for_path(
    target: Target,
    path: PathBuf,
    query: &str,
    dates: FileDates,
    now: u64,
    demotions: &[PathBuf],
) -> Option<Item> {
    let title = path.file_name()?.to_string_lossy().into_owned();
    let (score, matched) = match target {
        Target::Names => {
            let ranked = file_rank::score(query, &title, &path, dates, now, demotions);
            (Some(ranked.score), ranked.positions)
        }
        Target::Contents => {
            // Contents results keep Spotlight's order and the historic title-only highlight.
            let matched = fuzzy::score(query, &title)
                .map(|m| m.positions)
                .unwrap_or_default();
            (None, matched)
        }
    };
    let mut item = Item::path(path, title, matched);
    item.score = score;
    Some(item)
}

fn result_path(result: &AnyObject) -> Option<PathBuf> {
    let result: &NSMetadataItem = result.downcast_ref()?;
    // SAFETY: The path key is a valid immutable Foundation metadata key.
    let path = result.valueForAttribute(unsafe { NSMetadataItemPathKey })?;
    let path: Retained<NSString> = path.downcast().ok()?;
    Some(PathBuf::from(path.to_string()))
}

fn file_dates(result: &NSMetadataItem) -> FileDates {
    // SAFETY: These are valid immutable Foundation metadata key constants.
    let last_used = date_attribute(result, unsafe { NSMetadataItemLastUsedDateKey });
    // SAFETY: These are valid immutable Foundation metadata key constants.
    let modified = date_attribute(result, unsafe { NSMetadataItemFSContentChangeDateKey });
    FileDates { last_used, modified }
}

fn date_attribute(result: &NSMetadataItem, key: &NSString) -> Option<u64> {
    date_timestamp(result.valueForAttribute(key))
}

fn date_timestamp(value: Option<Retained<AnyObject>>) -> Option<u64> {
    let date: Retained<NSDate> = value?.downcast().ok()?;
    let seconds = date.timeIntervalSince1970();
    (seconds.is_finite() && seconds >= 0.0 && seconds < u64::MAX as f64).then_some(seconds as u64)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn expand_demotions(rules: &[String], home: Option<&Path>) -> Vec<PathBuf> {
    rules
        .iter()
        .filter(|rule| !rule.is_empty())
        .map(|rule| match (rule.as_str(), home) {
            ("~", Some(home)) => home.to_path_buf(),
            (rule, Some(home)) if rule.starts_with("~/") => home.join(&rule[2..]),
            _ => PathBuf::from(rule),
        })
        .collect()
}

fn snapshot_demotions(rules: &RwLock<Vec<String>>, home: Option<&Path>) -> Vec<PathBuf> {
    let rules = rules
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    expand_demotions(&rules, home)
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

    #[test]
    fn names_items_carry_the_file_rank_and_match_positions() {
        let path = PathBuf::from("/Users/test/Documents/Safari Guide.pdf");
        let dates = FileDates {
            last_used: Some(1_700_000_000),
            modified: Some(1_699_000_000),
        };
        let item = item_for_path(Target::Names, path, "saf", dates, 1_700_000_100, &[]).unwrap();

        assert!(item.score.is_some());
        assert_eq!(
            item.matched,
            fuzzy::score("saf", "Safari Guide.pdf").unwrap().positions
        );
    }

    #[test]
    fn contents_items_keep_spotlight_order_without_a_score() {
        let item = item_for_path(
            Target::Contents,
            PathBuf::from("/Users/test/Documents/Guide.pdf"),
            "gde",
            FileDates {
                last_used: Some(1_700_000_000),
                modified: Some(1_699_000_000),
            },
            1_700_000_100,
            &[],
        )
        .unwrap();

        assert_eq!(item.score, None);
        assert_eq!(item.matched, fuzzy::score("gde", "Guide.pdf").unwrap().positions);
    }

    #[test]
    fn date_attributes_accept_dates_and_ignore_missing_or_wrong_values() {
        let date = NSDate::dateWithTimeIntervalSince1970(1_700_000_000.75);
        let date: Retained<AnyObject> = date.into_super().into();
        assert_eq!(date_timestamp(Some(date)), Some(1_700_000_000));
        assert_eq!(date_timestamp(None), None);

        let not_a_date: Retained<AnyObject> = NSString::from_str("not a date").into_super().into();
        assert_eq!(date_timestamp(Some(not_a_date)), None);

        let before_epoch = NSDate::dateWithTimeIntervalSince1970(-1.0);
        let before_epoch: Retained<AnyObject> = before_epoch.into_super().into();
        assert_eq!(date_timestamp(Some(before_epoch)), None);
    }

    #[test]
    fn demotion_rules_expand_home_paths_in_the_native_layer() {
        let rules = vec![
            "~".into(),
            "~/Library/Caches".into(),
            "Vendor/Cache".into(),
            "".into(),
        ];
        assert_eq!(
            expand_demotions(&rules, Some(Path::new("/Users/test"))),
            vec![
                PathBuf::from("/Users/test"),
                PathBuf::from("/Users/test/Library/Caches"),
                PathBuf::from("Vendor/Cache")
            ]
        );
    }

    #[test]
    fn each_job_snapshots_the_current_demotion_rules() {
        let rules = RwLock::new(vec!["~/Library/Caches".into()]);
        let first = snapshot_demotions(&rules, Some(Path::new("/Users/test")));
        *rules.write().unwrap() = vec!["Vendor/Cache".into()];
        let second = snapshot_demotions(&rules, Some(Path::new("/Users/test")));

        assert_eq!(first, [PathBuf::from("/Users/test/Library/Caches")]);
        assert_eq!(second, [PathBuf::from("Vendor/Cache")]);
    }
}
