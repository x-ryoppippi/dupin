//! File search through the Spotlight index (`NSMetadataQuery`), matching file and display
//! names and streaming results as Spotlight gathers them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_core::Config;
use dupin_core::file_query::{FileKind, FileQuery};
use dupin_core::file_rank::{self, FileDates};
use dupin_core::fuzzy;
use dupin_core::{Item, Provider, ProviderError, Request};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSArray, NSCompoundPredicate, NSDate, NSDefaultRunLoopMode, NSMetadataItem,
    NSMetadataItemFSContentChangeDateKey, NSMetadataItemLastUsedDateKey, NSMetadataItemPathKey,
    NSMetadataQuery, NSMetadataQueryLocalComputerScope, NSPredicate, NSRunLoop, NSString, NSURL,
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
        let file_query = match FileQuery::parse(&request.query) {
            Ok(query) => query,
            Err(_) => return Ok(()),
        };
        if !eligible_query(self.target, &file_query) {
            return Ok(());
        }

        let home = std::env::var_os("HOME").map(PathBuf::from);
        let folder = match resolve_folder(file_query.folder.as_deref(), home.as_deref()) {
            Ok(folder) => folder,
            Err(()) => return Ok(()),
        };
        let folder = match folder.as_deref().map(canonical_folder).transpose() {
            Ok(folder) => folder,
            Err(()) => return Ok(()),
        };
        let demotions = if self.target == Target::Names {
            snapshot_demotions(&self.demotions, home.as_deref())
        } else {
            Vec::new()
        };
        let now = unix_now();
        let query_terms = free_terms(&file_query);
        let job = SearchJob {
            target: self.target,
            file_query: &file_query,
            folder: folder.as_deref(),
            query_terms: &query_terms,
            demotions: &demotions,
            now,
        };
        autoreleasepool(|_| run_query(request, emit, &job))
    }
}

struct SearchJob<'a> {
    target: Target,
    file_query: &'a FileQuery,
    folder: Option<&'a Path>,
    query_terms: &'a str,
    demotions: &'a [PathBuf],
    now: u64,
}

fn run_query(
    request: &Request,
    emit: &mut dyn FnMut(Vec<Item>),
    job: &SearchJob<'_>,
) -> Result<(), ProviderError> {
    let Some(predicate) = predicate(job.target, job.file_query, job.now) else {
        return Ok(());
    };
    let query = NSMetadataQuery::new();
    query.setPredicate(Some(&predicate));
    let Ok(scopes) = search_scopes(job.folder) else {
        return Ok(());
    };
    // SAFETY: The array contains the documented local-computer scope string or a file NSURL.
    unsafe { query.setSearchScopes(&scopes) };
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
        let batch: Vec<Item> = match job.target {
            Target::Names => {
                // Spotlight may reorder results while gathering. Rescan the bounded prefix so
                // a newly surfaced path at an earlier index is not skipped.
                let mut batch = Vec::new();
                for i in 0..count {
                    if seen_paths.len() >= request.limit || request.is_cancelled() {
                        break;
                    }
                    let result = query.resultAtIndex(i);
                    let Some(path) = result_path(&result).filter(|path| within_scope(path, job.folder))
                    else {
                        continue;
                    };
                    if !seen_paths.insert(path.clone()) {
                        continue;
                    }
                    if let Some(item) =
                        item(job.target, &result, path, job.query_terms, job.now, job.demotions)
                    {
                        batch.push(item);
                    }
                }
                batch
            }
            Target::Contents => {
                let mut batch = Vec::new();
                for i in seen_index..count {
                    if request.is_cancelled() {
                        break;
                    }
                    let result = query.resultAtIndex(i);
                    let Some(path) = result_path(&result).filter(|path| within_scope(path, job.folder))
                    else {
                        continue;
                    };
                    if let Some(item) =
                        item(job.target, &result, path, job.query_terms, job.now, job.demotions)
                    {
                        batch.push(item);
                    }
                }
                batch
            }
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
            || match job.target {
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
            (rule, Some(home)) if rule.starts_with("~/") => under_home(home, &rule[2..]),
            _ => PathBuf::from(rule),
        })
        .collect()
}

fn under_home(home: &Path, suffix: &str) -> PathBuf {
    home.join(suffix.trim_start_matches('/'))
}

fn snapshot_demotions(rules: &RwLock<Vec<String>>, home: Option<&Path>) -> Vec<PathBuf> {
    let rules = rules
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    expand_demotions(&rules, home)
}

fn eligible_query(target: Target, query: &FileQuery) -> bool {
    if target == Target::Contents && query.terms.is_empty() {
        return false;
    }
    !query.terms.is_empty()
        || query.kind.is_some()
        || query.extension.is_some()
        || query.folder.is_some()
        || query.modified_within.is_some()
}

fn predicate(target: Target, query: &FileQuery, now: u64) -> Option<Retained<NSPredicate>> {
    if !eligible_query(target, query) {
        return None;
    }
    let metadata =
        NSPredicate::predicateFromMetadataQueryString(&NSString::from_str(&query_string(target, query)?))?;
    let Some(modified_within) = query.modified_within else {
        return Some(metadata);
    };

    let modified = modified_predicate(modified_within, now);
    let subpredicates: Retained<NSArray<NSPredicate>> = NSArray::from_slice(&[&*metadata, &*modified]);
    Some(NSCompoundPredicate::andPredicateWithSubpredicates(&subpredicates).into_super())
}

fn query_string(target: Target, query: &FileQuery) -> Option<String> {
    if !eligible_query(target, query) {
        return None;
    }
    let mut clauses: Vec<String> = query
        .terms
        .iter()
        .map(|word| {
            let word = escape(word);
            match target {
                Target::Names => {
                    format!("(kMDItemFSName == \"*{word}*\"cd || kMDItemDisplayName == \"*{word}*\"cd)")
                }
                Target::Contents => format!("kMDItemTextContent == \"{word}*\"cdw"),
            }
        })
        .collect();

    if let Some(kind) = query.kind {
        clauses.push(format!("kMDItemContentTypeTree == \"{}\"", kind_uti(kind)));
    }
    if let Some(extension) = &query.extension {
        clauses.push(format!("kMDItemFSName == \"*.{}\"cd", escape(extension)));
    }

    if clauses.is_empty() {
        clauses.push("kMDItemFSName == \"*\"cd".into());
    }
    Some(clauses.join(" && "))
}

fn kind_uti(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Pdf => "com.adobe.pdf",
        FileKind::Image => "public.image",
        FileKind::Text => "public.text",
        FileKind::Audio => "public.audio",
        FileKind::Video => "public.movie",
        FileKind::Folder => "public.folder",
        FileKind::Archive => "public.archive",
        FileKind::Application => "com.apple.application-bundle",
    }
}

fn modified_predicate(within: Duration, now: u64) -> Retained<NSPredicate> {
    let cutoff = now.saturating_sub(within.as_secs());
    let cutoff = NSDate::dateWithTimeIntervalSince1970(cutoff as f64);
    let arguments: Retained<NSArray> = NSArray::from_slice(&[&*cutoff as &AnyObject]);
    let format = NSString::from_str("kMDItemFSContentChangeDate >= %@");
    // SAFETY: The format is a fixed predicate with one object placeholder; arguments contains
    // exactly one NSDate, so no user input is interpreted as predicate syntax.
    unsafe { NSPredicate::predicateWithFormat_argumentArray(&format, Some(&arguments)) }
}

fn resolve_folder(folder: Option<&str>, home: Option<&Path>) -> Result<Option<PathBuf>, ()> {
    let Some(folder) = folder else {
        return Ok(None);
    };
    let path = match folder {
        "~" => home.ok_or(())?.to_path_buf(),
        folder if folder.starts_with("~/") => under_home(home.ok_or(())?, &folder[2..]),
        folder => PathBuf::from(folder),
    };
    path.is_absolute().then_some(Some(path)).ok_or(())
}

fn search_scopes(folder: Option<&Path>) -> Result<Retained<NSArray>, ()> {
    let scopes = if let Some(folder) = folder {
        let url = folder_url(folder)?;
        NSArray::from_slice(&[&*url as &AnyObject])
    } else {
        // SAFETY: The scope constant is an immutable NSString exported for this property.
        unsafe { NSArray::from_slice(&[NSMetadataQueryLocalComputerScope as &AnyObject]) }
    };
    Ok(scopes)
}

fn canonical_folder(folder: &Path) -> Result<PathBuf, ()> {
    let folder = folder.canonicalize().map_err(|_| ())?;
    folder.is_dir().then_some(folder).ok_or(())
}

fn within_scope(path: &Path, folder: Option<&Path>) -> bool {
    // Spotlight can ignore an unavailable scope instead of returning no results.
    folder.is_none_or(|folder| path.starts_with(folder))
}

fn folder_url(folder: &Path) -> Result<Retained<NSURL>, ()> {
    let path = folder.to_str().ok_or(())?;
    Ok(NSURL::fileURLWithPath_isDirectory(
        &NSString::from_str(path),
        true,
    ))
}

fn free_terms(query: &FileQuery) -> String {
    query.terms.join(" ")
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
    use objc2_foundation::NSDictionary;

    #[test]
    fn every_word_must_match_a_name() {
        let query = FileQuery::parse("tax  2026").unwrap();
        assert_eq!(
            query_string(Target::Names, &query).unwrap(),
            "(kMDItemFSName == \"*tax*\"cd || kMDItemDisplayName == \"*tax*\"cd) && \
             (kMDItemFSName == \"*2026*\"cd || kMDItemDisplayName == \"*2026*\"cd)"
        );
    }

    #[test]
    fn every_word_must_start_a_word_of_the_contents() {
        assert_eq!(
            query_string(Target::Contents, &FileQuery::parse("quarterly rep").unwrap()).unwrap(),
            "kMDItemTextContent == \"quarterly*\"cdw && kMDItemTextContent == \"rep*\"cdw"
        );
    }

    #[test]
    fn escapes_quotes_wildcards_and_backslashes() {
        assert_eq!(escape(r#"a"b*c\d"#), r#"a\"b\*c\\d"#);
    }

    #[test]
    fn metadata_predicates_escape_user_terms_and_extension_suffixes() {
        let query = FileQuery::parse(r#"a"b*c\d ext:.rs"#).unwrap();
        assert_eq!(
            query_string(Target::Names, &query).unwrap(),
            r#"(kMDItemFSName == "*a\"b\*c\\d*"cd || kMDItemDisplayName == "*a\"b\*c\\d*"cd) && kMDItemFSName == "*.rs"cd"#
        );

        let percent = FileQuery::parse("report%@ ext:rs").unwrap();
        assert!(predicate(Target::Names, &percent, 1_700_000_000).is_some());
    }

    #[test]
    fn the_query_string_parses_as_a_predicate() {
        for target in [Target::Names, Target::Contents] {
            let query = FileQuery::parse(r#"weird "name" *.txt"#).unwrap();
            assert!(predicate(target, &query, 1_700_000_000).is_some(), "{target:?}");
        }
    }

    #[test]
    fn query_predicates_use_terms_filters_and_static_content_types() {
        let query = FileQuery::parse("draft ext:.rs kind:text").unwrap();
        assert_eq!(
            query_string(Target::Names, &query).unwrap(),
            "(kMDItemFSName == \"*draft*\"cd || kMDItemDisplayName == \"*draft*\"cd) && \
             kMDItemContentTypeTree == \"public.text\" && kMDItemFSName == \"*.rs\"cd"
        );

        for (kind, uti) in [
            (FileKind::Pdf, "com.adobe.pdf"),
            (FileKind::Image, "public.image"),
            (FileKind::Text, "public.text"),
            (FileKind::Audio, "public.audio"),
            (FileKind::Video, "public.movie"),
            (FileKind::Folder, "public.folder"),
            (FileKind::Archive, "public.archive"),
            (FileKind::Application, "com.apple.application-bundle"),
        ] {
            assert_eq!(kind_uti(kind), uti);
        }
    }

    #[test]
    fn only_filter_names_queries_match_but_empty_invalid_and_contents_filter_only_do_not() {
        let filter_only = FileQuery::parse("in:~/Work modified:7d").unwrap();
        assert!(eligible_query(Target::Names, &filter_only));
        assert_eq!(
            query_string(Target::Names, &filter_only).as_deref(),
            Some("kMDItemFSName == \"*\"cd")
        );
        assert!(predicate(Target::Names, &filter_only, 1_700_000_000).is_some());

        assert!(!eligible_query(Target::Contents, &filter_only));
        assert!(predicate(Target::Contents, &filter_only, 1_700_000_000).is_none());

        let empty = FileQuery::parse("   ").unwrap();
        assert!(!eligible_query(Target::Names, &empty));
        assert!(predicate(Target::Names, &empty, 1_700_000_000).is_none());
        assert!(FileQuery::parse("ext:bad*").is_err());
    }

    #[test]
    fn contents_query_uses_only_free_terms_and_preserves_no_score_semantics() {
        let query = FileQuery::parse("quarterly rep kind:pdf").unwrap();
        assert_eq!(free_terms(&query), "quarterly rep");
        assert_eq!(
            query_string(Target::Contents, &query).unwrap(),
            "kMDItemTextContent == \"quarterly*\"cdw && \
             kMDItemTextContent == \"rep*\"cdw && kMDItemContentTypeTree == \"com.adobe.pdf\""
        );
    }

    #[test]
    fn folder_scopes_expand_home_without_filesystem_resolution() {
        let home = Path::new("/Users/test");
        assert_eq!(
            resolve_folder(Some("~/Work/../Documents"), Some(home)).unwrap(),
            Some(PathBuf::from("/Users/test/Work/../Documents"))
        );
        assert!(resolve_folder(Some("~/Documents"), None).is_err());
        assert_eq!(resolve_folder(None, None).unwrap(), None);
        assert_eq!(
            resolve_folder(Some("~//Documents"), Some(home)).unwrap(),
            Some(home.join("Documents"))
        );
        assert_eq!(
            expand_demotions(&["~//Documents".into()], Some(home)),
            [home.join("Documents")]
        );
        assert_eq!(
            resolve_folder(Some("~///"), Some(home)).unwrap(),
            Some(home.to_owned())
        );

        let scopes = search_scopes(Some(Path::new("/Users/test/Work Docs"))).unwrap();
        let url: Retained<NSURL> = scopes.objectAtIndex(0).downcast().unwrap();
        assert_eq!(url.path().unwrap().to_string(), "/Users/test/Work Docs");
        assert!(url.hasDirectoryPath());
    }

    #[test]
    fn scope_validation_resolves_links_and_rejects_missing_folders_files_and_outside_rows() {
        let root = std::env::temp_dir().join(format!(
            "dupin-scope-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let folder = root.join("Work Docs");
        std::fs::create_dir(&folder).unwrap();
        let link = root.join("linked");
        std::os::unix::fs::symlink(&folder, &link).unwrap();
        let file = root.join("fixture.txt");
        std::fs::write(&file, "fixture").unwrap();
        let canonical = canonical_folder(&folder).unwrap();
        assert_eq!(canonical_folder(&link).unwrap(), canonical);
        assert!(canonical_folder(&root.join("missing")).is_err());
        assert!(canonical_folder(&file).is_err());
        assert!(within_scope(&canonical.join("inside.pdf"), Some(&canonical)));
        assert!(!within_scope(
            &canonical.with_file_name("Work Docs-other").join("outside.pdf"),
            Some(&canonical)
        ));
        assert!(!within_scope(
            Path::new("/System/Applications/Calculator.app"),
            Some(&canonical)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn modified_filter_uses_an_inclusive_cutoff_and_saturates_at_epoch() {
        let now = 1_700_000_000;
        let predicate = modified_predicate(Duration::from_secs(3600), now);
        assert!(evaluates_date(&predicate, now - 3600));
        assert!(!evaluates_date(&predicate, now - 3601));

        let at_epoch = modified_predicate(Duration::from_secs(u64::MAX), now);
        assert!(evaluates_date(&at_epoch, 0));
        assert!(evaluates_date(&at_epoch, 1));
    }

    #[test]
    fn combined_metadata_and_date_predicate_requires_both_conditions() {
        let query = FileQuery::parse("report modified:1h").unwrap();
        let predicate = predicate(Target::Names, &query, 1_700_000_000).unwrap();

        assert!(evaluates_file(&predicate, "report.md", 1_700_000_000 - 3600));
        assert!(!evaluates_file(&predicate, "report.md", 1_700_000_000 - 3601));
        assert!(!evaluates_file(&predicate, "summary.md", 1_700_000_000));
    }

    fn evaluates_file(predicate: &NSPredicate, name: &str, modified: u64) -> bool {
        let name_key = NSString::from_str("kMDItemFSName");
        let date_key = NSString::from_str("kMDItemFSContentChangeDate");
        let name = NSString::from_str(name);
        let date = NSDate::dateWithTimeIntervalSince1970(modified as f64);
        let object: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
            &[&*name_key, &*date_key],
            &[&*name as &AnyObject, &*date as &AnyObject],
        );
        let object: Retained<AnyObject> = object.into_super().into();
        // SAFETY: NSDictionary implements KVC lookup for the metadata keys used by the predicate.
        unsafe { predicate.evaluateWithObject(Some(&object)) }
    }

    fn evaluates_date(predicate: &NSPredicate, timestamp: u64) -> bool {
        let key = NSString::from_str("kMDItemFSContentChangeDate");
        let date = NSDate::dateWithTimeIntervalSince1970(timestamp as f64);
        let object: Retained<NSDictionary<NSString, NSDate>> = NSDictionary::from_slices(&[&*key], &[&*date]);
        let object: Retained<AnyObject> = object.into_super().into();
        // SAFETY: NSDictionary implements KVC lookup for the date key used in the predicate.
        unsafe { predicate.evaluateWithObject(Some(&object)) }
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
    fn names_ranking_and_highlighting_ignore_qualifier_tokens() {
        let query = FileQuery::parse("saf kind:pdf ext:pdf modified:7d").unwrap();
        let terms = free_terms(&query);
        let path = PathBuf::from("/Users/test/Documents/Safari.pdf");
        let dates = FileDates {
            last_used: Some(1_700_000_000),
            modified: Some(1_699_000_000),
        };
        let ranked = item_for_path(Target::Names, path.clone(), &terms, dates, 1_700_000_100, &[]).unwrap();
        let expected = file_rank::score("saf", "Safari.pdf", &path, dates, 1_700_000_100, &[]);

        assert_eq!(ranked.score, Some(expected.score));
        assert_eq!(ranked.matched, expected.positions);
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
